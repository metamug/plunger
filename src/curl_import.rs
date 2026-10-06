use crate::model::{FieldKind, FormField, ParsedRequest};
use serde::Deserialize;
use std::path::Path;

/// Reads the file (or standard input, for `-`) that `-d @path` names.
pub type FileReader<'a> = dyn FnMut(&str) -> Result<String, String> + 'a;

/// Options of a curl command line that are about the transfer rather than the request: what to
/// print, where to write it, when to fail. Only `plunger curl` acts on them.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CurlOptions {
    /// `-i`: print the status line and headers before the body.
    pub include_headers: bool,
    /// `-I`: a HEAD request, print only the headers.
    pub head_only: bool,
    /// `-s`: no error text unless `-S` is also given.
    pub silent: bool,
    pub show_error: bool,
    /// `-f`: exit 22 on a status of 400 or more, printing no body.
    pub fail: bool,
    pub fail_with_body: bool,
    /// `-L`: follow redirects (curl does not by default).
    pub follow: bool,
    /// `-k`: skip TLS certificate checks.
    pub insecure: bool,
    /// `-m`: seconds, may be fractional.
    pub max_time: Option<f64>,
    /// `-o`: write the body to a file (`-` is stdout).
    pub output: Option<String>,
    /// `-w`: a template printed after the transfer.
    pub write_out: Option<String>,
    /// `--retry N`: how many times to try again after a transient failure.
    pub retry: u32,
    /// `--retry-delay`: seconds between tries (curl doubles the wait each time when this is not given).
    pub retry_delay: Option<f64>,
    /// `--retry-max-time`: stop retrying after this many seconds in total.
    pub retry_max_time: Option<f64>,
    /// `--retry-all-errors`: retry on any HTTP error, not only the transient ones.
    pub retry_all_errors: bool,
    /// Flags that change how curl would connect and that Plunger cannot honour.
    pub unsupported: Vec<String>,
}

/// Short flags that take a value (`-H x`, `-Hx`, and last in a cluster such as `-sSLo file`).
const SHORT_WITH_VALUE: &str = "XHdFubAeomwxcTUK";
const LONG_WITH_VALUE: &[&str] = &[
    "--request", "--header", "--data", "--data-raw", "--data-binary", "--data-ascii", "--data-urlencode", "--json",
    "--form", "--form-string", "--user", "--cookie", "--user-agent", "--referer", "--url", "--output", "--max-time",
    "--connect-timeout", "--proxy", "--cacert", "--cert", "--key", "--write-out", "--retry", "--retry-delay",
    "--retry-max-time", "--resolve", "--max-redirs", "--cookie-jar", "--upload-file", "--interface", "--proxy-user",
    "--config",
];

/// True for `-sSL`, `-XPOST` or `-ofile.txt`: letters up to the first value-taking flag, whose
/// value is whatever follows it. A token like `-1` or `-x9` is not a cluster.
fn is_cluster(tok: &str) -> bool {
    if tok.len() <= 2 || !tok.starts_with('-') {
        return false;
    }
    for c in tok[1..].chars() {
        if SHORT_WITH_VALUE.contains(c) {
            return true;
        }
        if !c.is_ascii_alphabetic() {
            return false;
        }
    }
    true
}

/// Splits clustered short flags (`-sSL` -> `-s -S -L`, `-XPOST` -> `-X POST`, `-ofile` -> `-o file`),
/// leaving the values of value-taking flags untouched.
fn expand_clusters(tokens: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut expecting_value = false;
    for tok in tokens {
        if expecting_value {
            expecting_value = false;
            out.push(tok);
        } else if tok.starts_with("--") {
            expecting_value = !tok.contains('=') && LONG_WITH_VALUE.contains(&tok.as_str());
            out.push(tok);
        } else if is_cluster(&tok) {
            for (i, c) in tok[1..].char_indices() {
                out.push(format!("-{c}"));
                if SHORT_WITH_VALUE.contains(c) {
                    let rest = &tok[1 + i + 1..];
                    if rest.is_empty() {
                        expecting_value = true;
                    } else {
                        out.push(rest.to_string());
                    }
                    break;
                }
            }
        } else {
            expecting_value = tok.len() == 2 && tok.starts_with('-') && SHORT_WITH_VALUE.contains(&tok[1..]);
            out.push(tok);
        }
    }
    out
}

