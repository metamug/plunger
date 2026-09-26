use crate::model::{FieldKind, FormField, ParsedRequest};
use serde::Deserialize;
use std::path::Path;

/// Parses a pasted curl command (as copied from a browser's "Copy as cURL",
/// or typed by hand) into method/url/headers/body.
///
/// Understands `-X`, `-H`, `-d` and its variants (repeated `-d` are joined
/// with `&`, `--data-urlencode` is encoded), `--json`, `-F`, `-u` (becomes a
/// Basic `Authorization` header), `-b`, `-A`, `-e`, `-G`, `-I`, `--url`, and
/// the `--flag=value` forms. Flags that take a value but don't matter here
/// (`-o`, `-m`, `--proxy`, ...) are skipped together with their value so it
/// isn't mistaken for the URL; other flags (`--compressed`, `-k`, ...) are
/// ignored.
pub fn parse_curl(input: &str) -> Result<ParsedRequest, String> {
    let tokens = shell_words::split(input.trim()).map_err(|e| format!("Could not parse that as a shell command: {e}"))?;
    if tokens.is_empty() {
        return Err("Nothing to parse.".to_string());
    }

    let mut iter = tokens.into_iter().peekable();
    if let Some(first) = iter.peek() {
        if first.eq_ignore_ascii_case("curl") || first.eq_ignore_ascii_case("curl.exe") {
            iter.next();
        }
    }

    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut json: Option<String> = None;
    let mut form_fields: Vec<FormField> = Vec::new();
    let (mut get_mode, mut head_mode) = (false, false);

    while let Some(tok) = iter.next() {
        let (flag, inline) = match tok.starts_with("--").then(|| tok.split_once('=')).flatten() {
            Some((f, v)) => (f.to_string(), Some(v.to_string())),
            None => (tok.clone(), None),
        };
        let mut value = || inline.clone().or_else(|| iter.next());
        match flag.as_str() {
            "-X" | "--request" => method = value(),
            "-H" | "--header" => {
                if let Some((k, v)) = value().as_deref().and_then(|h| h.split_once(':')) {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" => data.extend(value()),
            "--data-urlencode" => data.extend(value().map(|v| encode_data(&v))),
            "--json" => json = value(),
            "-F" | "--form" | "--form-string" => {
                if let Some(field) = value().and_then(|f| parse_form_field(&f)) {
                    form_fields.push(field);
                }
            }
            "-u" | "--user" => {
                if let Some(creds) = value() {
                    let creds = if creds.contains(':') { creds } else { format!("{creds}:") };
                    headers.push(("Authorization".into(), format!("Basic {}", base64(creds.as_bytes()))));
                }
            }
            "-b" | "--cookie" => {
                // A value without `=` names a cookie file, which isn't supported.
                if let Some(cookie) = value().filter(|c| c.contains('=')) {
                    headers.push(("Cookie".into(), cookie));
                }
            }
            "-A" | "--user-agent" => headers.extend(value().map(|v| ("User-Agent".to_string(), v))),
            "-e" | "--referer" => headers.extend(value().map(|v| ("Referer".to_string(), v))),
            "--url" => url = url.or_else(value),
            "-G" | "--get" => get_mode = true,
            "-I" | "--head" => head_mode = true,
            "-o" | "--output" | "-m" | "--max-time" | "--connect-timeout" | "-x" | "--proxy" | "--cacert"
            | "--cert" | "--key" | "-w" | "--write-out" | "--retry" | "--resolve" | "--max-redirs" | "-c"
            | "--cookie-jar" | "-T" | "--upload-file" | "--interface" | "-U" | "--proxy-user" | "-K" | "--config" => {
                value();
            }
            _ if tok.starts_with('-') => {
                // Any other flag is ignored without taking the next token:
                // guessing wrong would eat the URL.
            }
            _ => {
                if url.is_none() {
                    url = Some(tok);
                }
            }
        }
    }

    let mut url = url.ok_or_else(|| "Could not find a URL in that curl command.".to_string())?;
    let mut body = if data.is_empty() { None } else { Some(data.join("&")) };
    if let Some(text) = json {
        for (name, val) in [("Content-Type", "application/json"), ("Accept", "application/json")] {
            if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name)) {
                headers.push((name.to_string(), val.to_string()));
            }
        }
        body = Some(text);
    }
    if get_mode {
        if let Some(query) = body.take() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&query);
        }
    }
    let has_payload = body.is_some() || !form_fields.is_empty();
    let method = method.unwrap_or_else(|| {
        if head_mode {
            "HEAD".to_string()
        } else if has_payload {
            "POST".to_string()
        } else {
            "GET".to_string()
        }
    });

    Ok(ParsedRequest {
        method: method.to_uppercase(),
        url,
        headers,
        body,
        form_fields,
    })
}

