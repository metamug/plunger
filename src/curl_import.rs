use crate::model::{FieldKind, FormField, ParsedRequest};
use serde::Deserialize;
use std::path::Path;

/// Parses a pasted curl command (as copied from a browser's "Copy as cURL",
/// or typed by hand) into method/url/headers/body.
///
/// Recognizes `-X`/`--request`, `-H`/`--header`, `-d`/`--data`/`--data-raw`/
/// `--data-binary`, and their `--flag=value` forms, plus the bare URL
/// argument. Everything else (`-u`, `--compressed`, `-k`, cookies, etc.) is
/// silently ignored rather than erroring — most real-world pastes only use
/// the flags above.
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
    let mut body: Option<String> = None;
    let mut form_fields: Vec<FormField> = Vec::new();

    while let Some(tok) = iter.next() {
        match tok.as_str() {
            "-X" | "--request" => method = iter.next(),
            "-H" | "--header" => {
                if let Some(h) = iter.next() {
                    if let Some((k, v)) = h.split_once(':') {
                        headers.push((k.trim().to_string(), v.trim().to_string()));
                    }
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" => {
                body = iter.next();
            }
            "-F" | "--form" | "--form-string" => {
                if let Some(field) = iter.next().and_then(|f| parse_form_field(&f)) {
                    form_fields.push(field);
                }
            }
            _ if tok.starts_with("--header=") => {
                let h = &tok["--header=".len()..];
                if let Some((k, v)) = h.split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            _ if tok.starts_with("--data=")
                || tok.starts_with("--data-raw=")
                || tok.starts_with("--data-binary=") =>
            {
                if let Some(idx) = tok.find('=') {
                    body = Some(tok[idx + 1..].to_string());
                }
            }
            _ if tok.starts_with("--request=") => {
                method = Some(tok["--request=".len()..].to_string());
            }
            _ if tok.starts_with('-') => {
                // Unknown/unsupported flag. Deliberately not consuming the
                // next token for these — guessing wrong would eat the URL.
            }
            _ => {
                if url.is_none() {
                    url = Some(tok);
                }
            }
        }
    }

    let url = url.ok_or_else(|| "Could not find a URL in that curl command.".to_string())?;
    let has_payload = body.is_some() || !form_fields.is_empty();
    let method = method.unwrap_or_else(|| if has_payload { "POST".to_string() } else { "GET".to_string() });

    Ok(ParsedRequest {
        method: method.to_uppercase(),
        url,
        headers,
        body,
        form_fields,
    })
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
}
