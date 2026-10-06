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
use crate::redact::is_sensitive_header;
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
            (Some(v), _, _) => Body::Json(serde_json::to_string(v).map_err(|e| e.to_string())?),
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
            let headers = self.headers.iter().flatten().map(|(k, v)| (k.clone(), v.clone())).collect();
            let spec = RequestSpec { body, headers, ..Default::default() }.to_state(session);
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form_fields: Vec<String>,
    /// {{variables}} the request uses.
    pub variables_used: Vec<String>,
    /// Set when saved under `save_as`; it now shows in Plunger's Saved list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_id: Option<i64>,
}

/// A request in the shape an agent reads: what it sends, and which `{{variables}}` it needs.
fn imported_from_state(state: &PersistedState, saved_id: Option<i64>) -> ImportedRequest {
    let body = match state.body_mode {
        BodyMode::Json => Some(state.json_body.clone()),
        BodyMode::Raw => Some(state.raw_body.clone()),
        BodyMode::UrlEncoded => Some(state.urlencoded_body.clone()),
        _ => None,
    };
    ImportedRequest {
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
        variables_used: engine::variables_used(state),
        saved_id,
    }
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
    Ok(imported_from_state(&state, saved_id))
}

/// Saves the request described by `params` (nothing is sent) under `name`, keeping its
/// `{{placeholders}}`. An existing name is refused unless `overwrite` is set, which replaces it.
pub fn save_request(name: &str, overwrite: bool, params: &SendParams, source: Source) -> Result<ImportedRequest, String> {
    let session = Session::load();
    let history = open_history()?;
    let state = params.to_state(&session, &history)?;
    let id = save_request_in(&history, name, overwrite, &state, source)?;
    Ok(imported_from_state(&state, Some(id)))
}

fn save_request_in(history: &History, name: &str, overwrite: bool, state: &PersistedState, source: Source) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the request a name.".into());
    }
    let existing = history.list_saved().map_err(|e| e.to_string())?.into_iter().find(|e| e.name.as_deref() == Some(name));
    match existing {
        Some(entry) if overwrite => {
            history.update_request(entry.id, state).map_err(|e| e.to_string())?;
            Ok(entry.id)
        }
        Some(_) => Err(format!("A saved request named \"{name}\" already exists. Pass overwrite: true to replace it, or choose another name.")),
        None => history.save_new_from(state, name, source).map_err(|e| format!("Couldn't save the request: {e}")),
    }
}

/// One saved request in full: method, URL, headers, body and the variables it needs.
pub fn show_saved_request(name: &str) -> Result<ImportedRequest, String> {
    let history = open_history()?;
    let entry = engine::find_saved(&history, name)?;
    Ok(imported_from_state(&entry.to_persisted_state(), Some(entry.id)))
}

/// Removes a saved request (its history entries stay) and returns what was removed.
pub fn delete_saved_request(name: &str) -> Result<ImportedRequest, String> {
    let history = open_history()?;
    delete_saved_in(&history, name)
}

fn delete_saved_in(history: &History, name: &str) -> Result<ImportedRequest, String> {
    let entry = engine::find_saved(history, name)?;
    history.set_name(entry.id, None).map_err(|e| e.to_string())?;
    Ok(imported_from_state(&entry.to_persisted_state(), Some(entry.id)))
}

pub fn list_saved_requests() -> Result<Vec<StoredRequestInfo>, String> {
    let saved = open_history()?.list_saved().map_err(|e| e.to_string())?;
    Ok(saved.iter().map(StoredRequestInfo::from).collect())
}

