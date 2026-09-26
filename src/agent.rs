//! The operations an agent can run, shared by the command line and the MCP
//! server so both behave identically. Each returns a serializable result or
//! a message saying what went wrong; neither ever contains a secret value.

use crate::curl_export::to_curl;
use crate::curl_import::parse_curl;
use crate::engine::{
    self, AgentResponse, Body, HistoryItem, NameValue, RequestSpec, Scrubber, SendError, Session, StoredRequestInfo,
    VariableInfo, DEFAULT_MAX_BODY_CHARS,
};
use crate::history::{History, Source};
use crate::model::{BodyMode, FieldKind, PersistedState};
use crate::request::{headers_to_text, parse_headers};
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const HISTORY_DEFAULT: i64 = 20;
const HISTORY_MAX: i64 = 200;

fn open_history() -> Result<History, String> {
    History::open().map_err(|e| format!("Couldn't open Plunger's history database: {e}"))
}

#[derive(Deserialize, Serialize, JsonSchema, Default, Debug, Clone)]
pub struct SendParams {
    /// Send a saved request by name (see list_saved_requests). Any other field given here overrides that part of it.
    #[serde(default)]
    pub saved_request: Option<String>,
    /// GET, POST, PUT, PATCH, DELETE, HEAD or OPTIONS. Defaults to GET.
    #[serde(default)]
    pub method: Option<String>,
    /// Full URL. May use {{variables}}, e.g. "{{base}}/users/{{id}}". Required unless saved_request is given.
    #[serde(default)]
    pub url: Option<String>,
    /// Request headers, e.g. {"Accept": "application/json", "Authorization": "Bearer {{token}}"}.
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
    /// A JSON body (any JSON value). Sent with Content-Type: application/json.
    #[serde(default)]
    pub json: Option<serde_json::Value>,
    /// A raw text body, sent as-is. Use instead of `json` for non-JSON payloads.
    #[serde(default)]
    pub body: Option<String>,
    /// Form fields, sent as application/x-www-form-urlencoded.
    #[serde(default)]
    pub form: Option<BTreeMap<String, String>>,
    /// Extra or overriding {{variable}} values for this request only. The user's own variables (including secrets) are already available by name; you don't need their values.
    #[serde(default)]
    pub variables: Option<BTreeMap<String, String>>,
    /// Attach the Bearer token the user saved in Plunger. Off by default: only turn it on for a host the user asked you to call with their credentials.
    #[serde(default)]
    pub use_saved_bearer: Option<bool>,
    /// Seconds to wait for a response (1-600). Defaults to the user's setting (20).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    /// Skip TLS certificate checks, e.g. for a local server with a self-signed certificate.
    #[serde(default)]
    pub insecure_tls: Option<bool>,
    /// Longest body to return, in characters (default 50000). Longer bodies are cut and marked.
    #[serde(default)]
    pub max_body_chars: Option<usize>,
}

/// Merges `extra` headers into `headers_text`, replacing same-named ones.
fn merge_headers(headers_text: &str, extra: &BTreeMap<String, String>) -> String {
    let mut rows: Vec<(String, String)> = parse_headers(headers_text)
        .into_iter()
        .filter(|(k, _)| !extra.keys().any(|e| e.eq_ignore_ascii_case(k)))
        .collect();
    rows.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    headers_to_text(&rows)
}

