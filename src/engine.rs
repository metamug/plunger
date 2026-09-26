//! The request engine for agents: what the command line and the MCP server
//! call. It sends through exactly the same path as the window's Ctrl+Enter
//! (`prepare_to_send` then `http::execute`), so an undefined `{{variable}}` is
//! refused the same way, and every send lands in the same history.
//!
//! On top of that it shapes results for an agent: structured fields, a body
//! cut to a size a model can read, and no secret value anywhere in the output.

use crate::history::{app_data_dir, History, HistoryEntry, Source};
use crate::http;
use crate::model::{BodyMode, PersistedState, ResponseData, Variable};
use crate::redact::{is_sensitive_header, redact_url};
use crate::request::{headers_to_text, parse_headers, prepare_to_send};
use crate::secrets::{OsStore, SecretStore, SecretSync};
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Default cap on body text handed to an agent, in characters. Big enough for
/// typical API responses, small enough not to flood a model's context.
pub const DEFAULT_MAX_BODY_CHARS: usize = 50_000;
/// Secret values shorter than this are not masked in output: masking "1" or
/// "ab" would mangle unrelated text.
const MIN_MASKED_LEN: usize = 4;

/// What the window has configured and an agent inherits: variables, options,
/// and the remembered secrets from the credential store.
pub struct Session {
    pub state: PersistedState,
    pub bearer: String,
    /// Non-fatal problems, e.g. the credential store couldn't be read.
    pub problems: Vec<String>,
}

impl Session {
    /// Reads the window's saved state (variables and options) and fills in
    /// remembered secrets. The window writes its state when it closes and
    /// about every 30 seconds, so a variable edited a moment ago may lag.
    pub fn load() -> Self {
        Self::load_with(&app_data_dir().join("app.ron"), &OsStore::new())
    }

    pub fn load_with(state_file: &Path, store: &dyn SecretStore) -> Self {
        let mut state = read_window_state(state_file).unwrap_or_default();
        let mut bearer = String::new();
        let problems = SecretSync::default().restore(store, &mut state, &mut bearer);
        Self { state, bearer, problems }
    }

    /// Variables as an agent may see them: names always, values only when not secret.
    pub fn variables(&self) -> Vec<VariableInfo> {
        self.state
            .variables
            .iter()
            .filter(|v| !v.name.trim().is_empty())
            .map(|v| VariableInfo {
                name: v.name.trim().to_string(),
                secret: v.is_secret(),
                has_value: !v.value.is_empty(),
                remembered: v.remember && v.is_secret(),
                value: (!v.is_secret()).then(|| v.value.clone()),
            })
            .collect()
    }
}