/// Parses a pasted curl command (as copied from a browser's "Copy as cURL",
/// or typed by hand) into method/url/headers/body.
///
/// Understands `-X`, `-H`, `-d` and its variants (repeated `-d` are joined
/// with `&`, `--data-urlencode` is encoded), `--json`, `-F`, `-u` (becomes a
/// Basic `Authorization` header), `-b`, `-A`, `-e`, `-G`, `-I`, `--url`, and
/// the `--flag=value` forms, and clustered short flags (`-sSL`). Flags that take a value but
/// don't matter here (`-o`, `-m`, `--proxy`, ...) are skipped together with their value so it
/// isn't mistaken for the URL; other flags (`--compressed`, `-k`, ...) are
/// ignored. `@file` in a body stays literal text: a pasted command never reads files.
pub fn parse_curl(input: &str) -> Result<ParsedRequest, String> {
    let trimmed = input.trim();
    // A pasted shell prompt ("$ curl ...") is not part of the command.
    let trimmed = trimmed.strip_prefix("$ ").or_else(|| trimmed.strip_prefix("> ")).unwrap_or(trimmed);
    let tokens = shell_words::split(trimmed).map_err(|e| format!("Could not parse that as a shell command: {e}"))?;
    parse_tokens(tokens, None).map(|(request, _)| request)
}

/// Parses curl's arguments as `plunger curl` receives them (no leading `curl`). `-d @file`,
/// `--data-binary @file`, `--json @file` and `@-` (stdin) are read through `read_file`.
pub fn parse_curl_args(
    args: &[String],
    read_file: &mut FileReader,
) -> Result<(ParsedRequest, CurlOptions), String> {
    parse_tokens(args.to_vec(), Some(read_file))
}