impl SendParams {
    fn body(&self) -> Result<Body, String> {
        let given = [self.json.is_some(), self.body.is_some(), self.form.is_some()].iter().filter(|b| **b).count();
        if given > 1 {
            return Err("Give only one of `json`, `body` or `form`.".into());
        }
        Ok(match (&self.json, &self.body, &self.form) {
            (Some(v), _, _) => Body::Json(serde_json::to_string_pretty(v).map_err(|e| e.to_string())?),
            (_, Some(text), _) => Body::Text(text.clone()),
            (_, _, Some(form)) => Body::Form(form.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            _ => Body::None,
        })
    }

    fn variables(&self) -> Vec<(String, String)> {
        self.variables.iter().flatten().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// The form state to send: a saved request with overrides, or a new one.
    fn to_state(&self, session: &Session, history: &History) -> Result<PersistedState, String> {
        let body = self.body()?;
        if let Some(h) = &self.headers {
            crate::request::check_header_lines(h.iter())?;
        }
        let Some(name) = &self.saved_request else {
            let url = self.url.clone().filter(|u| !u.trim().is_empty()).ok_or("Give a `url`, or a `saved_request` name.")?;
            let spec = RequestSpec {
                method: self.method.clone(),
                url,
                headers: self.headers.iter().flatten().map(|(k, v)| (k.clone(), v.clone())).collect(),
                body,
                variables: self.variables(),
                timeout_secs: self.timeout_secs,
                follow_redirects: self.follow_redirects,
                insecure_tls: self.insecure_tls,
            };
            return Ok(spec.to_state(session));
        };

        let (_, mut state) = engine::saved_request_state(history, name, session)?;
        if let Some(m) = &self.method {
            state.method = m.trim().to_ascii_uppercase();
        }
        if let Some(u) = self.url.as_ref().filter(|u| !u.trim().is_empty()) {
            state.url = u.trim().to_string();
        }
        if let Some(h) = &self.headers {
            state.headers_text = merge_headers(&state.headers_text, h);
        }
        if body != Body::None {
            // Reuse RequestSpec's body mapping so both paths agree.
            let spec = RequestSpec { body, ..Default::default() }.to_state(session);
            state.body_mode = spec.body_mode;
            state.json_body = spec.json_body;
            state.raw_body = spec.raw_body;
            state.urlencoded_body = spec.urlencoded_body;
        }
        engine::apply_overrides(&mut state, &self.variables(), self.timeout_secs, self.follow_redirects, self.insecure_tls);
        Ok(state)
    }
}

/// Why `send_request` produced no response. Messages never contain a secret.
#[derive(Debug, PartialEq)]
pub enum SendFailure {
    /// Nothing went out: bad input, or an undefined {{variable}}.
    NotSent(String),
    /// Sent, but no response came back (refused, timed out, DNS...).
    Failed(String),
}

impl std::fmt::Display for SendFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendFailure::NotSent(m) => write!(f, "Not sent: {m}"),
            SendFailure::Failed(m) => write!(f, "Request failed: {m}"),
        }
    }
}

/// Sends a request exactly like the window's Ctrl+Enter and records it in the
/// shared history. The result never contains a secret value.
pub fn send_request(params: &SendParams, source: Source) -> Result<AgentResponse, SendFailure> {
    let session = Session::load();
    let history = open_history().map_err(SendFailure::NotSent)?;
    let state = params.to_state(&session, &history).map_err(SendFailure::NotSent)?;
    send_prepared(state, params, &session, &history, source)
}

/// Sends a curl command (parsed, never run as a program); `params` supplies
/// only variables and options.
pub fn send_curl(curl: &str, params: &SendParams, source: Source) -> Result<AgentResponse, SendFailure> {
    let session = Session::load();
    let history = open_history().map_err(SendFailure::NotSent)?;
    let mut state = state_from_curl(curl).map_err(SendFailure::NotSent)?.with_session_from(&session.state);
    engine::apply_overrides(&mut state, &params.variables(), params.timeout_secs, params.follow_redirects, params.insecure_tls);
    send_prepared(state, params, &session, &history, source)
}

fn send_prepared(
    state: PersistedState,
    params: &SendParams,
    session: &Session,
    history: &History,
    source: Source,
) -> Result<AgentResponse, SendFailure> {
    // Mask every secret the session knows, used in this request or not.
    let scrubber = Scrubber::new(&state, &session.bearer);
    let bearer = if params.use_saved_bearer.unwrap_or(false) { session.bearer.as_str() } else { "" };
    let max_body_chars = params.max_body_chars.unwrap_or(DEFAULT_MAX_BODY_CHARS);
    match engine::send(state, bearer, Some(history), source) {
        Ok(sent) => Ok(AgentResponse::from_sent(&sent, &scrubber, max_body_chars)),
        Err(SendError::Refused(msg)) => {
            // The window's advice ("define it in the Variables tab") isn't
            // something an agent can do; say how it can supply one instead.
            let hint = if msg.starts_with("Undefined variable") {
                " An agent can pass a value for this request: `variables` in MCP, `--var name=value` on the command line."
            } else {
                ""
            };
            Err(SendFailure::NotSent(format!("{}{hint}", scrubber.text(&msg))))
        }
        Err(SendError::Failed { message, history_id }) => {
            let recorded = history_id.map(|id| format!(" (recorded in history as #{id})")).unwrap_or_default();
            Err(SendFailure::Failed(format!("{}{recorded}", scrubber.text(&message))))
        }
    }
}

/// The form state for a curl command, exactly as the window's import builds it.
fn state_from_curl(curl: &str) -> Result<PersistedState, String> {
    Ok(parse_curl(curl)?.into_state())
}

/// A curl command parsed into a Plunger request.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ImportedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<NameValue>,
    /// none, json, form-data or raw.
    pub body_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// multipart -F fields: "name=value" or "name=@file".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub form_fields: Vec<String>,
    /// {{variables}} the request uses.
    pub variables_used: Vec<String>,
    /// Set when saved under `save_as`; it now shows in Plunger's Saved list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_id: Option<i64>,
}

