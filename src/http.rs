use crate::json_view::pretty_json_if_possible;
use crate::model::{BodyMode, FieldKind, FormField, PersistedState, ResponseData, SendResult};
use crate::redact::is_sensitive_header;
use crate::vars::Resolver;
use reqwest::blocking::multipart::Form;
use std::io::Read;
use std::net::IpAddr;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// Bodies larger than this are cut off: they'd otherwise be held in memory
/// and laid out by the UI in full, which freezes the window.
pub const MAX_BODY_BYTES: u64 = 10 * 1024 * 1024;
const MAX_REDIRECTS: usize = 10;
const MIN_TIMEOUT_SECS: u64 = 1;
const MAX_TIMEOUT_SECS: u64 = 600;

pub enum OutgoingBody {
    None,
    Text(String),
    Multipart(Vec<FormField>),
}

/// A fully-resolved request (variables substituted), ready for the network thread.
pub struct OutgoingRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: OutgoingBody,
    pub timeout: Duration,
    pub follow_redirects: bool,
    pub insecure_tls: bool,
}

pub fn parse_headers(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.trim().to_string()))
        })
        .collect()
}

pub fn format_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

/// Adds a scheme when the user typed a bare host, e.g. `localhost:3000/api`.
/// Local and private-network hosts get `http://` (that's where dev APIs
/// live); everything else gets `https://`.
pub fn normalize_url(input: &str) -> String {
    let url = input.trim();
    if url.is_empty() || url.contains("://") {
        return url.to_string();
    }
    let authority = url.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    let scheme = if is_local_host(host) { "http" } else { "https" };
    format!("{scheme}://{url}")
}

fn is_local_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_unspecified(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Err(_) => false,
    }
}

/// Adds `name: value` unless a header with that name (any case) is present.
pub fn ensure_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name)) {
        headers.push((name.to_string(), value.to_string()));
    }
}

/// Turns the form state (plus the never-persisted bearer token) into the
/// request that will actually be sent: variables substituted, query
/// parameters appended, body assembled. Fails, without sending anything, if a
/// `{{variable}}` is undefined or the URL is invalid.
pub fn build_request(state: &PersistedState, bearer_token: &str) -> Result<OutgoingRequest, String> {
    let mut r = Resolver::new(&state.variables);

    let url_text = normalize_url(&r.apply(&state.url));
    let headers_text = r.apply(&state.headers_text);
    let bearer = r.apply(bearer_token.trim());
    let params: Vec<(String, String)> = state
        .params
        .iter()
        .filter(|p| p.enabled && !p.key.trim().is_empty())
        .map(|p| (r.apply(p.key.trim()), r.apply(&p.value)))
        .collect();

    let mut headers = parse_headers(&headers_text);
    // A credential header with an empty value is a blanked placeholder from
    // disk (see redact.rs), not something the user meant to send.
    headers.retain(|(k, v)| !(v.is_empty() && is_sensitive_header(k)));
    let bearer = bearer.trim();
    if !bearer.is_empty() {
        ensure_header(&mut headers, "Authorization", &format!("Bearer {bearer}"));
    }

    let body = match state.body_mode {
        BodyMode::None => OutgoingBody::None,
        BodyMode::Json => {
            ensure_header(&mut headers, "Content-Type", "application/json");
            OutgoingBody::Text(r.apply(&state.json_body))
        }
        BodyMode::UrlEncoded => {
            ensure_header(&mut headers, "Content-Type", "application/x-www-form-urlencoded");
            let lines: Vec<String> = state
                .urlencoded_body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| r.apply(l))
                .collect();
            OutgoingBody::Text(lines.join("&"))
        }
        BodyMode::Raw => OutgoingBody::Text(r.apply(&state.raw_body)),
        BodyMode::Multipart => {
            // reqwest sets the header itself, with the generated boundary.
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
            let fields = state
                .multipart_fields
                .iter()
                .filter(|f| f.enabled && !f.key.trim().is_empty())
                .map(|f| FormField {
                    key: r.apply(f.key.trim()),
                    kind: f.kind,
                    value: r.apply(&f.value),
                    enabled: true,
                })
                .collect();
            OutgoingBody::Multipart(fields)
        }
    };

    r.finish()?;

    if url_text.is_empty() {
        return Err("Enter a URL.".to_string());
    }
    let mut url = reqwest::Url::parse(&url_text).map_err(|e| format!("Invalid URL \"{url_text}\": {e}"))?;
    if !params.is_empty() {
        let mut query = url.query_pairs_mut();
        for (k, v) in &params {
            query.append_pair(k, v);
        }
    }

    Ok(OutgoingRequest {
        method: state.method.clone(),
        url: url.into(),
        headers,
        body,
        timeout: Duration::from_secs(state.timeout_secs.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS)),
        follow_redirects: state.follow_redirects,
        insecure_tls: state.insecure_tls,
    })
}