/// `--data-urlencode`: `name=content` encodes the content, a bare `content`
/// is encoded whole.
fn encode_data(spec: &str) -> String {
    match spec.split_once('=') {
        Some((name, content)) => format!("{name}={}", crate::query::encode_value(content)),
        None => crate::query::encode_value(spec),
    }
}

fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `-F name=value` is a text part; `-F name=@path` (optionally followed by
/// `;type=...`) is a file part.
fn parse_form_field(spec: &str) -> Option<FormField> {
    let (key, value) = spec.split_once('=')?;
    let (kind, value) = match value.strip_prefix('@') {
        Some(path) => (FieldKind::File, path.split(";type=").next().unwrap_or(path)),
        None => (FieldKind::Text, value),
    };
    Some(FormField {
        key: key.trim().to_string(),
        kind,
        value: value.to_string(),
        enabled: true,
    })
}

#[derive(Deserialize)]
struct Har {
    log: HarLog,
}
#[derive(Deserialize)]
struct HarLog {
    entries: Vec<HarEntryWrapper>,
}
#[derive(Deserialize)]
struct HarEntryWrapper {
    request: HarRequest,
}
#[derive(Deserialize)]
struct HarRequest {
    method: String,
    url: String,
    #[serde(default)]
    headers: Vec<HarHeader>,
    #[serde(rename = "postData", default)]
    post_data: Option<HarPostData>,
}
#[derive(Deserialize)]
struct HarHeader {
    name: String,
    value: String,
}
#[derive(Deserialize)]
struct HarPostData {
    #[serde(default)]
    text: Option<String>,
}

/// Parses every request entry out of a HAR file. A HAR captured from a
/// browser's Network tab usually has many entries (every request the page
/// made), so this returns all of them for the caller to present as a pick
/// list rather than guessing which one the user wants.
pub fn parse_har(path: &Path) -> Result<Vec<ParsedRequest>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("Could not read file: {e}"))?;
    parse_har_str(&content)
}