fn parse_tokens(
    tokens: Vec<String>,
    mut read_file: Option<&mut FileReader>,
) -> Result<(ParsedRequest, CurlOptions), String> {
    if tokens.is_empty() {
        return Err("Nothing to parse.".to_string());
    }

    let mut iter = expand_clusters(tokens).into_iter().peekable();
    if let Some(first) = iter.peek() {
        if first.eq_ignore_ascii_case("curl") || first.eq_ignore_ascii_case("curl.exe") {
            iter.next();
        } else if read_file.is_none() && !first.contains("://") {
            return Err("That doesn't look like a curl command: it should start with `curl` (or be a URL).".to_string());
        }
    }

    let mut opts = CurlOptions::default();
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
        // A body value; `@path` is read from a file (curl strips line breaks for -d and --data-ascii).
        let mut body_value = |raw: Option<String>, strip_newlines: bool| -> Result<Option<String>, String> {
            match (raw, read_file.as_mut()) {
                (Some(spec), Some(read)) if spec.starts_with('@') => {
                    let text = read(&spec[1..])?;
                    Ok(Some(if strip_newlines { text.replace(['\r', '\n'], "") } else { text }))
                }
                (raw, _) => Ok(raw),
            }
        };
        match flag.as_str() {
            "-X" | "--request" => method = value(),
            "-H" | "--header" => {
                if let Some((k, v)) = value().as_deref().and_then(|h| h.split_once(':')) {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            "-d" | "--data" | "--data-ascii" => data.extend(body_value(value(), true)?),
            "--data-binary" => data.extend(body_value(value(), false)?),
            "--data-raw" => data.extend(value()),
            "--data-urlencode" => data.extend(value().map(|v| encode_data(&v))),
            "--json" => json = body_value(value(), false)?,
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
            "-I" | "--head" => {
                head_mode = true;
                opts.head_only = true;
            }
            "-i" | "--include" => opts.include_headers = true,
            "-s" | "--silent" => opts.silent = true,
            "-S" | "--show-error" => opts.show_error = true,
            "-f" | "--fail" => opts.fail = true,
            "--fail-with-body" => opts.fail_with_body = true,
            "-L" | "--location" => opts.follow = true,
            "-k" | "--insecure" => opts.insecure = true,
            "-m" | "--max-time" => opts.max_time = value().and_then(|v| v.parse().ok()),
            "-o" | "--output" => opts.output = value(),
            "-w" | "--write-out" => opts.write_out = value(),
            // Changes how curl would connect or authenticate; Plunger cannot honour these.
            "--retry" => opts.retry = value().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--retry-delay" => opts.retry_delay = value().and_then(|v| v.parse().ok()),
            "--retry-max-time" => opts.retry_max_time = value().and_then(|v| v.parse().ok()),
            "--retry-all-errors" => opts.retry_all_errors = true,
            "--retry-connrefused" => {}
            "-x" | "--proxy" | "-U" | "--proxy-user" | "--cacert" | "--cert" | "--key" | "-T" | "--upload-file"
            | "-K" | "--config" | "--resolve" | "--interface" | "-c" | "--cookie-jar" => {
                opts.unsupported.push(flag.clone());
                value();
            }
            "--connect-timeout" | "--max-redirs" => {
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

    let request = ParsedRequest { method: method.to_uppercase(), url, headers, body, form_fields };
    Ok((request, opts))
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
    }

    #[test]
    fn text_that_is_not_a_curl_command_is_refused() {
        for text in ["not a curl command", "hello", "error: connection refused", "-X POST"] {
            assert!(parse_curl(text).is_err(), "{text:?}");
        }
        assert_eq!(parse_curl("$ curl http://h/x").unwrap().url, "http://h/x");
        assert_eq!(parse_curl("> curl.exe http://h/y").unwrap().url, "http://h/y");
        assert_eq!(parse_curl("http://h/z -X PUT").unwrap().method, "PUT");
        assert!(parse_curl("curl -H 'A: b'").is_err());
        assert!(parse_curl("curl 'unterminated").is_err());
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn no_files(spec: &str) -> Result<String, String> {
        Err(format!("unexpected file read: {spec}"))
    }

    #[test]
    fn clustered_short_flags_are_split() {
        let (r, o) = parse_curl_args(&args(&["-sSL", "-XPOST", "-H", "A: b", "-ofile.txt", "http://h/x"]), &mut no_files).unwrap();
        assert_eq!((r.method.as_str(), r.url.as_str()), ("POST", "http://h/x"));
        assert!(o.silent && o.show_error && o.follow);
        assert_eq!(o.output.as_deref(), Some("file.txt"));
        assert_eq!(r.headers, vec![("A".to_string(), "b".to_string())]);
        // a cluster that ends in a value flag takes the next argument as its value
        let (r, o) = parse_curl_args(&args(&["-fsSLo", "out.bin", "http://h/y"]), &mut no_files).unwrap();
        assert_eq!(r.url, "http://h/y");
        assert!(o.fail && o.silent && o.follow);
        assert_eq!(o.output.as_deref(), Some("out.bin"));
    }

    #[test]
    fn a_value_that_looks_like_a_flag_is_not_split() {
        let (r, _) = parse_curl_args(&args(&["-H", "-Weird: yes", "-d", "-abc", "http://h"]), &mut no_files).unwrap();
        assert_eq!(r.headers, vec![("-Weird".to_string(), "yes".to_string())]);
        assert_eq!(r.body.as_deref(), Some("-abc"));
    }

    #[test]
    fn transfer_options_are_recorded() {
        let (_, o) = parse_curl_args(
            &args(&["-i", "-k", "-m", "2.5", "-w", "%{http_code}", "--fail-with-body", "--compressed", "http://h"]),
            &mut no_files,
        )
        .unwrap();
        assert!(o.include_headers && o.insecure && o.fail_with_body);
        assert_eq!(o.max_time, Some(2.5));
        assert_eq!(o.write_out.as_deref(), Some("%{http_code}"));
        assert!(o.unsupported.is_empty());
        let (r, o) = parse_curl_args(&args(&["-I", "http://h"]), &mut no_files).unwrap();
        assert!(o.head_only);
        assert_eq!(r.method, "HEAD");
    }

    #[test]
    fn at_file_bodies_are_read_only_when_a_reader_is_given() {
        let mut read = |spec: &str| -> Result<String, String> {
            assert_eq!(spec, "body.json");
            Ok("{\"a\":1}
".to_string())
        };
        let (r, _) = parse_curl_args(&args(&["--data-binary", "@body.json", "http://h"]), &mut read).unwrap();
        assert_eq!(r.body.as_deref(), Some("{\"a\":1}
"));
        let (r, _) = parse_curl_args(&args(&["-d", "@body.json", "http://h"]), &mut read).unwrap();
        assert_eq!(r.body.as_deref(), Some("{\"a\":1}"), "-d strips line breaks like curl");
        let (r, _) = parse_curl_args(&args(&["--json", "@body.json", "http://h"]), &mut read).unwrap();
        assert_eq!(r.body.as_deref(), Some("{\"a\":1}
"));
        let (r, _) = parse_curl_args(&args(&["--data-raw", "@body.json", "http://h"]), &mut no_files).unwrap();
        assert_eq!(r.body.as_deref(), Some("@body.json"), "--data-raw is never a file");
        // a pasted command never reads files
        assert_eq!(parse_curl("curl -d @body.json http://h").unwrap().body.as_deref(), Some("@body.json"));
        assert!(parse_curl_args(&args(&["-d", "@missing", "http://h"]), &mut |_| Err("no such file".into())).is_err());
    }

    #[test]
    fn retry_options_are_recorded() {
        let (r, o) = parse_curl_args(&args(&["--retry", "3", "--retry-delay", "2", "--retry-max-time=30", "--retry-all-errors", "--retry-connrefused", "http://h"]), &mut no_files).unwrap();
        assert_eq!((o.retry, o.retry_delay, o.retry_max_time, o.retry_all_errors), (3, Some(2.0), Some(30.0), true));
        assert!(o.unsupported.is_empty());
        assert_eq!(r.url, "http://h");
    }

    #[test]
    fn options_plunger_cannot_honour_are_reported_not_dropped() {
        let (_, o) = parse_curl_args(&args(&["-x", "http://proxy:8080", "--cert", "c.pem", "--resolve", "h:80:1.2.3.4", "http://h"]), &mut no_files).unwrap();
        assert_eq!(o.unsupported, vec!["-x", "--cert", "--resolve"]);
        // pasted commands still just skip them
        assert_eq!(parse_curl("curl --proxy http://p:1 http://h/z").unwrap().url, "http://h/z");
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