pub fn get_history(limit: Option<i64>, search: Option<&str>) -> Result<Vec<HistoryItem>, String> {
    let limit = limit.unwrap_or(HISTORY_DEFAULT).clamp(1, HISTORY_MAX);
    let rows = open_history()?.search_recent(search.unwrap_or(""), limit).map_err(|e| e.to_string())?;
    Ok(rows.iter().map(HistoryItem::from).collect())
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct VariablesResult {
    pub variables: Vec<VariableInfo>,
    /// Always available: {{$uuid}}, {{$timestamp}}, {{$randomInt}}, and {{$env:NAME}} (an environment variable, read when the request is sent).
    pub built_in: Vec<String>,
    /// True when a Bearer token is saved in Plunger (send with use_saved_bearer). Its value is never shown.
    pub saved_bearer_available: bool,
    /// Problems reading remembered secrets, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
}

pub fn list_variables() -> VariablesResult {
    let session = Session::load();
    VariablesResult {
        variables: session.variables(),
        built_in: vec!["$uuid".into(), "$timestamp".into(), "$randomInt".into(), "$env:NAME".into()],
        saved_bearer_available: !session.bearer.is_empty(),
        problems: session.problems,
    }
}

const MAX_VARIABLES: usize = 200;
const MAX_VARIABLE_BYTES: usize = 64 * 1024;
const MAX_NAME_LEN: usize = 64;

fn valid_variable_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && !name.starts_with('$')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(name)
    } else {
        Err(format!(
            "`{name}` isn't a valid variable name: use letters, digits, _ - and . (at most {MAX_NAME_LEN} characters, not starting with $)."
        ))
    }
}

/// Sets a variable that later requests can use as `{{name}}`. It is kept between runs and shared
/// with the window, the command line and the MCP server. A secret (or a name that looks like a
/// credential) is kept in the system credential store, never in a file, and is masked in results.
pub fn set_variable(name: &str, value: &str, secret: Option<bool>, source: Source) -> Result<VariableInfo, String> {
    let session = Session::load();
    let history = open_history()?;
    let window: Vec<String> = session.window_variables().iter().map(|v| v.name.trim().to_string()).collect();
    set_variable_in(&history, &crate::secrets::OsStore::new(), &window, name, value, secret, source)
}

fn set_variable_in(
    history: &History,
    store: &dyn crate::secrets::SecretStore,
    window_names: &[String],
    name: &str,
    value: &str,
    secret: Option<bool>,
    source: Source,
) -> Result<VariableInfo, String> {
    let name = valid_variable_name(name)?;
    if value.len() > MAX_VARIABLE_BYTES {
        return Err(format!("The value is too long ({} bytes; the limit is {MAX_VARIABLE_BYTES}).", value.len()));
    }
    if window_names.iter().any(|n| n == name) {
        return Err(format!(
            "`{name}` is defined by the user in the Plunger window, and an agent cannot change it. Pick another name, or pass a value for one request in `variables`."
        ));
    }
    let existing = history.list_agent_variables().map_err(|e| e.to_string())?;
    if existing.len() >= MAX_VARIABLES && !existing.iter().any(|v| v.name == name) {
        return Err(format!("There are already {MAX_VARIABLES} variables set by agents; delete some first."));
    }
    let secret = secret.unwrap_or(false) || is_sensitive_header(name);
    if secret {
        store
            .set(&engine::agent_secret_key(name), value)
            .map_err(|e| format!("Couldn't keep the secret in the system credential store: {e}"))?;
        history.set_agent_variable(name, "", true, source).map_err(|e| e.to_string())?;
    } else {
        // A plain value replacing a secret of the same name must not leave the old secret behind.
        let _ = store.delete(&engine::agent_secret_key(name));
        history.set_agent_variable(name, value, false, source).map_err(|e| e.to_string())?;
    }
    Ok(VariableInfo {
        name: name.to_string(),
        secret,
        has_value: !value.is_empty(),
        remembered: secret,
        value: (!secret).then(|| value.to_string()),
        source: "agent".to_string(),
    })
}

/// Removes a variable an agent set. The user's own variables can only be removed in the window.
pub fn delete_variable(name: &str) -> Result<(), String> {
    let session = Session::load();
    let history = open_history()?;
    let window: Vec<String> = session.window_variables().iter().map(|v| v.name.trim().to_string()).collect();
    delete_variable_in(&history, &crate::secrets::OsStore::new(), &window, name)
}

