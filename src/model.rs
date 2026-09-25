use crate::redact::{is_sensitive_header, is_sensitive_param};
use serde::{Deserialize, Serialize};

#[derive(PartialEq, Clone, Copy, Serialize, Deserialize, Default, Debug)]
pub enum BodyMode {
    #[default]
    None,
    Json,
    UrlEncoded,
    Multipart,
    Raw,
}

#[derive(PartialEq, Clone, Copy)]
pub enum RequestTab {
    Params,
    Headers,
    Body,
    Variables,
    Options,
}

#[derive(PartialEq, Clone, Copy)]
pub enum ResponseTab {
    Body,
    Headers,
}

fn enabled_by_default() -> bool {
    true
}

/// A query parameter row.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

impl KeyValue {
    pub fn blank() -> Self {
        Self {
            key: String::new(),
            value: String::new(),
            enabled: true,
        }
    }

    pub fn is_blank(&self) -> bool {
        self.key.is_empty() && self.value.is_empty()
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
pub enum FieldKind {
    #[default]
    Text,
    File,
}

/// One part of a multipart/form-data body. For `File`, `value` is a path.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct FormField {
    pub key: String,
    pub kind: FieldKind,
    pub value: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

impl FormField {
    pub fn blank() -> Self {
        Self {
            key: String::new(),
            kind: FieldKind::Text,
            value: String::new(),
            enabled: true,
        }
    }

    pub fn is_blank(&self) -> bool {
        self.key.is_empty() && self.value.is_empty()
    }
}

/// A `{{name}}` variable. Secret values never go into the saved state file;
/// with `remember` they are kept in the OS credential store instead.
#[derive(Serialize, Deserialize, Clone, PartialEq, Default, Debug)]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub secret: bool,
    /// Keep the (secret) value in the OS credential store between runs.
    #[serde(default)]
    pub remember: bool,
}

impl Variable {
    /// Explicitly marked, or named like a credential (`token`, `password`...).
    pub fn is_secret(&self) -> bool {
        self.secret || is_sensitive_header(&self.name)
    }

    pub fn is_blank(&self) -> bool {
        self.name.is_empty() && self.value.is_empty()
    }
}

/// The subset of app state worth remembering between runs — request config,
/// not transient things like "is a request in flight right now."
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct PersistedState {
    pub method: String,
    pub url: String,
    pub params: Vec<KeyValue>,
    pub headers_text: String,
    pub body_mode: BodyMode,
    pub json_body: String,
    pub urlencoded_body: String,
    pub multipart_fields: Vec<FormField>,
    pub raw_body: String,
    pub variables: Vec<Variable>,
    pub remember_bearer: bool,
    pub timeout_secs: u64,
    pub follow_redirects: bool,
    pub insecure_tls: bool,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            method: "GET".to_string(),
            url: "https://jsonplaceholder.typicode.com/todos/1".to_string(),
            params: Vec::new(),
            headers_text: String::new(),
            body_mode: BodyMode::None,
            json_body: String::from("{\n  \"key\": \"value\"\n}"),
            urlencoded_body: String::new(),
            multipart_fields: Vec::new(),
            raw_body: String::new(),
            variables: Vec::new(),
            remember_bearer: false,
            timeout_secs: 20,
            follow_redirects: true,
            insecure_tls: false,
        }
    }
}

impl PersistedState {
    /// Session-wide settings (timeout, redirects, TLS, variables) belong to the
    /// session, not to a request, so loading a history entry must not change them.
    pub fn with_session_from(mut self, current: &PersistedState) -> Self {
        self.timeout_secs = current.timeout_secs;
        self.follow_redirects = current.follow_redirects;
        self.insecure_tls = current.insecure_tls;
        self.variables = current.variables.clone();
        self.remember_bearer = current.remember_bearer;
        self
    }

    /// A copy that is safe to write to disk: credential values are blanked.
    /// The Bearer field is never part of the state at all.
    pub fn redacted(&self) -> PersistedState {
        let mut s = self.clone();
        // The URL carries the query params too, so it needs the same treatment.
        s.url = crate::redact::redact_url(&s.url);
        s.headers_text = crate::redact::redact_headers_text(&s.headers_text);
        for p in &mut s.params {
            if is_sensitive_param(&p.key) {
                p.value.clear();
            }
        }
        for f in &mut s.multipart_fields {
            if f.kind == FieldKind::Text && is_sensitive_param(&f.key) {
                f.value.clear();
            }
        }
        for v in &mut s.variables {
            if v.is_secret() {
                v.value.clear();
            }
        }
        s
    }
}

