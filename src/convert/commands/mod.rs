//! A request written out as, and read back from, a command someone can paste into a shell:
//! curl for bash, curl for the Windows command prompt, and PowerShell. `{{variables}}` stay as
//! placeholders when exporting, so an exported command never carries a secret.

mod cmd;
mod powershell;

use crate::domain::model::{BodyMode, FieldKind, ParsedRequest, PersistedState};
use crate::domain::request::parse_headers;

/// A command syntax Plunger can write and read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    CurlBash,
    CurlCmd,
    PowerShellRest,
    PowerShellWeb,
}

impl Dialect {
    pub const ALL: [Dialect; 4] = [Dialect::CurlBash, Dialect::CurlCmd, Dialect::PowerShellRest, Dialect::PowerShellWeb];

    pub fn label(self) -> &'static str {
        match self {
            Dialect::CurlBash => "curl (bash)",
            Dialect::CurlCmd => "curl (Windows cmd)",
            Dialect::PowerShellRest => "PowerShell (Invoke-RestMethod)",
            Dialect::PowerShellWeb => "PowerShell (Invoke-WebRequest)",
        }
    }
}

/// What a request carries, independent of any dialect.
pub(crate) struct Parts {
    pub method: String,
    pub url: String,
    /// Includes the Content-Type the body implies, when the user did not set one.
    pub headers: Vec<(String, String)>,
    pub body: PartBody,
    pub follow_redirects: bool,
    pub insecure_tls: bool,
}

pub(crate) enum PartBody {
    None,
    Text(String),
    /// Already joined with `&`.
    Form(String),
    /// (name, is a file, value or path)
    Multipart(Vec<(String, bool, String)>),
}

impl Parts {
    pub fn from_state(state: &PersistedState) -> Self {
        let mut headers = parse_headers(&state.headers_text);
        let has_type = |headers: &[(String, String)]| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
        let body = match state.body_mode {
            BodyMode::None => PartBody::None,
            BodyMode::Json => {
                if !has_type(&headers) {
                    headers.push(("Content-Type".into(), "application/json".into()));
                }
                PartBody::Text(state.json_body.clone())
            }
            BodyMode::Raw => PartBody::Text(state.raw_body.clone()),
            BodyMode::UrlEncoded => {
                // curl and PowerShell both send this Content-Type on their own.
                let lines: Vec<&str> = state.urlencoded_body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
                PartBody::Form(lines.join("&"))
            }
            BodyMode::Multipart => PartBody::Multipart(
                state
                    .multipart_fields
                    .iter()
                    .filter(|f| f.enabled && !f.key.trim().is_empty())
                    .map(|f| (f.key.trim().to_string(), f.kind == FieldKind::File, f.value.clone()))
                    .collect(),
            ),
        };
        Parts {
            method: state.method.trim().to_ascii_uppercase(),
            url: state.url.trim().to_string(),
            headers,
            body,
            follow_redirects: state.follow_redirects,
            insecure_tls: state.insecure_tls,
        }
    }

    pub fn has_body(&self) -> bool {
        !matches!(self.body, PartBody::None)
    }
}

/// The request as a command in `dialect`.
pub fn export(state: &PersistedState, dialect: Dialect) -> String {
    match dialect {
        Dialect::CurlBash => crate::convert::curl_export::to_curl(state),
        Dialect::CurlCmd => cmd::export(&Parts::from_state(state)),
        Dialect::PowerShellRest => powershell::export(&Parts::from_state(state), "Invoke-RestMethod"),
        Dialect::PowerShellWeb => powershell::export(&Parts::from_state(state), "Invoke-WebRequest"),
    }
}

/// Which dialect a pasted command is written in.
pub fn detect(input: &str) -> Dialect {
    let t = input.trim_start().to_ascii_lowercase();
    let first = t.split_whitespace().next().unwrap_or("");
    if first.starts_with('$') || ["invoke-restmethod", "invoke-webrequest", "irm", "iwr"].contains(&first) || t.contains("invoke-webrequest") || t.contains("invoke-restmethod") {
        return if t.contains("invoke-restmethod") || first == "irm" { Dialect::PowerShellRest } else { Dialect::PowerShellWeb };
    }
    if cmd::looks_like_cmd(input) {
        Dialect::CurlCmd
    } else {
        Dialect::CurlBash
    }
}