pub fn send_request(req: OutgoingRequest, tx: Sender<SendResult>) {
    std::thread::spawn(move || {
        let _ = tx.send(execute(req));
    });
}

/// reqwest's own message is just "error sending request for url (...)"; the
/// useful part (refused, DNS failure, bad certificate, timeout) is in the
/// source chain, so join it all together.
fn describe_error(err: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(e) = source {
        let msg = e.to_string();
        if parts.last() != Some(&msg) {
            parts.push(msg);
        }
        source = e.source();
    }
    parts.join(": ")
}

fn multipart_form(fields: Vec<FormField>) -> Result<Form, String> {
    let mut form = Form::new();
    for f in fields {
        form = match f.kind {
            FieldKind::Text => form.text(f.key, f.value),
            FieldKind::File => {
                if f.value.trim().is_empty() {
                    return Err(format!("Field \"{}\" is a file field but no file is chosen.", f.key));
                }
                form.file(f.key, &f.value)
                    .map_err(|e| format!("Could not read file \"{}\": {e}", f.value))?
            }
        };
    }
    Ok(form)
}

fn execute(req: OutgoingRequest) -> SendResult {
    let redirect = if req.follow_redirects {
        reqwest::redirect::Policy::limited(MAX_REDIRECTS)
    } else {
        reqwest::redirect::Policy::none()
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(req.timeout)
        .redirect(redirect)
        .danger_accept_invalid_certs(req.insecure_tls)
        .build()
        .map_err(|e| e.to_string())?;

    let method = reqwest::Method::from_bytes(req.method.as_bytes()).map_err(|_| "Invalid HTTP method".to_string())?;

    let mut builder = client.request(method, &req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    builder = match req.body {
        OutgoingBody::None => builder,
        OutgoingBody::Text(text) => builder.body(text),
        OutgoingBody::Multipart(fields) => builder.multipart(multipart_form(fields)?),
    };

    let start = Instant::now();
    let res = builder.send().map_err(|e| describe_error(&e))?;
    let elapsed_ms = start.elapsed().as_millis();

    let status = res.status().as_u16();
    let status_text = res.status().canonical_reason().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = res
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
        .collect();
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Read raw bytes (capped) so the reported size is the real payload size
    // even when the body isn't valid UTF-8; decode lossily only for display.
    let total_size = res.content_length();
    let mut bytes = Vec::new();
    res.take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > MAX_BODY_BYTES;
    if truncated {
        bytes.truncate(MAX_BODY_BYTES as usize);
    }
    let size_bytes = bytes.len();
    let text = String::from_utf8_lossy(&bytes).into_owned();

    let trimmed = text.trim_start();
    let looks_json = content_type.contains("json") || trimmed.starts_with('{') || trimmed.starts_with('[');
    let (body, json_value) = if looks_json {
        pretty_json_if_possible(&text)
    } else {
        (text, None)
    };

    Ok(ResponseData {
        status,
        status_text,
        elapsed_ms,
        size_bytes,
        headers,
        body,
        json_value,
        truncated,
        total_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyValue, Variable};
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn state() -> PersistedState {
        PersistedState {
            method: "POST".into(),
            url: "api.example.com/x".into(),
            ..Default::default()
        }
    }

    fn build(s: &PersistedState, bearer: &str) -> OutgoingRequest {
        build_request(s, bearer).unwrap()
    }

    fn text_body(r: &OutgoingRequest) -> Option<&str> {
        match &r.body {
            OutgoingBody::Text(t) => Some(t),
            _ => None,
        }
    }

    fn var(name: &str, value: &str) -> Variable {
        Variable {
            name: name.into(),
            value: value.into(),
            secret: false,
        }
    }

    fn kv(key: &str, value: &str, enabled: bool) -> KeyValue {
        KeyValue {
            key: key.into(),
            value: value.into(),
            enabled,
        }
    }

    #[test]
    fn parse_headers_skips_blank_and_malformed_lines() {
        let h = parse_headers("Accept: */*\n\n  X-Trim :  spaced  \nno-colon\n: no-key\nA: b:c");
        assert_eq!(
            h,
            vec![
                ("Accept".to_string(), "*/*".to_string()),
                ("X-Trim".to_string(), "spaced".to_string()),
                ("A".to_string(), "b:c".to_string()),
            ]
        );
    }

    #[test]
    fn format_bytes_picks_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn normalize_url_leaves_full_urls_alone() {
        assert_eq!(normalize_url("https://a.com/x"), "https://a.com/x");
        assert_eq!(normalize_url("  http://a.com  "), "http://a.com");
        assert_eq!(normalize_url(""), "");
    }

    #[test]
    fn normalize_url_uses_http_for_local_hosts() {
        for u in [
            "localhost:3000/api",
            "localhost",
            "127.0.0.1:8080",
            "192.168.1.20/x",
            "10.0.0.5:9000",
            "[::1]:3000/x",
            "app.local/x",
            "user:pw@localhost:3000/x",
            "0.0.0.0:80",
        ] {
            assert_eq!(normalize_url(u), format!("http://{u}"), "{u}");
        }
    }

    #[test]
    fn normalize_url_uses_https_for_public_hosts() {
        assert_eq!(normalize_url("api.example.com/users?a=b"), "https://api.example.com/users?a=b");
        assert_eq!(normalize_url("8.8.8.8/x"), "https://8.8.8.8/x");
        assert_eq!(normalize_url("localhost.evil.com/x"), "https://localhost.evil.com/x");
    }

    #[test]
    fn ensure_header_is_case_insensitive() {
        let mut h = vec![("content-type".to_string(), "text/plain".to_string())];
        ensure_header(&mut h, "Content-Type", "application/json");
        ensure_header(&mut h, "X-New", "1");
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].1, "text/plain");
    }

    #[test]
    fn build_request_json_adds_content_type_and_bearer() {
        let mut s = state();
        s.body_mode = BodyMode::Json;
        s.json_body = "{\"a\":1}".into();
        let r = build(&s, "  tok  ");
        assert_eq!(r.url, "https://api.example.com/x");
        assert_eq!(text_body(&r), Some("{\"a\":1}"));
        assert!(r.headers.contains(&("Authorization".into(), "Bearer tok".into())));
        assert!(r.headers.contains(&("Content-Type".into(), "application/json".into())));
    }

    #[test]
    fn build_request_respects_user_supplied_headers() {
        let mut s = state();
        s.body_mode = BodyMode::Json;
        s.headers_text = "authorization: Basic abc\nContent-Type: application/vnd.api+json".into();
        let r = build(&s, "tok");
        assert_eq!(r.headers.len(), 2);
        assert!(r.headers.iter().all(|(_, v)| !v.contains("Bearer")));
    }

    #[test]
    fn build_request_urlencoded_joins_lines() {
        let mut s = state();
        s.body_mode = BodyMode::UrlEncoded;
        s.urlencoded_body = "a=1\n\n  b=2  \n".into();
        let r = build(&s, "");
        assert_eq!(text_body(&r), Some("a=1&b=2"));
        assert!(r
            .headers
            .contains(&("Content-Type".into(), "application/x-www-form-urlencoded".into())));
    }

    #[test]
    fn build_request_none_has_no_body_or_content_type() {
        let r = build(&state(), "");
        assert!(matches!(r.body, OutgoingBody::None));
        assert!(r.headers.is_empty());
    }

    #[test]
    fn build_request_skips_blanked_credential_headers_only() {
        let mut s = state();
        s.headers_text = "Authorization:\nX-Api-Key: real\nAccept:".into();
        let r = build(&s, "");
        assert_eq!(
            r.headers,
            vec![
                ("X-Api-Key".to_string(), "real".to_string()),
                ("Accept".to_string(), String::new())
            ]
        );
    }

    #[test]
    fn build_request_clamps_timeout_and_carries_options() {
        let mut s = state();
        s.timeout_secs = 0;
        s.follow_redirects = false;
        s.insecure_tls = true;
        let r = build(&s, "");
        assert_eq!(r.timeout, Duration::from_secs(1));
        assert!(!r.follow_redirects && r.insecure_tls);
        s.timeout_secs = 99_999;
        assert_eq!(build(&s, "").timeout, Duration::from_secs(600));
    }

    #[test]
    fn query_params_are_appended_encoded_and_respect_the_enabled_flag() {
        let mut s = state();
        s.url = "http://h/x?a=1".into();
        s.params = vec![
            kv("q", "a b&c", true),
            kv("skip", "me", false),
            kv("", "no key", true),
            kv("é", "ü", true),
        ];
        let r = build(&s, "");
        assert_eq!(r.url, "http://h/x?a=1&q=a+b%26c&%C3%A9=%C3%BC");
    }

    #[test]
    fn variables_are_substituted_everywhere() {
        let mut s = state();
        s.variables = vec![var("host", "localhost:3000"), var("id", "42"), var("tok", "T0K"), var("name", "Ann")];
        s.url = "{{host}}/users/{{id}}".into();
        s.headers_text = "X-Trace: {{id}}".into();
        s.params = vec![kv("who", "{{name}}", true)];
        s.body_mode = BodyMode::Json;
        s.json_body = "{\"name\":\"{{name}}\",\"nested\":{\"a\":{\"b\":1}}}".into();
        let r = build(&s, "{{tok}}");
        assert_eq!(r.url, "http://localhost:3000/users/42?who=Ann");
        assert!(r.headers.contains(&("X-Trace".into(), "42".into())));
        assert!(r.headers.contains(&("Authorization".into(), "Bearer T0K".into())));
        assert_eq!(text_body(&r), Some("{\"name\":\"Ann\",\"nested\":{\"a\":{\"b\":1}}}"));
    }

    #[test]
    fn undefined_variables_block_the_send_and_disabled_rows_are_ignored() {
        let mut s = state();
        s.url = "http://h/{{missing}}".into();
        s.params = vec![kv("k", "{{alsoMissing}}", false)];
        let err = build_request(&s, "").err().unwrap();
        assert!(err.contains("{{missing}}") && !err.contains("alsoMissing"), "{err}");
    }

    #[test]
    fn invalid_or_empty_urls_give_a_readable_error() {
        let mut s = state();
        s.url = "  ".into();
        assert_eq!(build_request(&s, "").err().unwrap(), "Enter a URL.");
        s.url = "http://".into();
        assert!(build_request(&s, "").err().unwrap().starts_with("Invalid URL"));
    }

    #[test]
    fn multipart_drops_the_content_type_header_and_keeps_enabled_named_fields() {
        let mut s = state();
        s.body_mode = BodyMode::Multipart;
        s.headers_text = "Content-Type: application/json\nAccept: */*".into();
        s.variables = vec![var("who", "Ann")];
        s.multipart_fields = vec![
            FormField { key: "title".into(), kind: FieldKind::Text, value: "hi {{who}}".into(), enabled: true },
            FormField { key: "off".into(), kind: FieldKind::Text, value: "x".into(), enabled: false },
            FormField { key: "".into(), kind: FieldKind::Text, value: "orphan".into(), enabled: true },
        ];
        let r = build(&s, "");
        assert_eq!(r.headers, vec![("Accept".to_string(), "*/*".to_string())]);
        let OutgoingBody::Multipart(fields) = r.body else { panic!("expected multipart") };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].value, "hi Ann");
    }

    // ---- real requests against a throwaway loopback server -----------------

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// Serves one canned response and reports the raw request it received.
    fn serve_once(head: String, body: Vec<u8>, stall: Option<Duration>) -> (String, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                if let Some(pos) = find(&data, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                    let content_length = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok());
                    let done = match content_length {
                        Some(cl) => data.len() >= pos + 4 + cl,
                        None if head.contains("transfer-encoding: chunked") => data.ends_with(b"0\r\n\r\n"),
                        None => true,
                    };
                    if done {
                        break;
                    }
                }
            }
            let _ = tx.send(data);
            if let Some(d) = stall {
                std::thread::sleep(d);
            }
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        });
        (format!("http://127.0.0.1:{port}/"), rx)
    }

    fn ok_head(len: usize) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n")
    }

    fn req(url: String) -> OutgoingRequest {
        let mut r = build(&state(), "");
        r.method = "GET".into();
        r.url = url;
        r
    }

    #[test]
    fn execute_parses_a_json_response() {
        let (url, _) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 7\r\nConnection: close\r\n\r\n".into(),
            br#"{"a":1}"#.to_vec(),
            None,
        );
        let r = execute(req(url)).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.size_bytes, 7);
        assert!(r.json_value.is_some() && !r.truncated);
    }

    #[test]
    fn execute_truncates_oversized_bodies() {
        let total = MAX_BODY_BYTES as usize + 100;
        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n");
        let (url, _) = serve_once(head, vec![b'a'; total], None);
        let r = execute(req(url)).unwrap();
        assert!(r.truncated);
        assert_eq!(r.size_bytes as u64, MAX_BODY_BYTES);
        assert_eq!(r.total_size, Some(total as u64));
    }

    #[test]
    fn execute_can_stop_at_a_redirect() {
        let (url, _) = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/never\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            vec![],
            None,
        );
        let mut r = req(url);
        r.follow_redirects = false;
        assert_eq!(execute(r).unwrap().status, 302);
    }

    #[test]
    fn execute_times_out() {
        let (url, _) = serve_once(ok_head(0), vec![], Some(Duration::from_secs(4)));
        let mut r = req(url);
        r.timeout = Duration::from_secs(1);
        let started = Instant::now();
        assert!(execute(r).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn execute_reports_the_underlying_cause_not_just_a_generic_error() {
        let err = execute(req("http://127.0.0.1:1/".into())).err().unwrap();
        // reqwest alone says only "error sending request for url (...)".
        assert!(err.contains("error sending request") && err.contains("(Connect)"), "{err}");
    }

    #[test]
    fn multipart_upload_sends_text_and_file_parts_with_a_boundary() {
        let path = std::env::temp_dir().join(format!("mat-upload-{}.txt", std::process::id()));
        std::fs::write(&path, "FILEDATA-123").unwrap();

        let mut s = state();
        s.body_mode = BodyMode::Multipart;
        s.headers_text = "Content-Type: application/json".into();
        s.multipart_fields = vec![
            FormField { key: "title".into(), kind: FieldKind::Text, value: "hello".into(), enabled: true },
            FormField { key: "doc".into(), kind: FieldKind::File, value: path.to_string_lossy().into_owned(), enabled: true },
        ];
        let (url, rx) = serve_once(ok_head(2), b"ok".to_vec(), None);
        let mut r = build(&s, "");
        r.url = url;
        assert_eq!(execute(r).unwrap().status, 200);

        let raw = String::from_utf8_lossy(&rx.recv().unwrap()).into_owned();
        let lower = raw.to_lowercase();
        assert!(lower.contains("content-type: multipart/form-data; boundary="), "{raw}");
        assert_eq!(lower.matches("content-type: multipart/form-data").count(), 1);
        assert!(!lower.contains("application/json"), "{raw}");
        assert!(raw.contains("name=\"title\"") && raw.contains("hello"));
        let file_name = path.file_name().unwrap().to_string_lossy();
        assert!(raw.contains(&format!("name=\"doc\"; filename=\"{file_name}\"")), "{raw}");
        assert!(raw.contains("FILEDATA-123"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn multipart_reports_a_missing_or_unchosen_file() {
        let mut s = state();
        s.body_mode = BodyMode::Multipart;
        s.multipart_fields = vec![FormField {
            key: "doc".into(),
            kind: FieldKind::File,
            value: "Z:/definitely/not/here.bin".into(),
            enabled: true,
        }];
        let err = execute(build(&s, "")).err().unwrap();
        assert!(err.contains("Could not read file"), "{err}");

        s.multipart_fields[0].value = String::new();
        let err = execute(build(&s, "")).err().unwrap();
        assert!(err.contains("no file is chosen"), "{err}");
    }
}