pub type SendResult = Result<ResponseData, String>;

/// What the response area should show. One enum instead of two `Option`s that
/// were only ever meant to be set one at a time.
#[derive(Default)]
pub enum Outcome {
    #[default]
    Empty,
    Response(ResponseData),
    Failed(String),
}

pub struct ResponseData {
    pub status: u16,
    pub status_text: String,
    pub elapsed_ms: u128,
    pub size_bytes: usize,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub json_value: Option<serde_json::Value>,
    /// True when the body was cut at the read cap; `total_size` is the
    /// server-reported length when it sent one.
    pub truncated: bool,
    pub total_size: Option<u64>,
}

/// A parsed request — the common output shape for both curl and HAR import,
/// so the UI only needs one "apply this" code path regardless of source.
pub struct ParsedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub form_fields: Vec<FormField>,
}

impl ParsedRequest {
    /// The form an imported request becomes: a JSON body when the body parses
    /// as JSON, raw text otherwise, form-data for `-F` fields. Query params are
    /// left in the URL (the Params table is rebuilt from it).
    pub fn into_state(self) -> PersistedState {
        let mut state = PersistedState {
            method: self.method,
            url: self.url,
            params: Vec::new(),
            headers_text: crate::request::headers_to_text(&self.headers),
            ..Default::default()
        };
        match self.body {
            Some(body) if serde_json::from_str::<serde_json::Value>(&body).is_ok() => {
                state.body_mode = BodyMode::Json;
                state.json_body = body;
            }
            Some(body) => {
                state.body_mode = BodyMode::Raw;
                state.raw_body = body;
            }
            None if !self.form_fields.is_empty() => state.body_mode = BodyMode::Multipart,
            None => state.body_mode = BodyMode::None,
        }
        state.multipart_fields = self.form_fields;
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_blanks_credentials_but_keeps_everything_else() {
        let s = PersistedState {
            headers_text: "Accept: */*\nAuthorization: Bearer X".into(),
            params: vec![
                KeyValue { key: "page".into(), value: "2".into(), enabled: true },
                KeyValue { key: "api_key".into(), value: "SECRET".into(), enabled: true },
            ],
            multipart_fields: vec![
                FormField { key: "password".into(), kind: FieldKind::Text, value: "pw".into(), enabled: true },
                FormField { key: "avatar".into(), kind: FieldKind::File, value: "C:/a.png".into(), enabled: true },
            ],
            variables: vec![
                Variable { name: "host".into(), value: "localhost".into(), secret: false, remember: false },
                Variable { name: "authToken".into(), value: "T".into(), secret: false, remember: false },
                Variable { name: "custom".into(), value: "V".into(), secret: true, remember: false },
            ],
            ..Default::default()
        };
        let r = s.redacted();
        assert_eq!(r.headers_text, "Accept: */*\nAuthorization:");
        assert_eq!(r.params[0].value, "2");
        assert_eq!(r.params[1].value, "");
        assert_eq!(r.multipart_fields[0].value, "");
        assert_eq!(r.multipart_fields[1].value, "C:/a.png");
        assert_eq!(r.variables[0].value, "localhost");
        assert_eq!(r.variables[1].value, "");
        assert_eq!(r.variables[2].value, "");
        // the original is untouched
        assert_eq!(s.variables[1].value, "T");
    }

    #[test]
    fn old_saved_state_without_new_fields_still_loads() {
        let json = r#"{"method":"POST","url":"http://a","body_mode":"Json"}"#;
        let s: PersistedState = serde_json::from_str(json).unwrap();
        assert_eq!(s.method, "POST");
        assert!(s.params.is_empty() && s.variables.is_empty() && s.multipart_fields.is_empty());
        assert_eq!(s.timeout_secs, 20);
    }

    #[test]
    fn session_settings_survive_loading_a_request() {
        let current = PersistedState {
            insecure_tls: true,
            timeout_secs: 99,
            variables: vec![Variable { name: "a".into(), value: "1".into(), secret: false, remember: false }],
            ..Default::default()
        };
        let loaded = PersistedState { url: "http://other".into(), ..Default::default() }.with_session_from(&current);
        assert_eq!(loaded.url, "http://other");
        assert!(loaded.insecure_tls);
        assert_eq!(loaded.timeout_secs, 99);
        assert_eq!(loaded.variables.len(), 1);
    }
}