fn parse_har_str(content: &str) -> Result<Vec<ParsedRequest>, String> {
    let har: Har = serde_json::from_str(content).map_err(|e| format!("Not a valid HAR file: {e}"))?;

    Ok(har
        .log
        .entries
        .into_iter()
        .map(|entry| {
            let req = entry.request;
            let headers = req
                .headers
                .into_iter()
                // HTTP/2 pseudo-headers (:authority, :method, :path, :scheme)
                // show up in Chrome's HAR exports but aren't valid to send
                // as regular request headers.
                .filter(|h| !h.name.starts_with(':'))
                .map(|h| (h.name, h.value))
                .collect();
            let body = req.post_data.and_then(|p| p.text);
            ParsedRequest {
                method: req.method.to_uppercase(),
                url: req.url,
                headers,
                body,
                form_fields: Vec::new(),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_browser_copy_as_curl() {
        let r = parse_curl(
            r#"curl 'https://api.example.com/items?x=1' -H 'Accept: application/json' -H "Authorization: Bearer abc" --compressed -X put --data-raw '{"a":"b c"}'"#,
        )
        .unwrap();
        assert_eq!(r.method, "PUT");
        assert_eq!(r.url, "https://api.example.com/items?x=1");
        assert_eq!(r.headers.len(), 2);
        assert_eq!(r.headers[1], ("Authorization".to_string(), "Bearer abc".to_string()));
        assert_eq!(r.body.as_deref(), Some(r#"{"a":"b c"}"#));
    }

    #[test]
    fn data_implies_post_and_no_data_implies_get() {
        assert_eq!(parse_curl("curl https://a.com -d x=1").unwrap().method, "POST");
        assert_eq!(parse_curl("curl https://a.com").unwrap().method, "GET");
    }

    #[test]
    fn supports_equals_forms_and_optional_curl_prefix() {
        let r = parse_curl("https://a.com --request=DELETE --header=X-A:1 --data=raw").unwrap();
        assert_eq!(r.method, "DELETE");
        assert_eq!(r.headers, vec![("X-A".to_string(), "1".to_string())]);
        assert_eq!(r.body.as_deref(), Some("raw"));
    }

    #[test]
    fn form_flags_become_multipart_fields_and_imply_post() {
        let r = parse_curl(
            "curl https://a.com/upload -F 'title=My Doc' -F doc=@/tmp/a.pdf;type=application/pdf --form note=@\"C:/x y/n.txt\"",
        )
        .unwrap();
        assert_eq!(r.method, "POST");
        assert!(r.body.is_none());
        assert_eq!(r.form_fields.len(), 3);
        assert_eq!(r.form_fields[0].key, "title");
        assert_eq!(r.form_fields[0].kind, FieldKind::Text);
        assert_eq!(r.form_fields[0].value, "My Doc");
        assert_eq!(r.form_fields[1].kind, FieldKind::File);
        assert_eq!(r.form_fields[1].value, "/tmp/a.pdf");
        assert_eq!(r.form_fields[2].value, "C:/x y/n.txt");
    }

    #[test]
    fn unknown_flags_do_not_swallow_the_url() {
        let r = parse_curl("curl -k -s https://a.com").unwrap();
        assert_eq!(r.url, "https://a.com");
    }

    #[test]
    fn errors_are_reported() {
        assert!(parse_curl("   ").is_err());
        assert!(parse_curl("curl -H 'A: b'").is_err());
        assert!(parse_curl("curl 'unterminated").is_err());
    }

    #[test]
    fn har_parses_entries_and_drops_pseudo_headers() {
        let har = r#"{"log":{"entries":[
            {"request":{"method":"post","url":"https://a.com/x",
              "headers":[{"name":":authority","value":"a.com"},{"name":"Accept","value":"*/*"}],
              "postData":{"text":"hello"}}},
            {"request":{"method":"GET","url":"https://a.com/y"}}]}}"#;
        let v = parse_har_str(har).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].method, "POST");
        assert_eq!(v[0].headers, vec![("Accept".to_string(), "*/*".to_string())]);
        assert_eq!(v[0].body.as_deref(), Some("hello"));
        assert!(v[1].body.is_none());
        assert!(parse_har_str("{}").is_err());
    }

    #[test]
    fn user_becomes_a_basic_authorization_header_not_the_url() {
        let r = parse_curl("curl -u alice:s3cret http://h/x").unwrap();
        assert_eq!(r.url, "http://h/x");
        assert_eq!(r.headers, vec![("Authorization".to_string(), "Basic YWxpY2U6czNjcmV0".to_string())]);
    }

    #[test]
    fn base64_pads_correctly() {
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b""), "");
    }

    #[test]
    fn flags_that_take_a_value_do_not_leak_it_into_the_url() {
        let r = parse_curl("curl -o out.txt -m 5 --proxy http://p:1 http://h/x -b 'sid=abc' -A agent -e http://ref").unwrap();
        assert_eq!(r.url, "http://h/x");
        assert!(r.headers.contains(&("Cookie".to_string(), "sid=abc".to_string())));
        assert!(r.headers.contains(&("User-Agent".to_string(), "agent".to_string())));
        assert!(r.headers.contains(&("Referer".to_string(), "http://ref".to_string())));
    }

    #[test]
    fn repeated_data_flags_are_joined_and_urlencode_is_encoded() {
        let r = parse_curl("curl -d a=1 -d b=2 --data-urlencode 'q=x y&z' http://h/x").unwrap();
        assert_eq!(r.body.as_deref(), Some("a=1&b=2&q=x%20y%26z"));
        assert_eq!(r.method, "POST");
    }

    #[test]
    fn get_flag_moves_data_into_the_query_and_head_sets_the_method() {
        let r = parse_curl("curl -G -d x=1 -d y=2 http://h/x").unwrap();
        assert_eq!((r.method.as_str(), r.url.as_str(), r.body), ("GET", "http://h/x?x=1&y=2", None));
        assert_eq!(parse_curl("curl -I http://h/x").unwrap().method, "HEAD");
    }

    #[test]
    fn json_flag_sets_body_and_headers() {
        let r = parse_curl(r#"curl --json '{"a":1}' http://h/x"#).unwrap();
        assert_eq!((r.method.as_str(), r.body.as_deref()), ("POST", Some(r#"{"a":1}"#)));
        assert!(r.headers.contains(&("Content-Type".to_string(), "application/json".to_string())));
    }
}
