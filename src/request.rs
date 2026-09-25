//! Turns the form state into a ready-to-send request: variables substituted,
//! query parameters appended, body assembled. Pure logic, no network.

use crate::model::{BodyMode, FormField, PersistedState};
use crate::query::{encode_value, split_url};
use crate::redact::is_sensitive_header;
use crate::vars::Resolver;
use std::net::IpAddr;
use std::time::Duration;

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

/// The text form of a header list: one `Name: value` per line.
pub fn headers_to_text(headers: &[(String, String)]) -> String {
    headers.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n")
}

/// What Send does, in one place for the window and for agents: fill in a
/// missing scheme, then build. A URL that uses variables keeps its scheme-less
/// form (a variable may supply the scheme, and the field must keep the
/// `{{template}}` rather than a resolved secret).
pub fn prepare_to_send(state: &mut PersistedState, bearer_token: &str) -> Result<OutgoingRequest, String> {
    if !state.url.contains("{{") {
        state.url = normalize_url(&state.url);
    }
    build_request(state, bearer_token)
}

/// Turns the form state (plus the never-persisted bearer token) into the
/// request that will actually be sent: variables substituted, body
/// assembled. Fails, without sending anything, if a
/// `{{variable}}` is undefined or the URL is invalid.
pub fn build_request(state: &PersistedState, bearer_token: &str) -> Result<OutgoingRequest, String> {
    let mut r = Resolver::new(&state.variables);

    // Query params already live in the URL (the Params tab edits it). Values
    // substituted into the query are encoded so `&`, `#` or `+` in a variable
    // can't split or corrupt it.
    let (base, query, fragment) = split_url(&state.url);
    let mut url_text = r.apply(base);
    if let Some(q) = query {
        url_text.push('?');
        url_text.push_str(&r.apply_with(q, encode_value));
    }
    if let Some(f) = fragment {
        url_text.push('#');
        url_text.push_str(&r.apply(f));
    }
    let url_text = normalize_url(&url_text);
    let headers_text = r.apply(&state.headers_text);
    let bearer = r.apply(bearer_token.trim());

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
    let url = reqwest::Url::parse(&url_text).map_err(|e| format!("Invalid URL \"{url_text}\": {e}"))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FieldKind, KeyValue, Variable};

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
            remember: false,
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
    fn the_query_comes_from_the_url_not_the_param_rows() {
        let mut s = state();
        s.url = "http://h/x?a=1&q=a%20b%26c&é=ü".into();
        // Rows mirror the URL; sending must not append them a second time.
        s.params = vec![kv("a", "1", true), kv("skip", "me", false)];
        let r = build(&s, "");
        assert_eq!(r.url, "http://h/x?a=1&q=a%20b%26c&%C3%A9=%C3%BC");
    }

    #[test]
    fn variables_in_the_query_are_encoded_but_not_in_the_path() {
        let mut s = state();
        s.variables = vec![var("seg", "a/b"), var("tok", "x&y=1+2#z")];
        s.url = "http://h/{{seg}}?t={{tok}}&n=1#{{seg}}".into();
        let r = build(&s, "");
        assert_eq!(r.url, "http://h/a/b?t=x%26y=1%2B2%23z&n=1#a/b");
    }

    #[test]
    fn variables_are_substituted_everywhere() {
        let mut s = state();
        s.variables = vec![var("host", "localhost:3000"), var("id", "42"), var("tok", "T0K"), var("name", "Ann")];
        s.url = "{{host}}/users/{{id}}?who={{name}}".into();
        s.headers_text = "X-Trace: {{id}}".into();
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

}
