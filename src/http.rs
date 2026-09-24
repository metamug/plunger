use crate::json_view::pretty_json_if_possible;
use crate::model::{BodyMode, PersistedState, ResponseData, SendResult};
use std::net::IpAddr;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// A fully-resolved request, ready to hand to the network thread.
pub struct OutgoingRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
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
/// request that will actually be sent.
pub fn build_request(state: &PersistedState, bearer_token: &str) -> OutgoingRequest {
    let mut headers = parse_headers(&state.headers_text);
    let bearer = bearer_token.trim();
    if !bearer.is_empty() {
        ensure_header(&mut headers, "Authorization", &format!("Bearer {bearer}"));
    }

    let body = match state.body_mode {
        BodyMode::None => None,
        BodyMode::Json => {
            ensure_header(&mut headers, "Content-Type", "application/json");
            Some(state.json_body.clone())
        }
        BodyMode::UrlEncoded => {
            ensure_header(&mut headers, "Content-Type", "application/x-www-form-urlencoded");
            Some(
                state
                    .urlencoded_body
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join("&"),
            )
        }
        BodyMode::Raw => Some(state.raw_body.clone()),
    };

    OutgoingRequest {
        method: state.method.clone(),
        url: normalize_url(&state.url),
        headers,
        body,
    }
}

pub fn send_request(req: OutgoingRequest, tx: Sender<SendResult>) {
    std::thread::spawn(move || {
        let _ = tx.send(execute(req));
    });
}

fn execute(req: OutgoingRequest) -> SendResult {
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;

    let method = reqwest::Method::from_bytes(req.method.as_bytes()).map_err(|_| "Invalid HTTP method".to_string())?;

    let mut builder = client.request(method, &req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    if let Some(b) = req.body {
        builder = builder.body(b);
    }

    let start = Instant::now();
    let res = builder.send().map_err(|e| e.to_string())?;
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

    // Read raw bytes so the reported size is the real payload size even when
    // the body isn't valid UTF-8; decode lossily only for display.
    let bytes = res.bytes().map_err(|e| e.to_string())?;
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PersistedState {
        PersistedState {
            method: "POST".into(),
            url: "api.example.com/x".into(),
            ..Default::default()
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
        let r = build_request(&s, "  tok  ");
        assert_eq!(r.url, "https://api.example.com/x");
        assert_eq!(r.body.as_deref(), Some("{\"a\":1}"));
        assert!(r.headers.contains(&("Authorization".into(), "Bearer tok".into())));
        assert!(r.headers.contains(&("Content-Type".into(), "application/json".into())));
    }

    #[test]
    fn build_request_respects_user_supplied_headers() {
        let mut s = state();
        s.body_mode = BodyMode::Json;
        s.headers_text = "authorization: Basic abc\nContent-Type: application/vnd.api+json".into();
        let r = build_request(&s, "tok");
        assert_eq!(r.headers.len(), 2);
        assert!(r.headers.iter().all(|(_, v)| !v.contains("Bearer")));
    }

    #[test]
    fn build_request_urlencoded_joins_lines() {
        let mut s = state();
        s.body_mode = BodyMode::UrlEncoded;
        s.urlencoded_body = "a=1\n\n  b=2  \n".into();
        let r = build_request(&s, "");
        assert_eq!(r.body.as_deref(), Some("a=1&b=2"));
        assert!(r
            .headers
            .contains(&("Content-Type".into(), "application/x-www-form-urlencoded".into())));
    }

    #[test]
    fn build_request_none_has_no_body_or_content_type() {
        let r = build_request(&state(), "");
        assert!(r.body.is_none());
        assert!(r.headers.is_empty());
    }
}