/// eframe's state file is a RON map of key -> RON string; the request form
/// (with the variables) is under eframe's `APP_KEY`.
fn read_window_state(path: &Path) -> Option<PersistedState> {
    let text = std::fs::read_to_string(path).ok()?;
    let map: BTreeMap<String, String> = ron::from_str(&text).ok()?;
    ron::from_str(map.get(eframe::APP_KEY)?).ok()
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct VariableInfo {
    pub name: String,
    /// Secret values are masked in the window and never returned here.
    pub secret: bool,
    pub has_value: bool,
    /// Kept in the operating system's credential store between runs.
    pub remembered: bool,
    /// Only for non-secret variables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// A request described by an agent. Everything but the URL is optional.
#[derive(Default, Debug, Clone)]
pub struct RequestSpec {
    pub method: Option<String>,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
    /// Added to (or overriding) the window's variables for this request only.
    pub variables: Vec<(String, String)>,
    pub timeout_secs: Option<u64>,
    pub follow_redirects: Option<bool>,
    pub insecure_tls: Option<bool>,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub enum Body {
    #[default]
    None,
    Json(String),
    Text(String),
    /// application/x-www-form-urlencoded pairs.
    Form(Vec<(String, String)>),
}

impl RequestSpec {
    /// The form state the window would have for this request, with the
    /// session's variables and options underneath.
    pub fn to_state(&self, session: &Session) -> PersistedState {
        let mut state = PersistedState {
            method: self.method.clone().unwrap_or_else(|| "GET".into()).trim().to_ascii_uppercase(),
            url: self.url.trim().to_string(),
            params: Vec::new(),
            headers_text: headers_to_text(&self.headers),
            ..Default::default()
        }
        .with_session_from(&session.state);
        match &self.body {
            Body::None => state.body_mode = BodyMode::None,
            Body::Json(text) => {
                state.body_mode = BodyMode::Json;
                state.json_body = text.clone();
            }
            Body::Text(text) => {
                state.body_mode = BodyMode::Raw;
                state.raw_body = text.clone();
            }
            Body::Form(pairs) => {
                state.body_mode = BodyMode::UrlEncoded;
                state.urlencoded_body = pairs
                    .iter()
                    .map(|(k, v)| format!("{}={}", encode_keeping_variables(k), encode_keeping_variables(v)))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
        }
        apply_overrides(&mut state, &self.variables, self.timeout_secs, self.follow_redirects, self.insecure_tls);
        state
    }
}

/// Percent-encodes a form key or value, leaving `{{variables}}` for the send
/// step to fill in, so a value like `a&b` can't split the field.
fn encode_keeping_variables(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else { break };
        out.push_str(&crate::query::encode_value(&rest[..start]));
        out.push_str(&rest[start..start + len + 2]);
        rest = &rest[start + len + 2..];
    }
    out.push_str(&crate::query::encode_value(rest));
    out
}

/// Adds or replaces variables and options for one request.
pub fn apply_overrides(
    state: &mut PersistedState,
    variables: &[(String, String)],
    timeout_secs: Option<u64>,
    follow_redirects: Option<bool>,
    insecure_tls: Option<bool>,
) {
    for (name, value) in variables {
        let name = name.trim();
        match state.variables.iter_mut().find(|v| v.name.trim() == name) {
            Some(v) => v.value = value.clone(),
            None => state.variables.push(Variable {
                name: name.to_string(),
                value: value.clone(),
                secret: false,
                remember: false,
            }),
        }
    }
    if let Some(t) = timeout_secs {
        state.timeout_secs = t;
    }
    if let Some(f) = follow_redirects {
        state.follow_redirects = f;
    }
    if let Some(i) = insecure_tls {
        state.insecure_tls = i;
    }
}

/// A saved request, ready to send with the session's variables and options.
pub fn saved_request_state(history: &History, name: &str, session: &Session) -> Result<(HistoryEntry, PersistedState), String> {
    let entry = find_saved(history, name)?;
    let state = entry.to_persisted_state().with_session_from(&session.state);
    Ok((entry, state))
}

/// Finds a saved request by name: exact first, then case-insensitive.
pub fn find_saved(history: &History, name: &str) -> Result<HistoryEntry, String> {
    let saved = history.list_saved().map_err(|e| format!("Couldn't read saved requests: {e}"))?;
    let wanted = name.trim();
    if let Some(e) = saved.iter().find(|e| e.name.as_deref() == Some(wanted)) {
        return Ok(e.clone());
    }
    let matches: Vec<&HistoryEntry> = saved
        .iter()
        .filter(|e| e.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(wanted)))
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => {
            let names: Vec<&str> = saved.iter().filter_map(|e| e.name.as_deref()).collect();
            Err(if names.is_empty() {
                format!("No saved request named \"{wanted}\". There are no saved requests yet.")
            } else {
                format!("No saved request named \"{wanted}\". Saved requests: {}.", names.join(", "))
            })
        }
        _ => Err(format!("More than one saved request is named \"{wanted}\" (ignoring case); use the exact name.")),
    }
}

/// Why a request didn't produce a response.
#[derive(Debug)]
pub enum SendError {
    /// Not sent at all: an undefined `{{variable}}`, a missing or invalid URL...
    Refused(String),
    /// Sent (and recorded in history) but no response came back.
    Failed { message: String, history_id: Option<i64> },
}

pub struct Sent {
    pub response: ResponseData,
    pub history_id: Option<i64>,
    /// What was actually sent (URL with its scheme filled in, etc.).
    pub state: PersistedState,
}

/// Sends exactly like the window's Ctrl+Enter and records it in the shared
/// history, tagged with `source`.
pub fn send(mut state: PersistedState, bearer: &str, history: Option<&History>, source: Source) -> Result<Sent, SendError> {
    let request = prepare_to_send(&mut state, bearer).map_err(SendError::Refused)?;
    let result = http::execute(request);
    let history_id = history.and_then(|h| {
        let (status, elapsed) = match &result {
            Ok(r) => (Some(r.status), Some(r.elapsed_ms)),
            Err(_) => (None, None),
        };
        h.insert_from(&state, status, elapsed, source).ok()
    });
    match result {
        Ok(response) => Ok(Sent { response, history_id, state }),
        Err(message) => Err(SendError::Failed { message, history_id }),
    }
}

/// Percent-encodes every byte except the RFC 3986 unreserved characters.
fn percent_encode_all(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Replaces every known secret value in text with `[redacted:<name>]`, so a
/// server that echoes a token back (many do) can't hand it to the agent.
pub struct Scrubber {
    secrets: Vec<(String, String)>,
}

impl Scrubber {
    /// Secrets are the secret variables' values and the Bearer token.
    pub fn new(state: &PersistedState, bearer: &str) -> Self {
        let mut secrets: Vec<(String, String)> = state
            .variables
            .iter()
            .filter(|v| v.is_secret() && v.value.trim().len() >= MIN_MASKED_LEN)
            .map(|v| (v.name.trim().to_string(), v.value.trim().to_string()))
            .collect();
        if bearer.trim().len() >= MIN_MASKED_LEN {
            secrets.push(("bearer".to_string(), bearer.trim().to_string()));
        }
        // Servers often echo values URL-encoded (a query string, a form body),
        // so mask those spellings too.
        let encoded: Vec<(String, String)> = secrets
            .iter()
            .flat_map(|(name, value)| {
                [crate::query::encode_value(value), percent_encode_all(value)]
                    .into_iter()
                    .filter(move |e| e != value)
                    .map(move |e| (name.clone(), e))
            })
            .collect();
        secrets.extend(encoded);
        secrets.dedup();
        // Longest first, so a secret that contains another is masked whole.
        secrets.sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
        Self { secrets }
    }

    pub fn text(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (name, value) in &self.secrets {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), &format!("[redacted:{name}]"));
            }
        }
        out
    }

    pub fn json(&self, value: &serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match value {
            Value::String(s) => Value::String(self.text(s)),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.json(v)).collect()),
            Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (self.text(k), self.json(v))).collect()),
            other => other.clone(),
        }
    }

    /// Names of the secrets that were found (and masked) in `text`.
    fn found_in(&self, text: &str) -> Vec<String> {
        self.secrets.iter().filter(|(_, v)| text.contains(v.as_str())).map(|(n, _)| n.clone()).collect()
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct NameValue {
    pub name: String,
    pub value: String,
}

/// A response as returned to an agent.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct AgentResponse {
    /// True for a 2xx status.
    pub ok: bool,
    pub status: u16,
    pub status_text: String,
    pub elapsed_ms: u64,
    /// Bytes of body received.
    pub size_bytes: usize,
    /// The body is valid JSON, parsed. Absent for non-JSON bodies, or when the
    /// JSON is longer than the size limit (then it is in `body`, cut short).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<serde_json::Value>,
    /// The body as text, when it is not JSON or is too long to return whole.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The body was longer than the limit and has been cut; this is its full length in characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_cut_from_chars: Option<usize>,
    /// The response exceeded Plunger's 10 MB read limit; only the first 10 MB were read.
    pub truncated_at_10mb: bool,
    pub headers: Vec<NameValue>,
    pub request: SentRequest,
    /// Secrets whose values appeared in the response and were replaced with
    /// `[redacted:<name>]`. Names only, never values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub redacted: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct SentRequest {
    pub method: String,
    /// With credential-looking query values blanked.
    pub url: String,
    /// Row in the shared history, visible in the Plunger window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_id: Option<i64>,
}

