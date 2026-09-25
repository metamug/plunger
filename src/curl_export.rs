//! Turns a request back into a curl command, for handing off to CI or any tool
//! that only speaks curl. `{{variables}}` are left as placeholders, never
//! resolved, so an exported command can't carry a secret.

use crate::model::{BodyMode, FieldKind, PersistedState};
use crate::request::parse_headers;

/// POSIX-shell single quoting: `it's` -> `'it'\''s'`.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn to_curl(state: &PersistedState) -> String {
    let mut parts: Vec<String> = vec!["curl".to_string()];
    let method = state.method.trim().to_ascii_uppercase();
    let headers = parse_headers(&state.headers_text);
    let has_header = |name: &str| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));

    let mut body_args: Vec<String> = Vec::new();
    match state.body_mode {
        BodyMode::None => {}
        BodyMode::Json => {
            if !has_header("content-type") {
                body_args.push(format!("-H {}", quote("Content-Type: application/json")));
            }
            body_args.push(format!("--data-raw {}", quote(&state.json_body)));
        }
        BodyMode::Raw => body_args.push(format!("--data-raw {}", quote(&state.raw_body))),
        BodyMode::UrlEncoded => {
            for line in state.urlencoded_body.lines().map(str::trim).filter(|l| !l.is_empty()) {
                body_args.push(format!("--data-raw {}", quote(line)));
            }
        }
        BodyMode::Multipart => {
            for f in state.multipart_fields.iter().filter(|f| f.enabled && !f.key.trim().is_empty()) {
                let value = match f.kind {
                    FieldKind::Text => format!("{}={}", f.key.trim(), f.value),
                    FieldKind::File => format!("{}=@{}", f.key.trim(), f.value),
                };
                body_args.push(format!("-F {}", quote(&value)));
            }
        }
    }

    // curl picks GET, or POST once there is a body; say so only when it differs.
    let implied = if body_args.is_empty() { "GET" } else { "POST" };
    if method != implied {
        parts.push(format!("-X {method}"));
    }
    parts.push(quote(state.url.trim()));
    for (k, v) in &headers {
        parts.push(format!("-H {}", quote(&format!("{k}: {v}"))));
    }
    parts.extend(body_args);
    if state.follow_redirects {
        parts.push("-L".to_string());
    }
    if state.insecure_tls {
        parts.push("--insecure".to_string());
    }
    parts.join(" \\\n  ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curl_import::parse_curl;
    use crate::model::FormField;

    #[test]
    fn a_json_post_round_trips_through_the_curl_importer() {
        let state = PersistedState {
            method: "POST".into(),
            url: "{{base}}/users?q=it's".into(),
            headers_text: "Authorization: Bearer {{token}}\nAccept: */*".into(),
            body_mode: BodyMode::Json,
            json_body: "{\"name\": \"O'Brien\"}".into(),
            follow_redirects: false,
            ..Default::default()
        };
        let curl = to_curl(&state);
        assert!(!curl.contains("-X POST"), "POST is implied by the body: {curl}");
        assert!(curl.contains("{{token}}"), "placeholders are kept: {curl}");

        let parsed = parse_curl(&curl).unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.url, "{{base}}/users?q=it's");
        assert!(parsed.headers.contains(&("Authorization".into(), "Bearer {{token}}".into())));
        assert_eq!(parsed.body.as_deref(), Some("{\"name\": \"O'Brien\"}"));
    }

    #[test]
    fn methods_forms_and_options_are_spelled_out() {
        let get = PersistedState { url: "https://h/x".into(), follow_redirects: true, ..Default::default() };
        assert_eq!(to_curl(&get), "curl \\\n  'https://h/x' \\\n  -L");

        let delete = PersistedState { method: "DELETE".into(), url: "https://h/1".into(), insecure_tls: true, follow_redirects: false, ..Default::default() };
        assert!(to_curl(&delete).contains("-X DELETE") && to_curl(&delete).contains("--insecure"));

        let upload = PersistedState {
            method: "POST".into(),
            url: "https://h/up".into(),
            body_mode: BodyMode::Multipart,
            multipart_fields: vec![
                FormField { key: "title".into(), kind: FieldKind::Text, value: "Hi".into(), enabled: true },
                FormField { key: "doc".into(), kind: FieldKind::File, value: "C:/a b.pdf".into(), enabled: true },
                FormField { key: "off".into(), kind: FieldKind::Text, value: "x".into(), enabled: false },
            ],
            follow_redirects: false,
            ..Default::default()
        };
        let curl = to_curl(&upload);
        assert!(curl.contains("-F 'title=Hi'") && curl.contains("-F 'doc=@C:/a b.pdf'") && !curl.contains("off="));
        let parsed = parse_curl(&curl).unwrap();
        assert_eq!(parsed.form_fields.len(), 2);
    }
}