/// Parses a curl command (without sending it), optionally saving it by name.
pub fn import_curl(curl: &str, save_as: Option<&str>, source: Source) -> Result<ImportedRequest, String> {
    let state = state_from_curl(curl)?;
    let saved_id = match save_as.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => Some(
            open_history()?
                .save_new_from(&state, name, source)
                .map_err(|e| format!("Couldn't save the request: {e}"))?,
        ),
        None => None,
    };
    let body = match state.body_mode {
        BodyMode::Json => Some(state.json_body.clone()),
        BodyMode::Raw => Some(state.raw_body.clone()),
        _ => None,
    };
    Ok(ImportedRequest {
        method: state.method.clone(),
        url: state.url.clone(),
        headers: parse_headers(&state.headers_text).into_iter().map(|(name, value)| NameValue { name, value }).collect(),
        body_type: engine::body_type_name(state.body_mode).to_string(),
        body,
        form_fields: state
            .multipart_fields
            .iter()
            .map(|f| match f.kind {
                FieldKind::Text => format!("{}={}", f.key, f.value),
                FieldKind::File => format!("{}=@{}", f.key, f.value),
            })
            .collect(),
        variables_used: engine::variables_used(&state),
        saved_id,
    })
}

pub fn list_saved_requests() -> Result<Vec<StoredRequestInfo>, String> {
    let saved = open_history()?.list_saved().map_err(|e| e.to_string())?;
    Ok(saved.iter().map(StoredRequestInfo::from).collect())
}

pub fn get_history(limit: Option<i64>) -> Result<Vec<HistoryItem>, String> {
    let limit = limit.unwrap_or(HISTORY_DEFAULT).clamp(1, HISTORY_MAX);
    let rows = open_history()?.list_recent(limit).map_err(|e| e.to_string())?;
    Ok(rows.iter().map(HistoryItem::from).collect())
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct VariablesResult {
    pub variables: Vec<VariableInfo>,
    /// Always available: {{$uuid}}, {{$timestamp}}, {{$randomInt}}.
    pub built_in: Vec<String>,
    /// True when a Bearer token is saved in Plunger (send with use_saved_bearer). Its value is never shown.
    pub saved_bearer_available: bool,
    /// Problems reading remembered secrets, if any.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
}

pub fn list_variables() -> VariablesResult {
    let session = Session::load();
    VariablesResult {
        variables: session.variables(),
        built_in: vec!["$uuid".into(), "$timestamp".into(), "$randomInt".into()],
        saved_bearer_available: !session.bearer.is_empty(),
        problems: session.problems,
    }
}

/// The curl command for a saved request (by name) or a history row (by id).
/// Placeholders stay placeholders, so no secret is written out.
pub fn export_curl(saved_request: Option<&str>, history_id: Option<i64>) -> Result<String, String> {
    let history = open_history()?;
    let entry = match (saved_request, history_id) {
        (Some(name), None) => engine::find_saved(&history, name)?,
        (None, Some(id)) => history
            .get(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("No request #{id} in the history or saved requests."))?,
        _ => return Err("Give exactly one of `saved_request` or `history_id`.".into()),
    };
    Ok(to_curl(&entry.to_persisted_state()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_body_kind_at_a_time() {
        let p = SendParams { json: Some(serde_json::json!({"a": 1})), body: Some("x".into()), ..Default::default() };
        assert!(p.body().is_err());
        let p = SendParams { form: Some(BTreeMap::from([("a".into(), "1".into())])), ..Default::default() };
        assert_eq!(p.body().unwrap(), Body::Form(vec![("a".into(), "1".into())]));
    }

    #[test]
    fn a_saved_request_takes_overrides() {
        let h = History::in_memory();
        let saved = PersistedState {
            method: "GET".into(),
            url: "https://h/users".into(),
            headers_text: "Accept: text/plain\nX-Keep: 1".into(),
            ..Default::default()
        };
        h.save_new(&saved, "Users").unwrap();
        let session = Session { state: PersistedState::default(), bearer: String::new(), problems: vec![] };
        let p = SendParams {
            saved_request: Some("users".into()),
            headers: Some(BTreeMap::from([("accept".into(), "application/json".into())])),
            json: Some(serde_json::json!({"n": 1})),
            method: Some("post".into()),
            ..Default::default()
        };
        let state = p.to_state(&session, &h).unwrap();
        assert_eq!(state.method, "POST");
        assert_eq!(state.url, "https://h/users");
        assert_eq!(state.headers_text, "X-Keep: 1\naccept: application/json");
        assert!(state.body_mode == BodyMode::Json && state.json_body.contains("\"n\": 1"));
    }

    #[test]
    fn a_url_or_saved_name_is_required() {
        let h = History::in_memory();
        let session = Session { state: PersistedState::default(), bearer: String::new(), problems: vec![] };
        assert!(SendParams::default().to_state(&session, &h).is_err());
    }
}