fn delete_variable_in(
    history: &History,
    store: &dyn crate::secrets::SecretStore,
    window_names: &[String],
    name: &str,
) -> Result<(), String> {
    let name = name.trim();
    if window_names.iter().any(|n| n == name) {
        return Err(format!("`{name}` belongs to the user (it is defined in the Plunger window); delete it there."));
    }
    if !history.delete_agent_variable(name).map_err(|e| e.to_string())? {
        return Err(format!("No variable named `{name}` was set by an agent."));
    }
    let _ = store.delete(&engine::agent_secret_key(name));
    Ok(())
}

/// Removes every variable agents set; returns how many.
pub fn clear_variables() -> Result<usize, String> {
    let history = open_history()?;
    let store = crate::secrets::OsStore::new();
    let names = history.clear_agent_variables().map_err(|e| e.to_string())?;
    for name in &names {
        let _ = crate::secrets::SecretStore::delete(&store, &engine::agent_secret_key(name));
    }
    Ok(names.len())
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
    fn agents_set_and_delete_variables_and_a_secret_never_reaches_the_database() {
        use crate::secrets::test_support::MemoryStore;
        let h = History::in_memory();
        let store = MemoryStore::default();

        let plain = set_variable_in(&h, &store, &[], "base", "http://h", None, Source::Mcp).unwrap();
        assert_eq!((plain.value.as_deref(), plain.secret, plain.source.as_str()), (Some("http://h"), false, "agent"));

        // A credential-looking name is always a secret; its value goes to the credential store only.
        let secret = set_variable_in(&h, &store, &[], "api_token", "s3cret-value", None, Source::Cli).unwrap();
        assert!(secret.secret && secret.value.is_none() && secret.remembered);
        assert!(h.list_agent_variables().unwrap().iter().all(|v| !v.value.contains("s3cret")));
        assert_eq!(store.data.borrow()["agentvar:api_token"], "s3cret-value");

        // The next session sees both, with the secret value restored for sending.
        let session = Session::load_with(std::path::Path::new("Z:/nope/app.ron"), &store, Some(&h));
        let by_name = |n: &str| session.state.variables.iter().find(|v| v.name == n).unwrap();
        assert_eq!(by_name("base").value, "http://h");
        assert_eq!(by_name("api_token").value, "s3cret-value");
        let listed = session.variables();
        assert!(listed.iter().all(|v| v.source == "agent"));
        assert!(listed.iter().find(|v| v.name == "api_token").unwrap().value.is_none());

        // Updating works; a plain value replacing a secret forgets the secret.
        set_variable_in(&h, &store, &[], "base", "http://other", None, Source::Cli).unwrap();
        set_variable_in(&h, &store, &[], "mode", "x", Some(true), Source::Cli).unwrap();
        set_variable_in(&h, &store, &[], "mode", "y", Some(false), Source::Cli).unwrap();
        assert!(!store.data.borrow().contains_key("agentvar:mode"));

        delete_variable_in(&h, &store, &[], "api_token").unwrap();
        assert!(!store.data.borrow().contains_key("agentvar:api_token"));
        assert!(delete_variable_in(&h, &store, &[], "api_token").unwrap_err().contains("No variable"));
    }

    #[test]
    fn the_users_own_variables_are_protected_and_bad_names_are_refused() {
        use crate::secrets::test_support::MemoryStore;
        let h = History::in_memory();
        let store = MemoryStore::default();
        let window = vec!["host".to_string()];
        assert!(set_variable_in(&h, &store, &window, "host", "evil", None, Source::Mcp).unwrap_err().contains("defined by the user"));
        assert!(delete_variable_in(&h, &store, &window, "host").unwrap_err().contains("belongs to the user"));
        for bad in ["", "a b", "$uuid", "a/b", &"x".repeat(65)] {
            assert!(set_variable_in(&h, &store, &[], bad, "v", None, Source::Mcp).is_err(), "{bad:?}");
        }
        assert!(set_variable_in(&h, &store, &[], "big", &"x".repeat(MAX_VARIABLE_BYTES + 1), None, Source::Mcp).is_err());
        for n in 0..MAX_VARIABLES {
            set_variable_in(&h, &store, &[], &format!("v{n}"), "1", None, Source::Mcp).unwrap();
        }
        assert!(set_variable_in(&h, &store, &[], "one-too-many", "1", None, Source::Mcp).is_err());
        assert!(set_variable_in(&h, &store, &[], "v0", "2", None, Source::Mcp).is_ok(), "updating an existing one is fine at the limit");
    }

    #[test]
    fn agent_variables_load_when_there_is_no_window_state_file() {
        use crate::secrets::test_support::MemoryStore;
        let h = History::in_memory();
        h.set_agent_variable("host", "agent-value", false, Source::Mcp).unwrap();
        let dir = std::env::temp_dir().join(format!("plunger-winvar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // no window state file: only the agent's exists
        let session = Session::load_with(&dir.join("app.ron"), &MemoryStore::default(), Some(&h));
        assert_eq!(session.state.variables.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_agent_can_save_overwrite_show_and_delete_a_request_and_placeholders_survive() {
        let h = History::in_memory();
        let params = SendParams {
            url: Some("{{base}}/orders".into()),
            method: Some("POST".into()),
            headers: Some(BTreeMap::from([("Authorization".into(), "Bearer {{token}}".into())])),
            json: Some(serde_json::json!({"qty": 2})),
            ..Default::default()
        };
        let session = Session { state: PersistedState::default(), bearer: String::new(), problems: vec![], agent_variables: Default::default() };
        let state = params.to_state(&session, &h).unwrap();

        let id = save_request_in(&h, "create order", false, &state, Source::Mcp).unwrap();
        let err = save_request_in(&h, "create order", false, &state, Source::Mcp).unwrap_err();
        assert!(err.contains("already exists") && err.contains("overwrite"), "{err}");

        // fix the saved request: same id, new body, placeholders intact
        let fixed = SendParams { json: Some(serde_json::json!({"qty": 3})), ..params };
        let state = fixed.to_state(&session, &h).unwrap();
        assert_eq!(save_request_in(&h, "create order", true, &state, Source::Mcp).unwrap(), id);
        let shown = imported_from_state(&engine::find_saved(&h, "create order").unwrap().to_persisted_state(), Some(id));
        assert_eq!(shown.body.as_deref(), Some("{\"qty\":3}"));
        assert_eq!(shown.variables_used, vec!["base", "token"]);
        assert!(shown.headers.iter().any(|h| h.name == "Authorization" && h.value == "Bearer {{token}}"));

        assert_eq!(delete_saved_in(&h, "create order").unwrap().saved_id, Some(id));
        assert!(h.list_saved().unwrap().is_empty());
        assert!(delete_saved_in(&h, "create order").is_err());
        assert!(save_request_in(&h, "  ", false, &state, Source::Mcp).is_err());
    }

    #[test]
    fn one_body_kind_at_a_time() {
        let p = SendParams { json: Some(serde_json::json!({"a": 1})), body: Some("x".into()), ..Default::default() };
        assert!(p.body().is_err());
        let p = SendParams { form: Some(BTreeMap::from([("a".into(), "1".into())])), ..Default::default() };
        assert_eq!(p.body().unwrap(), Body::Form(vec![("a".into(), "1".into())]));
    }

    #[test]
    fn a_json_object_from_an_agent_is_sent_compact() {
        let p = SendParams { json: Some(serde_json::json!({"a": 1, "b": [1, 2]})), ..Default::default() };
        assert_eq!(p.body().unwrap(), Body::Json("{\"a\":1,\"b\":[1,2]}".into()));
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
        let session = Session { state: PersistedState::default(), bearer: String::new(), problems: vec![], agent_variables: Default::default() };
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
        assert!(state.body_mode == BodyMode::Json && state.json_body.contains("\"n\":1"));
    }

    #[test]
    fn a_url_or_saved_name_is_required() {
        let h = History::in_memory();
        let session = Session { state: PersistedState::default(), bearer: String::new(), problems: vec![], agent_variables: Default::default() };
        assert!(SendParams::default().to_state(&session, &h).is_err());
    }
}
