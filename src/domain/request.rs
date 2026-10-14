//! Turns the form state into a ready-to-send request: variables substituted,
//! query parameters appended, body assembled. Pure logic, no network.

use crate::domain::model::{BodyMode, FormField, PersistedState};
use crate::domain::query::{encode_value, split_url};
use crate::domain::redact::is_sensitive_header;
use crate::domain::vars::Resolver;
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

/// A value in a form body: everything the query encoding does, plus `=`, so a value can never look like
/// another `key=value` pair.
fn form_value(value: &str) -> String {
    encode_value(value).replace('=', "%3D")
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

/// Refuses `https:///path`: the URL parser quietly reads it as host `path`, so the request would go
/// to the wrong server instead of failing.
fn check_has_host(url: &str) -> Result<(), String> {
    let Some((scheme, rest)) = url.split_once("://") else { return Ok(()) };
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let has_credentials_only = authority.rsplit('@').next().unwrap_or("").is_empty();
    if authority.is_empty() || has_credentials_only {
        return Err(format!(
            "Invalid URL \"{url}\": there is no host after \"{scheme}://\". A URL has exactly two slashes there, then the host (for example {scheme}://example.com/path)."
        ));
    }
    Ok(())
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

/// Headers are stored as text, one per line, so a value containing a line break
/// would silently become a second header. Refuse it instead.
pub fn check_header_lines<'a>(headers: impl IntoIterator<Item = (&'a String, &'a String)>) -> Result<(), String> {
    for (k, v) in headers {
        if k.contains(['\r', '\n']) || v.contains(['\r', '\n']) {
            return Err(format!("Header \"{}\" contains a line break, which isn't allowed in a header.", k.trim()));
        }
    }
    Ok(())
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
    // a value in the path cannot start a query or a fragment; the origin may be a whole base URL
    let origin = crate::domain::query::origin_end(base).min(base.len());
    let mut url_text = r.apply(&base[..origin]);
    url_text.push_str(&r.apply_with(&base[origin..], crate::domain::query::encode_path_value));
    if let Some(q) = query {
        url_text.push('?');
        url_text.push_str(&r.apply_with(q, encode_value));
    }
    if let Some(f) = fragment {
        url_text.push('#');
        url_text.push_str(&r.apply(f));
    }
    let url_text = normalize_url(&url_text);
    // one header per line: a value with a line break would start another header
    let mut header_lines = Vec::new();
    for line in state.headers_text.lines() {
        let filled = r.apply(line);
        if line.contains("{{") && filled.lines().count() > 1 {
            let name = line.split(':').next().unwrap_or("").trim();
            return Err(format!("A variable in the \"{name}\" header has a line break, which a header cannot hold."));
        }
        header_lines.push(filled);
    }
    let headers_text = header_lines.join("\n");
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
            OutgoingBody::Text(r.apply_json(&state.json_body))
        }
        BodyMode::UrlEncoded => {
            ensure_header(&mut headers, "Content-Type", "application/x-www-form-urlencoded");
            let lines: Vec<String> = state
                .urlencoded_body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| match l.split_once('=') {
                    // a value with `&`, `=` or a line break must not become another field
                    Some((key, value)) => format!("{}={}", r.apply(key), r.apply_with(value, form_value)),
                    None => r.apply(l),
                })
                .collect();
            OutgoingBody::Text(lines.join("&"))
        }
        BodyMode::Raw => {
            // A raw body that says it is JSON gets the same care as the JSON editor: values are escaped inside
            // strings and `{{x?}}` can leave a member out. This is how an agent sends a body with an unquoted
            // placeholder (`"labels": {{labels}}`), which is not a JSON value it could give as `json`.
            let is_json = headers.iter().any(|(k, v)| {
                let v = v.to_ascii_lowercase();
                k.eq_ignore_ascii_case("content-type") && (v.contains("application/json") || v.contains("+json"))
            });
            OutgoingBody::Text(if is_json { r.apply_json(&state.raw_body) } else { r.apply(&state.raw_body) })
        }
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
    check_has_host(&url_text)?;
    let url = reqwest::Url::parse(&url_text).map_err(|e| format!("Invalid URL \"{url_text}\": {e}"))?;

    if reqwest::Method::from_bytes(state.method.as_bytes()).is_err() {
        return Err(format!("Invalid HTTP method \"{}\".", state.method));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{FieldKind, KeyValue, Variable};

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
    fn a_form_value_with_an_equals_sign_is_encoded() {
        let mut s = state();
        s.method = "POST".into();
        s.body_mode = BodyMode::UrlEncoded;
        s.urlencoded_body = "a={{v}}".into();
        s.variables = vec![var("v", "x y&z=1")];
        assert_eq!(text_body(&build(&s, "")), Some("a=x%20y%26z%3D1"));
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

    /// Builds a JSON body from `template` with `value` as the variable `v`, and parses what would be sent.
    fn json_sent(template: &str, value: &str) -> (String, serde_json::Value) {
        let mut s = state();
        s.method = "POST".into();
        s.variables = vec![var("v", value)];
        s.body_mode = BodyMode::Json;
        s.json_body = template.into();
        let r = build(&s, "");
        let text = text_body(&r).expect("a text body").to_string();
        let parsed = serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text:?}"));
        (text, parsed)
    }

    #[test]
    fn a_variable_in_a_json_string_is_escaped_for_every_kind_of_character() {
        // each of these used to produce a body the server refused ("Illegal unquoted character")
        for value in [
            "line one\nline two",   // a newline, the case that was reported
            "say \"hi\"",           // quotes on their own
            "C:\\temp\\new",        // backslashes on their own
            "a\tb",                 // a tab
            "a\r\nb",               // a Windows line break
            "bell\u{7}and\u{1}ctl", // other control characters
            "\\\"",                 // a backslash then a quote
            "caf\u{e9} \u{65e5}\u{672c} \u{1f9e0}", // non-ASCII is kept
            "{{not a variable}}",   // text that only looks like a placeholder
            "",
        ] {
            let (_, parsed) = json_sent("{\"description\": \"{{v}}\"}", value);
            assert_eq!(parsed["description"], value, "value {value:?}");
        }
    }

    #[test]
    fn escaping_follows_the_json_around_the_placeholder() {
        // text before it in the same string, an escaped quote earlier in the string, several placeholders
        let (_, parsed) = json_sent(r#"{"a": "pre \" {{v}} post", "b": ["{{v}}", "{{v}}"], "{{v}}": 1}"#, "x\"y\nz");
        assert_eq!(parsed["a"], "pre \" x\"y\nz post");
        assert_eq!(parsed["b"], serde_json::json!(["x\"y\nz", "x\"y\nz"]));
        assert_eq!(parsed["x\"y\nz"], 1, "a placeholder in a key is escaped too");
    }

    #[test]
    fn a_variable_outside_a_json_string_is_inserted_as_json() {
        // numbers, booleans and whole arrays or objects: how a list such as labels gets into a body
        let (_, p) = json_sent(r#"{"n": {{v}}, "s": "{{v}}"}"#, "42");
        assert_eq!((p["n"].clone(), p["s"].clone()), (serde_json::json!(42), serde_json::json!("42")));
        let (_, p) = json_sent(r#"{"labels": {{v}}}"#, r#"["a", "b c"]"#);
        assert_eq!(p["labels"], serde_json::json!(["a", "b c"]));
        let (_, p) = json_sent(r#"{"fields": {{v}}}"#, r#"{"x": "quote \" kept"}"#);
        assert_eq!(p["fields"]["x"], "quote \" kept");
    }

    #[test]
    fn only_a_json_body_is_json_escaped() {
        let mut s = state();
        s.method = "POST".into();
        s.variables = vec![var("v", "a\nb \"q\"")];
        s.body_mode = BodyMode::Raw;
        s.raw_body = "text {{v}}".into();
        assert_eq!(text_body(&build(&s, "")), Some("text a\nb \"q\""), "a raw body stays raw");
        s.body_mode = BodyMode::UrlEncoded;
        s.urlencoded_body = "k={{v}}".into();
        assert_eq!(text_body(&build(&s, "")), Some("k=a%0Ab%20%22q%22"), "a form value is percent-encoded");
    }

    fn json_with(template: &str, vars: &[(&str, &str)]) -> Result<serde_json::Value, String> {
        let mut s = state();
        s.method = "POST".into();
        s.variables = vars.iter().map(|(n, v)| var(n, v)).collect();
        s.body_mode = BodyMode::Json;
        s.json_body = template.into();
        let r = build_request(&s, "")?;
        let text = text_body(&r).expect("a text body").to_string();
        Ok(serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text:?}")))
    }

    #[test]
    fn a_default_is_used_when_there_is_no_value() {
        let body = r#"{"assignee": "{{who|unassigned}}", "n": {{n|3}}, "note": "Re: {{topic|none}}"}"#;
        let p = json_with(body, &[]).unwrap();
        assert_eq!((&p["assignee"], &p["n"], &p["note"]), (&serde_json::json!("unassigned"), &serde_json::json!(3), &serde_json::json!("Re: none")));
        // a value wins; an empty value counts as no value
        let p = json_with(body, &[("who", "ann"), ("n", "9"), ("topic", "")]).unwrap();
        assert_eq!((&p["assignee"], &p["n"], &p["note"]), (&serde_json::json!("ann"), &serde_json::json!(9), &serde_json::json!("Re: none")));
        // the default is escaped like a value, and may be empty
        assert_eq!(json_with(r##"{"a": "{{x|say "hi"}}"}"##, &[]).unwrap()["a"], "say \"hi\"");
        assert_eq!(json_with(r#"{"a": "{{x|}}"}"#, &[]).unwrap()["a"], "");
        // a placeholder without one is still refused
        assert!(json_with(r#"{"a": "{{x}}"}"#, &[]).unwrap_err().contains("{{x}}"));
    }

    #[test]
    fn an_optional_placeholder_with_no_value_takes_its_member_out_of_the_body() {
        let body = r#"{"fields": {"summary": "{{summary}}", "assignee": {"name": "{{who?}}"}, "epic": "{{epic?}}", "labels": {{labels?}}, "tags": ["a", "{{tag?}}", "b"]}}"#;
        // nothing given: the optional members are gone, the rest is untouched
        let p = json_with(body, &[("summary", "S")]).unwrap();
        assert_eq!(p, serde_json::json!({"fields": {"summary": "S", "tags": ["a", "b"]}}), "an object emptied by the omission goes too");
        // all given
        let p = json_with(body, &[("summary", "S"), ("who", "ann"), ("epic", "E-1"), ("labels", r#"["x","y"]"#), ("tag", "t")]).unwrap();
        assert_eq!(
            p,
            serde_json::json!({"fields": {"summary": "S", "assignee": {"name": "ann"}, "epic": "E-1", "labels": ["x", "y"], "tags": ["a", "t", "b"]}})
        );
        // an empty value is no value
        let p = json_with(r#"{"a": 1, "b": "{{b?}}"}"#, &[("b", "")]).unwrap();
        assert_eq!(p, serde_json::json!({"a": 1}));
        // inside a longer string it is just empty
        assert_eq!(json_with(r#"{"a": "x{{b?}}y"}"#, &[]).unwrap()["a"], "xy");
        // a body with nothing omitted is sent exactly as written
        let mut s = state();
        s.variables = vec![var("b", "1")];
        s.body_mode = BodyMode::Json;
        s.json_body = "{\n  \"b\": \"{{b?}}\"\n}".into();
        assert_eq!(text_body(&build(&s, "")), Some("{\n  \"b\": \"1\"\n}"));
    }

    #[test]
    fn defaults_and_optional_marks_work_outside_a_json_body_too() {
        let mut s = state();
        s.variables = vec![var("have", "yes")];
        s.url = "http://h/{{missing|dflt}}?a={{have?}}&b={{nope?}}&c={{c|x y}}".into();
        s.headers_text = "X-Opt: {{nope?}}\nX-Def: {{nope|fallback}}".into();
        let r = build(&s, "");
        assert_eq!(r.url, "http://h/dflt?a=yes&b=&c=x%20y");
        assert!(r.headers.contains(&("X-Def".into(), "fallback".into())));
        assert!(r.headers.iter().any(|(k, v)| k == "X-Opt" && v.is_empty()));
    }

    #[test]
    fn the_names_a_request_needs_leave_out_defaults_and_marks() {
        let mut s = state();
        s.url = "http://h/{{a|d}}/{{b?}}/{{c}}".into();
        s.body_mode = BodyMode::Json;
        s.json_body = r#"{"x": "{{a}}", "y": {{d?}}}"#.into();
        assert_eq!(crate::engine::variables_used(&s), vec!["a", "b", "c", "d"]);
    }

    /// Values that have broken a request somewhere: line breaks, quotes, backslashes, separators of the
    /// context they land in, control characters and non-ASCII text.
    const NASTY: [&str; 12] = [
        "line one\nline two",
        "say \"hi\"",
        "C:\\temp\\new",
        "a&b=c",
        "a?b#c",
        "a b",
        "tab\there",
        "bell\u{7}",
        "caf\u{e9} \u{65e5}\u{672c}",
        "{{x}}",
        "50%",
        "",
    ];

    fn built_with(value: &str, setup: impl FnOnce(&mut PersistedState)) -> Result<OutgoingRequest, String> {
        let mut s = state();
        s.method = "POST".into();
        s.variables = vec![var("v", value)];
        setup(&mut s);
        build_request(&s, "")
    }

    #[test]
    fn a_value_never_changes_the_shape_of_the_url() {
        for value in NASTY {
            // path: it cannot start a query or a fragment, whatever it holds
            let r = built_with(value, |s| s.url = "http://h/items/{{v}}/detail".into()).unwrap();
            let url = reqwest::Url::parse(&r.url).unwrap_or_else(|e| panic!("{value:?} -> {} ({e})", r.url));
            assert!(url.query().is_none() && url.fragment().is_none(), "{value:?} -> {}", r.url);
            assert!(url.path().ends_with("/detail"), "{value:?} -> {}", r.url);
            // the same with the whole origin in a variable
            let r = built_with(value, |s| {
                s.variables.push(var("base", "http://h:8080"));
                s.url = "{{base}}/items/{{v}}".into();
            })
            .unwrap();
            let url = reqwest::Url::parse(&r.url).unwrap_or_else(|e| panic!("{value:?} -> {} ({e})", r.url));
            assert_eq!((url.host_str(), url.port(), url.query()), (Some("h"), Some(8080), None), "{value:?} -> {}", r.url);
            // query: the value is one value, in one parameter
            let r = built_with(value, |s| s.url = "http://h/s?a=1&q={{v}}&z=2".into()).unwrap();
            let url = reqwest::Url::parse(&r.url).unwrap();
            let pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert_eq!(pairs.len(), 3, "{value:?} -> {}", r.url);
            assert_eq!(pairs[1], ("q".to_string(), value.to_string()), "{value:?}");
        }
    }

    #[test]
    fn a_value_never_becomes_another_header() {
        // a line break would start a new header line (header injection)
        let err = built_with("ok\nX-Evil: 1", |s| s.headers_text = "X-Trace: {{v}}\nAccept: */*".into()).err().unwrap();
        assert!(err.contains("X-Trace") && err.contains("line break"), "{err}");
        // every other awkward value is a single header value, and the next header is untouched
        for value in NASTY.iter().filter(|v| !v.contains('\n')) {
            let r = built_with(value, |s| s.headers_text = "X-Trace: {{v}}\nAccept: */*".into()).unwrap();
            assert!(r.headers.iter().any(|(k, v)| k == "Accept" && v == "*/*"), "{value:?}");
            assert!(r.headers.iter().filter(|(k, _)| k.starts_with("X-")).count() == 1, "{value:?}: {:?}", r.headers);
        }
    }

    #[test]
    fn a_value_is_one_field_value_in_a_form_body() {
        for value in NASTY {
            let r = built_with(value, |s| {
                s.body_mode = BodyMode::UrlEncoded;
                s.urlencoded_body = "a=1\nname={{v}}\nz=2".into();
            })
            .unwrap();
            let body = text_body(&r).unwrap();
            let fields: Vec<(String, String)> = reqwest::Url::parse(&format!("http://h/?{body}"))
                .unwrap()
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            assert_eq!(fields.len(), 3, "{value:?} -> {body}");
            assert_eq!(fields[1], ("name".to_string(), value.to_string()), "{value:?} -> {body}");
        }
    }

    #[test]
    fn a_value_is_kept_exactly_in_json_raw_and_multipart_bodies() {
        for value in NASTY {
            let (_, parsed) = json_sent(r#"{"v": "{{v}}"}"#, value);
            assert_eq!(parsed["v"], value, "json {value:?}");
            let r = built_with(value, |s| {
                s.body_mode = BodyMode::Raw;
                s.raw_body = "<{{v}}>".into();
            })
            .unwrap();
            assert_eq!(text_body(&r), Some(format!("<{value}>").as_str()), "raw {value:?}");
            let r = built_with(value, |s| {
                s.body_mode = BodyMode::Multipart;
                s.multipart_fields = vec![FormField { key: "f".into(), kind: FieldKind::Text, value: "{{v}}".into(), enabled: true }];
            })
            .unwrap();
            match &r.body {
                OutgoingBody::Multipart(fields) => assert_eq!(fields[0].value, value, "multipart {value:?}"),
                _ => panic!("not a multipart body"),
            }
        }
    }

    #[test]
    fn a_raw_body_that_says_it_is_json_is_filled_in_as_json() {
        // how an agent sends a placeholder outside a string: `body` with a JSON content type
        let mut s = state();
        s.method = "POST".into();
        s.variables = vec![var("v", "two\nlines"), var("l", r#"["a","b"]"#)];
        s.headers_text = "Content-Type: application/json".into();
        s.body_mode = BodyMode::Raw;
        s.raw_body = r#"{"d": "{{v}}", "labels": {{l?}}, "epic": "{{epic?}}"}"#.into();
        let r = build(&s, "");
        let parsed: serde_json::Value = serde_json::from_str(text_body(&r).unwrap()).unwrap();
        assert_eq!(parsed, serde_json::json!({"d": "two\nlines", "labels": ["a", "b"]}));
        // without the content type it stays plain text
        s.headers_text = "Content-Type: text/plain".into();
        s.raw_body = "{{v}}".into();
        assert_eq!(text_body(&build(&s, "")), Some("two\nlines"));
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
    fn a_url_with_no_host_is_refused_not_sent_to_the_first_path_segment() {
        let mut s = state();
        for bad in ["https:///AphiaRecordsByAphiaIDs", "http:///x?y=1", "https://?q=1", "https://user@/path", "https:///"] {
            s.url = bad.into();
            let err = build_request(&s, "").err().unwrap_or_else(|| panic!("{bad} was accepted"));
            assert!(err.starts_with("Invalid URL") && err.contains("no host"), "{bad}: {err}");
        }
        for good in ["https://example.com/a", "http://localhost:3000", "https://user:pw@example.com/x", "http://[::1]:8080/"] {
            s.url = good.into();
            assert!(build_request(&s, "").is_ok(), "{good} should be accepted");
        }
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


    #[test]
    fn an_invalid_method_is_refused_before_anything_is_sent() {
        let s = PersistedState { url: "http://h/x".into(), method: "GE T".into(), ..Default::default() };
        assert!(build_request(&s, "").err().unwrap().starts_with("Invalid HTTP method"));
    }

    #[test]
    fn a_header_with_a_line_break_is_refused() {
        let (k, evil, fine) = ("X-A".to_string(), "b\r\nX-Evil: 1".to_string(), "b".to_string());
        assert!(check_header_lines([(&k, &evil)]).unwrap_err().contains("line break"));
        assert!(check_header_lines([(&k, &fine)]).is_ok());
    }
}