impl AgentResponse {
    pub fn from_sent(sent: &Sent, scrubber: &Scrubber, max_body_chars: usize) -> Self {
        let r = &sent.response;
        let mut redacted = scrubber.found_in(&r.body);
        for (_, v) in &r.headers {
            redacted.extend(scrubber.found_in(v));
        }
        redacted.sort();
        redacted.dedup();

        let body_chars = r.body.chars().count();
        let (json, body, body_cut_from_chars) = match &r.json_value {
            Some(value) if body_chars <= max_body_chars => (Some(scrubber.json(value)), None, None),
            _ if body_chars <= max_body_chars => (None, Some(scrubber.text(&r.body)), None),
            _ => {
                // Mask before cutting: a cut through the middle of a secret
                // would leave half of it unrecognisable, and unmasked.
                let cut: String = scrubber.text(&r.body).chars().take(max_body_chars).collect();
                (None, Some(cut), Some(body_chars))
            }
        };
        let headers = r
            .headers
            .iter()
            .map(|(name, value)| NameValue {
                name: name.clone(),
                // Set-Cookie and friends carry credentials of their own.
                value: if is_sensitive_header(name) { "[redacted]".to_string() } else { scrubber.text(value) },
            })
            .collect();
        Self {
            ok: (200..300).contains(&r.status),
            status: r.status,
            status_text: r.status_text.clone(),
            elapsed_ms: r.elapsed_ms as u64,
            size_bytes: r.size_bytes,
            json,
            body,
            body_cut_from_chars,
            truncated_at_10mb: r.truncated,
            headers,
            request: SentRequest {
                method: sent.state.method.clone(),
                url: scrubber.text(&redact_url(&sent.state.url)),
                history_id: sent.history_id,
            },
            redacted,
        }
    }
}

