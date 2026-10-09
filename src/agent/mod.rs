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
use crate::model::{BodyMode, FieldKind, PersistedState, ResponseData};
use crate::request::{headers_to_text, parse_headers};
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const HISTORY_DEFAULT: i64 = 20;
const HISTORY_MAX: i64 = 200;

pub(super) fn open_history() -> Result<History, String> {
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
    /// Keep values from the response as variables for later requests, e.g.
    /// [{"name": "token", "from": "json:$.access_token"}]. `from` is `json:$.path`, `header:Name` or `status`.
    /// A name like token or password is kept as a hidden secret.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extract: Vec<crate::workflow::Extract>,
    /// Return only these values instead of the whole body, e.g. ["$.data[0].id", "header:Location", "status"].
    /// Saves context on a big response; `json` and `body` are then left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Vec<String>>,
}

/// Merges `extra` headers into `headers_text`, replacing same-named ones.
pub(super) fn merge_headers(headers_text: &str, extra: &BTreeMap<String, String>) -> String {
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
    let (mut shaped, raw) = send_prepared_raw(state, params, &session, &history, source)?;
    for e in &params.extract {
        let stored = crate::workflow::extract::extract(&raw, &e.from).and_then(|value| set_variable(e.name.trim(), &value, e.secret, source).map(|info| (info, value)));
        match stored {
            Ok((info, value)) => {
                // The response that carried a new secret must not show it either.
                if info.secret {
                    if let Some(scrubber) = Scrubber::only(&info.name, &value) {
                        shaped.mask_with(&scrubber);
                    }
                }
                shaped.variables_set.push(info);
            }
            Err(err) => shaped.problems.push(format!("could not set `{}` from `{}`: {err}", e.name, e.from)),
        }
    }
    Ok(shaped)
}

/// Replaces `json` and `body` with just the values asked for, so a big response costs a few tokens.
/// Values come from the response as sent and are masked like the rest of the output.
fn apply_select(shaped: &mut AgentResponse, raw: &ResponseData, paths: &[String], scrubber: &Scrubber) {
    let mut selected = BTreeMap::new();
    for path in paths {
        match crate::workflow::extract::select(raw, path) {
            Ok(value) => {
                selected.insert(path.clone(), scrubber.json(&value));
            }
            Err(err) => shaped.problems.push(format!("could not select `{path}`: {err}")),
        }
    }
    shaped.selected = Some(selected);
    shaped.json = None;
    shaped.body = None;
    shaped.body_cut_from_chars = None;
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

pub(super) fn send_prepared(
    state: PersistedState,
    params: &SendParams,
    session: &Session,
    history: &History,
    source: Source,
) -> Result<AgentResponse, SendFailure> {
    send_prepared_raw(state, params, session, history, source).map(|(shaped, _)| shaped)
}

/// Like `send_request`, but also returns the response as the server sent it, so a workflow can take
/// a value out of it before any secret is masked. The raw response never goes to an agent.
pub fn send_request_raw(params: &SendParams, source: Source) -> Result<(AgentResponse, ResponseData), SendFailure> {
    let session = Session::load();
    let history = open_history().map_err(SendFailure::NotSent)?;
    let state = params.to_state(&session, &history).map_err(SendFailure::NotSent)?;
    send_prepared_raw(state, params, &session, &history, source)
}

fn send_prepared_raw(
    state: PersistedState,
    params: &SendParams,
    session: &Session,
    history: &History,
    source: Source,
) -> Result<(AgentResponse, ResponseData), SendFailure> {
    // Mask every secret the session knows, used in this request or not.
    let scrubber = Scrubber::new(&state, &session.bearer);
    let bearer = if params.use_saved_bearer.unwrap_or(false) { session.bearer.as_str() } else { "" };
    let max_body_chars = params.max_body_chars.unwrap_or(DEFAULT_MAX_BODY_CHARS);
    match engine::send(state, bearer, Some(history), source) {
        Ok(sent) => {
            let mut shaped = AgentResponse::from_sent(&sent, &scrubber, max_body_chars);
            if let Some(paths) = &params.select {
                apply_select(&mut shaped, &sent.response, paths, &scrubber);
            }
            Ok((shaped, sent.response))
        }
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


mod requests;
mod variables;

pub use requests::*;
pub use variables::*;

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