/// Reads a pasted command in any supported dialect into a request.
pub fn import(input: &str) -> Result<ParsedRequest, String> {
    match detect(input) {
        Dialect::CurlBash => crate::convert::curl_import::parse_curl(input),
        Dialect::CurlCmd => crate::convert::curl_import::parse_curl(&cmd::normalize(input)),
        Dialect::PowerShellRest | Dialect::PowerShellWeb => powershell::parse(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::FormField;

    fn state(method: &str, url: &str, headers: &str, mode: BodyMode, body: &str) -> PersistedState {
        let mut s = PersistedState { method: method.into(), url: url.into(), headers_text: headers.into(), body_mode: mode, ..Default::default() };
        match mode {
            BodyMode::Json => s.json_body = body.into(),
            BodyMode::Raw => s.raw_body = body.into(),
            BodyMode::UrlEncoded => s.urlencoded_body = body.into(),
            _ => {}
        }
        s
    }

    fn sample_states() -> Vec<PersistedState> {
        let mut multipart = state("POST", "https://h/upload", "", BodyMode::Multipart, "");
        multipart.multipart_fields = vec![
            FormField { key: "title".into(), kind: FieldKind::Text, value: "my doc".into(), enabled: true },
            FormField { key: "file".into(), kind: FieldKind::File, value: "C:/data/a.txt".into(), enabled: true },
        ];
        vec![
            state("GET", "https://api.example.com/items?page=2&q=a%20b", "Accept: application/json", BodyMode::None, ""),
            state("POST", "https://h/orders", "Authorization: Bearer {{token}}\nX-Trace: it's \"quoted\"", BodyMode::Json, r#"{"name":"widget","tags":["a","b"],"note":"say \"hi\" it's ok"}"#),
            state("PUT", "https://h/items/1", "Content-Type: text/plain", BodyMode::Raw, "plain text, with 'quotes' and \"doubles\""),
            state("DELETE", "https://h/items/1", "", BodyMode::None, ""),
            state("POST", "https://h/form", "", BodyMode::UrlEncoded, "a=1\nb=two%20words"),
            multipart,
        ]
    }

    type Essentials = (String, String, Vec<(String, String)>, Option<String>, Vec<(String, String, String)>);

    /// What must survive a trip through a dialect: method, URL, headers, and the body.
    fn essentials(r: &ParsedRequest) -> Essentials {
        let mut headers: Vec<(String, String)> = r.headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.clone())).collect();
        headers.sort();
        let fields = r.form_fields.iter().map(|f| (f.key.clone(), format!("{:?}", f.kind), f.value.clone())).collect();
        let body = r.body.as_ref().map(|b| match serde_json::from_str::<serde_json::Value>(b) {
            Ok(v) => v.to_string(),
            Err(_) => b.clone(),
        });
        (r.method.clone(), r.url.clone(), headers, body, fields)
    }

    #[test]
    fn every_dialect_round_trips_through_its_own_importer() {
        for dialect in Dialect::ALL {
            for s in sample_states() {
                let text = export(&s, dialect);
                let back = import(&text).unwrap_or_else(|e| panic!("{dialect:?}: {e}\n{text}"));
                let original = crate::convert::curl_import::parse_curl(&export(&s, Dialect::CurlBash)).unwrap();
                let (m1, u1, h1, b1, f1) = essentials(&original);
                let (m2, u2, mut h2, b2, f2) = essentials(&back);
                // PowerShell moves the Content-Type into -ContentType; it must still be there.
                h2.sort();
                assert_eq!((m1, u1, b1, f1), (m2, u2, b2, f2), "{dialect:?}\n{text}");
                assert_eq!(h1, h2, "{dialect:?} headers\n{text}");
            }
        }
    }

    #[test]
    fn exports_never_resolve_variables() {
        let s = state("GET", "https://h/{{id}}", "Authorization: Bearer {{token}}", BodyMode::None, "");
        for dialect in Dialect::ALL {
            let text = export(&s, dialect);
            assert!(text.contains("{{id}}") && text.contains("{{token}}"), "{dialect:?}: {text}");
        }
    }

    #[test]
    fn the_dialect_of_a_pasted_command_is_detected() {
        assert_eq!(detect("curl 'https://h' -H 'A: b'"), Dialect::CurlBash);
        assert_eq!(detect("curl \"https://h\" ^\n  -H \"A: b\""), Dialect::CurlCmd);
        assert_eq!(detect("curl \"https://h\" --data-raw ^\"^{^\\^\"a^\\^\":1^}^\""), Dialect::CurlCmd);
        assert_eq!(detect("Invoke-RestMethod -Uri 'https://h'"), Dialect::PowerShellRest);
        assert_eq!(detect("  $session = New-Object x\nInvoke-WebRequest -Uri \"https://h\""), Dialect::PowerShellWeb);
        assert_eq!(detect("irm https://h"), Dialect::PowerShellRest);
    }
}