/// A request as stored (saved or history), described for an agent.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct StoredRequestInfo {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub method: String,
    /// Credential-looking query values are blanked on disk.
    pub url: String,
    /// Credential headers are stored with their values blanked.
    pub headers: Vec<NameValue>,
    /// none, json, form-data, x-www-form-urlencoded or raw.
    pub body_type: String,
    /// `{{variables}}` this request needs. Built-ins like `{{$uuid}}` are filled in automatically.
    pub variables_used: Vec<String>,
}

/// One row of the shared history.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct HistoryItem {
    pub id: i64,
    /// RFC 3339, UTC.
    pub time: String,
    pub method: String,
    pub url: String,
    /// Absent when no response came back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<i64>,
    /// Set when the request is also saved under a name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Who sent it: gui (a person in the window), cli or mcp (an agent).
    pub source: String,
}

impl From<&HistoryEntry> for HistoryItem {
    fn from(e: &HistoryEntry) -> Self {
        Self {
            id: e.id,
            time: e.created_at.clone(),
            method: e.method.clone(),
            url: e.url.clone(),
            status: e.status,
            elapsed_ms: e.elapsed_ms,
            name: e.name.clone(),
            source: e.source.as_str().to_string(),
        }
    }
}

pub fn body_type_name(mode: BodyMode) -> &'static str {
    match mode {
        BodyMode::None => "none",
        BodyMode::Json => "json",
        BodyMode::Multipart => "form-data",
        BodyMode::UrlEncoded => "x-www-form-urlencoded",
        BodyMode::Raw => "raw",
    }
}

impl From<&HistoryEntry> for StoredRequestInfo {
    fn from(e: &HistoryEntry) -> Self {
        let state = e.to_persisted_state();
        Self {
            id: e.id,
            name: e.name.clone(),
            method: e.method.clone(),
            url: e.url.clone(),
            headers: parse_headers(&e.headers_text)
                .into_iter()
                .map(|(name, value)| NameValue { name, value })
                .collect(),
            body_type: body_type_name(e.body_mode).to_string(),
            variables_used: variables_used(&state),
        }
    }
}

/// `{{names}}` referenced anywhere in the request, without built-ins, in order of first use.
pub fn variables_used(state: &PersistedState) -> Vec<String> {
    let mut texts = vec![state.url.as_str(), state.headers_text.as_str()];
    match state.body_mode {
        BodyMode::Json => texts.push(&state.json_body),
        BodyMode::Raw => texts.push(&state.raw_body),
        BodyMode::UrlEncoded => texts.push(&state.urlencoded_body),
        BodyMode::Multipart | BodyMode::None => {}
    }
    let mut names: Vec<String> = Vec::new();
    for field in &state.multipart_fields {
        collect_names(&field.value, &mut names);
    }
    for text in texts {
        collect_names(text, &mut names);
    }
    names
}

fn collect_names(text: &str, names: &mut Vec<String>) {
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { return };
        let name = after[..end].trim();
        let valid = !name.is_empty()
            && !name.starts_with('$')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.');
        if valid && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        rest = &after[end + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::build_request;
    use crate::secrets::test_support::MemoryStore;
    use crate::test_server::serve_echo;

    fn session(vars: Vec<Variable>) -> Session {
        Session {
            state: PersistedState { variables: vars, ..Default::default() },
            bearer: String::new(),
            problems: Vec::new(),
        }
    }

    fn var(name: &str, value: &str, secret: bool) -> Variable {
        Variable { name: name.into(), value: value.into(), secret, remember: false }
    }

    #[test]
    fn an_undefined_variable_is_refused_and_nothing_is_recorded() {
        let history = History::in_memory();
        let s = session(vec![]);
        let spec = RequestSpec { url: "http://127.0.0.1:9/{{missing}}".into(), ..Default::default() };
        let err = send(spec.to_state(&s), "", Some(&history), Source::Mcp).err().unwrap();
        let SendError::Refused(message) = err else { panic!("expected a refusal") };
        assert!(message.contains("{{missing}}"), "{message}");
        assert!(history.list_recent(5).unwrap().is_empty());
    }

    #[test]
    fn a_secret_echoed_back_by_the_server_never_reaches_the_agent() {
        let base = serve_echo(1);
        let history = History::in_memory();
        let s = session(vec![var("apiToken", "SUPER-SECRET-VALUE", true), var("who", "ann", false)]);
        let spec = RequestSpec {
            url: format!("{base}/users?name={{{{who}}}}&token={{{{apiToken}}}}"),
            headers: vec![("Authorization".into(), "Bearer {{apiToken}}".into())],
            ..Default::default()
        };
        let sent = send(spec.to_state(&s), "", Some(&history), Source::Mcp).unwrap();
        let out = AgentResponse::from_sent(&sent, &Scrubber::new(&sent.state, ""), DEFAULT_MAX_BODY_CHARS);
        let json = serde_json::to_string(&out).unwrap();
        assert!(!json.contains("SUPER-SECRET-VALUE"), "{json}");
        assert!(json.contains("[redacted:apiToken]"));
        assert!(json.contains("name=ann"), "non-secret variables are used as-is");
        assert_eq!(out.redacted, vec!["apiToken".to_string()]);
        assert!(out.headers.iter().any(|h| h.name.eq_ignore_ascii_case("set-cookie") && h.value == "[redacted]"));

        // The shared history has it, tagged as sent by an agent, with the token blanked.
        let rows = history.list_recent(5).unwrap();
        assert_eq!(rows[0].source, Source::Mcp);
        assert!(!rows[0].url.contains("SUPER-SECRET-VALUE") && !rows[0].headers_text.contains("SUPER-SECRET-VALUE"));
        assert_eq!(out.request.history_id, Some(rows[0].id));
    }

    #[test]
    fn url_encoded_echoes_of_a_secret_are_masked_too() {
        let state = PersistedState { variables: vec![var("key", "a/b c+d&e", true)], ..Default::default() };
        let s = Scrubber::new(&state, "");
        for echoed in ["a/b c+d&e", "a/b%20c%2Bd%26e", "a%2Fb%20c%2Bd%26e"] {
            assert_eq!(s.text(&format!("x={echoed};")), "x=[redacted:key];", "{echoed}");
        }
    }

    #[test]
    fn a_cut_through_the_middle_of_a_secret_leaves_no_part_of_it() {
        let base = serve_echo(1);
        let s = session(vec![var("token", "ABCDEFGHIJKLMNOP", true)]);
        let spec = RequestSpec {
            method: Some("POST".into()),
            url: format!("{base}/x"),
            body: Body::Text("{{token}}".into()),
            ..Default::default()
        };
        let sent = send(spec.to_state(&s), "", None, Source::Cli).unwrap();
        let at = sent.response.body.find("ABCDEFGH").unwrap();
        // Cut so the limit lands inside the secret.
        let out = AgentResponse::from_sent(&sent, &Scrubber::new(&sent.state, ""), at + 5);
        let body = out.body.unwrap();
        assert!(!body.contains("ABCDE"), "{body}");
    }

    #[test]
    fn long_bodies_are_cut_for_the_agent_and_say_so() {
        let base = serve_echo(1);
        let s = session(vec![]);
        let spec = RequestSpec {
            method: Some("POST".into()),
            url: format!("{base}/big"),
            body: Body::Text("x".repeat(5_000)),
            ..Default::default()
        };
        let sent = send(spec.to_state(&s), "", None, Source::Cli).unwrap();
        let out = AgentResponse::from_sent(&sent, &Scrubber::new(&sent.state, ""), 1_000);
        assert!(out.json.is_none());
        assert_eq!(out.body.as_ref().unwrap().chars().count(), 1_000);
        assert!(out.body_cut_from_chars.unwrap() > 5_000);
    }

    #[test]
    fn request_variables_override_the_windows_and_options_apply() {
        let s = session(vec![var("host", "old", false)]);
        let spec = RequestSpec {
            url: "http://{{host}}/x".into(),
            variables: vec![("host".into(), "new".into()), ("extra".into(), "1".into())],
            timeout_secs: Some(3),
            insecure_tls: Some(true),
            body: Body::Form(vec![("a".into(), "1".into()), ("b".into(), "2".into())]),
            ..Default::default()
        };
        let state = spec.to_state(&s);
        let req = build_request(&state, "").unwrap();
        assert_eq!(req.url, "http://new/x");
        assert!(state.insecure_tls && state.timeout_secs == 3);
        assert_eq!(state.urlencoded_body, "a=1\nb=2");
    }

    #[test]
    fn the_windows_state_file_is_read_for_variables_and_secrets_come_from_the_store() {
        let dir = std::env::temp_dir().join(format!("plunger-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("app.ron");
        let state = PersistedState {
            variables: vec![
                Variable { name: "base".into(), value: "http://h".into(), secret: false, remember: false },
                Variable { name: "token".into(), value: String::new(), secret: true, remember: true },
            ],
            ..Default::default()
        };
        let mut map = BTreeMap::new();
        map.insert(eframe::APP_KEY.to_string(), ron::to_string(&state).unwrap());
        map.insert("egui".to_string(), "()".to_string());
        std::fs::write(&file, ron::to_string(&map).unwrap()).unwrap();

        let store = MemoryStore::default();
        store.data.borrow_mut().insert("var:token".into(), "T0KEN-VALUE".into());
        let s = Session::load_with(&file, &store);
        assert_eq!(s.state.variables[1].value, "T0KEN-VALUE");

        let listed = serde_json::to_string(&s.variables()).unwrap();
        assert!(listed.contains("http://h") && listed.contains("\"token\""));
        assert!(!listed.contains("T0KEN-VALUE"), "{listed}");
        assert!(s.variables()[1].remembered && s.variables()[1].has_value);
    }

    #[test]
    fn a_missing_state_file_is_just_an_empty_session() {
        let s = Session::load_with(Path::new("Z:/nope/app.ron"), &MemoryStore::default());
        assert!(s.state.variables.is_empty());
    }

    #[test]
    fn saved_requests_are_found_by_name_and_list_their_variables() {
        let h = History::in_memory();
        let st = PersistedState {
            url: "{{base}}/users/{{id}}?x={{$uuid}}".into(),
            headers_text: "Authorization: Bearer {{token}}".into(),
            ..Default::default()
        };
        h.save_new(&st, "Get user").unwrap();
        assert!(find_saved(&h, "get USER").is_ok());
        let err = find_saved(&h, "nope").err().unwrap();
        assert!(err.contains("Get user"), "{err}");
        let info = StoredRequestInfo::from(&find_saved(&h, "Get user").unwrap());
        assert_eq!(info.variables_used, vec!["base", "id"]);
    }

    #[test]
    fn form_values_are_encoded_but_variables_are_left_for_the_send_step() {
        assert_eq!(encode_keeping_variables("a&b c=d"), "a%26b%20c=d");
        assert_eq!(encode_keeping_variables("Bearer {{tok}}&x"), "Bearer%20{{tok}}%26x");
        assert_eq!(encode_keeping_variables("{{a}}{{b}}"), "{{a}}{{b}}");
        assert_eq!(encode_keeping_variables("open {{ never closed"), "open%20{{%20never%20closed");
    }
}
