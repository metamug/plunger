//! The operations an agent can run, shared by the command line and the MCP
//! server so both behave identically. Each returns a serializable result or
//! a message saying what went wrong; neither ever contains a secret value.

use crate::convert::curl_export::to_curl;
use crate::convert::curl_import::parse_curl;
use crate::engine::{
    self, AgentResponse, Body, HistoryItem, NameValue, RequestSpec, Scrubber, SendError, Session, StoredRequestInfo,
    VariableInfo, DEFAULT_MAX_BODY_CHARS,
};
use crate::store::history::{History, Source};
use crate::domain::redact::is_sensitive_header;
use crate::domain::model::{BodyMode, FieldKind, PersistedState, ResponseData};
use crate::domain::request::{headers_to_text, parse_headers};
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
    /// A list of {"name", "value"} objects or "Name: value" strings is accepted too.
    #[serde(default, deserialize_with = "lenient_headers")]
    #[schemars(with = "Option<BTreeMap<String, String>>")]
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
    /// Follow redirects (default true). Set false to receive the redirect response itself and read its `Location` header.
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    /// Skip TLS certificate checks, e.g. for a local server with a self-signed certificate.
    #[serde(default)]
    pub insecure_tls: Option<bool>,
    /// Longest body to return, in characters (default 50000). A longer JSON body comes back as an outline of its shape (keys, types, array lengths) with a hint to use `select`; a longer text body is cut and marked with `body_cut_from_chars`.
    #[serde(default)]
    pub max_body_chars: Option<usize>,
    /// Keep values from the response as variables for later requests, e.g.
    /// [{"name": "token", "from": "json:$.access_token"}]. `from` is `json:$.path`, `header:Name` or `status`.
    /// A name like token or password is kept as a hidden secret.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extract: Vec<crate::domain::extract::Extract>,
    /// Return only these values instead of the whole body, e.g. ["$.data[0].id", "header:Location", "status"].
    /// Saves context on a big response; `json`, `body` and the headers are then left out. `[?(@.name=='Done')]`
    /// keeps the array items whose field equals the value. If every path fails the whole response is returned with the reasons; if only some fail, the values that could be read come back and `problems` names the rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Vec<String>>,
    /// Which response headers to return: `all`, `none`, or `relevant` (location, link, retry-after,
    /// content-type, content-disposition, etag, rate-limit headers and set-cookie names). Left out:
    /// `all`, or `none` when `select` is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_headers: Option<String>,
    /// Fields this version does not know. They are refused by name, never ignored, so a request that
    /// asks for something Plunger cannot do (`retries`) fails loudly instead of quietly not doing it.
    #[serde(flatten, default, skip_serializing)]
    #[schemars(skip)]
    pub unknown_fields: BTreeMap<String, serde_json::Value>,
}

/// The names of the fields of [`SendParams`], for the message that refuses an unknown one.
const SEND_FIELDS: &str = "saved_request, method, url, headers, json, body, form, variables, use_saved_bearer, \
timeout_secs, follow_redirects, insecure_tls, max_body_chars, include_headers, extract, select";

/// `headers` as an object, a list of {"name", "value"} (or {"key", "value"}) objects, or a list of
/// "Name: value" strings: the shapes an agent tends to produce.
fn lenient_headers<'de, D>(deserializer: D) -> Result<Option<BTreeMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let text = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    };
    let bad = || D::Error::custom("`headers` is an object like {\"Accept\": \"application/json\"}; a list of {\"name\", \"value\"} objects or \"Name: value\" strings also works");
    let Some(value) = value else { return Ok(None) };
    let mut out = BTreeMap::new();
    match value {
        serde_json::Value::Null => return Ok(None),
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                out.insert(k, text(&v).ok_or_else(bad)?);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                match item {
                    serde_json::Value::String(line) => {
                        let (k, v) = line.split_once(':').ok_or_else(bad)?;
                        out.insert(k.trim().to_string(), v.trim().to_string());
                    }
                    serde_json::Value::Object(map) => {
                        let name = map.get("name").or_else(|| map.get("key")).and_then(text).ok_or_else(bad)?;
                        let value = map.get("value").and_then(text).ok_or_else(bad)?;
                        out.insert(name, value);
                    }
                    _ => return Err(bad()),
                }
            }
        }
        _ => return Err(bad()),
    }
    Ok(Some(out))
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
        if let Some(form) = &self.form {
            // `@path` means "upload this file" to curl; over MCP it would only be sent as the text `@path`
            if let Some((field, value)) = form.iter().find(|(_, v)| v.starts_with('@') && v.contains(['/', '\\', '.'])) {
                return Err(format!(
                    "`form` value for `{field}` is `{value}`, which looks like a file to upload. Plunger's MCP `form` sends \
                     text only (it would send the characters `@...`). Upload with the window or `plunger send --form`, \
                     or send the content as text in `body`."
                ));
            }
        }
        Ok(match (&self.json, &self.body, &self.form) {
            // an agent often gives the JSON as a string: that is the body it meant, not a JSON string value
            (Some(serde_json::Value::String(text)), _, _)
                if serde_json::from_str::<serde_json::Value>(text).is_ok_and(|v| v.is_object() || v.is_array()) =>
            {
                Body::Json(text.trim().to_string())
            }
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
    /// These parameters with the saved request's own `select` filled in when the call gave none.
    fn with_saved_select(&self, history: &History) -> SendParams {
        let mut params = self.clone();
        if params.select.is_none() {
            if let Some(name) = &self.saved_request {
                if let Ok(entry) = engine::find_saved(history, name) {
                    if !entry.options.select.is_empty() {
                        params.select = Some(entry.options.select);
                    }
                }
            }
        }
        params
    }

    pub(crate) fn to_state(&self, session: &Session, history: &History) -> Result<PersistedState, String> {
        if let Some(mode) = &self.include_headers {
            if !matches!(mode.trim().to_ascii_lowercase().as_str(), "all" | "none" | "relevant") {
                return Err(format!("`include_headers` is `all`, `none` or `relevant`, not `{mode}`."));
            }
        }
        if !self.unknown_fields.is_empty() {
            let names: Vec<&str> = self.unknown_fields.keys().map(String::as_str).collect();
            return Err(format!(
                "Unknown field{} `{}`: Plunger has no such option, so it was not sent. Fields: {SEND_FIELDS}.",
                if names.len() == 1 { "" } else { "s" },
                names.join("`, `")
            ));
        }
        let body = self.body()?;
        if let Some(h) = &self.headers {
            crate::domain::request::check_header_lines(h.iter())?;
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
    send_request_full(params, source).map(|(shaped, _)| shaped)
}

/// Like [`send_request`], also returning the response as the server sent it, for what needs the real bytes
/// (writing a body to a file). The raw response must not reach an agent.
pub fn send_request_full(params: &SendParams, source: Source) -> Result<(AgentResponse, ResponseData), SendFailure> {
    // A mistake in `extract` or `select` is reported now, not after the request has already had its effect.
    check_reading_instructions(params).map_err(SendFailure::NotSent)?;
    let session = Session::load();
    let history = open_history().map_err(SendFailure::NotSent)?;
    let state = params.to_state(&session, &history).map_err(SendFailure::NotSent)?;
    let params = &params.with_saved_select(&history);
    let (mut shaped, raw) = send_prepared_raw(state, params, &session, &history, source)?;
    for e in &params.extract {
        let stored = crate::domain::extract::extract(&raw, &e.from).and_then(|value| set_variable(e.name.trim(), &value, e.secret, source).map(|info| (info, value)));
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
    Ok((shaped, raw))
}

/// Whether a workflow condition holds: `expression` with the session's variables and `values` filled in
/// (a variable with no value is empty, never an error), then `==`, `!=` or truthiness.
pub fn evaluate_condition(expression: &str, values: &BTreeMap<String, String>) -> Result<bool, String> {
    let session = Session::load();
    let mut variables = session.state.variables.clone();
    variables.extend(values.iter().map(|(name, value)| crate::domain::model::Variable { name: name.clone(), value: value.clone(), secret: false, remember: false }));
    // later entries win, as they do for a request
    variables.reverse();
    let filled = crate::domain::vars::Resolver::new(&variables).apply(expression);
    Ok(crate::domain::condition::condition_holds(&filled))
}

/// Checks `extract` and `select` before anything is sent.
pub(crate) fn check_reading_instructions(params: &SendParams) -> Result<(), String> {
    for e in &params.extract {
        variables::valid_variable_name(&e.name).map_err(|m| format!("`extract` name: {m}"))?;
        crate::domain::extract::check_source(&e.from, false).map_err(|m| format!("`extract` for `{}`: {m}", e.name.trim()))?;
    }
    for path in params.select.iter().flatten() {
        crate::domain::extract::check_source(path, true).map_err(|m| format!("`select` `{path}`: {m}"))?;
    }
    Ok(())
}

/// Headers worth an agent's tokens when it asks for `relevant`.
fn is_relevant_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "location" | "link" | "retry-after" | "content-type" | "content-disposition" | "etag" | "set-cookie" | "www-authenticate" | "x-request-id"
    ) || n.starts_with("x-ratelimit")
        || n.starts_with("ratelimit")
        || n.starts_with("x-rate-limit")
}

/// Sets the response headers as `include_headers` says; without it, none when `select` was used (a header
/// is still available as `header:Name`) and all otherwise. A failed `select` returns the whole response.
fn apply_header_mode(shaped: &mut AgentResponse, every_header: Vec<engine::NameValue>, params: &SendParams) {
    let default = if params.select.is_some() && shaped.problems.is_empty() { "none" } else { "all" };
    let mode = params.include_headers.as_deref().map(|m| m.trim().to_ascii_lowercase());
    shaped.headers = match mode.as_deref().unwrap_or(default) {
        "none" => Vec::new(),
        "relevant" => every_header
            .into_iter()
            .filter(|h| is_relevant_header(&h.name))
            .map(|mut h| {
                // a cookie's name is useful, its value is not: the value is a credential
                if h.name.eq_ignore_ascii_case("set-cookie") {
                    h.value = h.value.split(['=', ';']).next().unwrap_or("").to_string();
                }
                h
            })
            .collect(),
        _ => every_header,
    };
}

/// Replaces `json`, `body` and the response headers with just the values asked for, so a big response
/// costs a few tokens (a header is still available as `header:Name`). Values come from the response as
/// sent and are masked like the rest of the output.
///
/// If a path could not be read (the server answered with an error page, say), the response is returned
/// whole instead, with the reason in `problems`: the request has already been sent, and sending it
/// again to see what came back could do its work twice.
fn apply_select(shaped: &mut AgentResponse, raw: &ResponseData, paths: &[String], scrubber: &Scrubber) {
    let mut selected = BTreeMap::new();
    let mut failed = 0;
    for path in paths {
        match crate::domain::extract::select(raw, path) {
            Ok(value) => {
                selected.insert(path.clone(), scrubber.json(&value));
            }
            Err(err) => {
                failed += 1;
                shaped.problems.push(format!("could not select `{path}`: {err}"));
            }
        }
    }
    if failed > 0 && selected.is_empty() {
        shaped.selected = Some(selected);
        shaped.problems.push("`select` did not work, so the whole response is returned (the request was sent once).".to_string());
        return;
    }
    if failed > 0 {
        // the paths that worked are the answer; returning the whole body as well would cost what `select` is for
        shaped.problems.push(format!(
            "{failed} of {} `select` paths could not be read (named above); the values that could are in `selected`. The request was sent once.",
            paths.len()
        ));
    }
    shaped.selected = Some(selected);
    shaped.json = None;
    shaped.body = None;
    shaped.body_cut_from_chars = None;
    shaped.headers.clear();
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
    send_prepared_raw(state, &params.with_saved_select(&history), &session, &history, source)
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
            // things that were quietly adjusted are said, so an agent does not rely on what did not happen
            if let Some(t) = params.timeout_secs.filter(|t| !(1..=600).contains(t)) {
                shaped.problems.push(format!("timeout_secs {t} is outside 1 to 600, so {} was used", t.clamp(1, 600)));
            }
            if let Some(m) = params.method.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                const STANDARD: [&str; 9] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE", "CONNECT"];
                if !STANDARD.contains(&m.to_ascii_uppercase().as_str()) {
                    shaped.problems.push(format!("`{m}` is not a standard HTTP method; it was sent as written"));
                }
            }
            let every_header = shaped.headers.clone();
            if let Some(paths) = &params.select {
                apply_select(&mut shaped, &sent.response, paths, &scrubber);
            }
            apply_header_mode(&mut shaped, every_header, params);
            Ok((shaped, sent.response))
        }
        Err(SendError::Refused(msg)) => {
            // The window's advice ("define it in Tools > Variables") isn't
            // something an agent can do; say how it can supply one instead.
            // the window's advice ("define it in Tools > Variables") is dropped for an agent
            let (msg, hint) = if msg.starts_with("Undefined variable") {
                let cut = msg.find(" Define ").map_or(msg.as_str(), |at| &msg[..at]);
                (
                    cut.to_string(),
                    " Pass a value for this request: `variables` in MCP, `--var name=value` on the command line; set it for later requests with set_variable. \
                     `{{name?}}` makes it optional and `{{name|default}}` gives a fallback.",
                )
            } else {
                (msg, "")
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
        use crate::store::secrets::test_support::MemoryStore;
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
        use crate::store::secrets::test_support::MemoryStore;
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
        use crate::store::secrets::test_support::MemoryStore;
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

    fn plain_session() -> Session {
        Session { state: PersistedState::default(), bearer: String::new(), problems: vec![], agent_variables: Default::default() }
    }

    #[test]
    fn a_saved_request_keeps_its_timeout_and_select_and_a_call_can_still_override_them() {
        let h = History::in_memory();
        let session = plain_session();
        let params = SendParams {
            url: Some("https://h/issues".into()),
            timeout_secs: Some(90),
            follow_redirects: Some(false),
            select: Some(vec!["$.key".into(), "header:Location".into()]),
            ..Default::default()
        };
        let saved = requests::save_request_with(&h, &session, "create issue", false, &params, None, Source::Mcp).unwrap();
        // it used to say nothing and keep neither; now both are stored and shown
        let options = saved.options.expect("the options are echoed back");
        assert_eq!((options.timeout_secs, options.follow_redirects), (Some(90), Some(false)));
        assert_eq!(options.select, vec!["$.key".to_string(), "header:Location".to_string()]);
        assert!(saved.not_saved.is_empty());

        let entry = engine::find_saved(&h, "create issue").unwrap();
        assert_eq!(entry.options.timeout_secs, Some(90));
        assert_eq!(entry.options.select, vec!["$.key".to_string(), "header:Location".to_string()]);

        // sending it by name uses them: the timeout in the state, the select in the params
        let by_name = SendParams { saved_request: Some("create issue".into()), ..Default::default() };
        let state = by_name.to_state(&session, &h).unwrap();
        assert_eq!((state.timeout_secs, state.follow_redirects), (90, false));
        assert_eq!(by_name.with_saved_select(&h).select, Some(vec!["$.key".to_string(), "header:Location".to_string()]));

        // a value in the call wins
        let overridden = SendParams {
            saved_request: Some("create issue".into()),
            timeout_secs: Some(5),
            select: Some(vec!["status".into()]),
            ..Default::default()
        };
        assert_eq!(overridden.to_state(&session, &h).unwrap().timeout_secs, 5);
        assert_eq!(overridden.with_saved_select(&h).select, Some(vec!["status".to_string()]));

        // get_saved_request shows them
        assert_eq!(requests::show_saved_request_in(&h, "create issue").unwrap().options.unwrap().timeout_secs, Some(90));
    }

    #[test]
    fn saving_the_form_again_keeps_the_options_and_overwriting_replaces_them() {
        let h = History::in_memory();
        let session = plain_session();
        let params = SendParams { url: Some("https://h/x".into()), timeout_secs: Some(60), ..Default::default() };
        let saved = requests::save_request_with(&h, &session, "slow", false, &params, None, Source::Cli).unwrap();
        let id = saved.saved_id.unwrap();

        // the window's Save writes the form back; the options an agent saved must survive it
        let mut form = engine::find_saved(&h, "slow").unwrap().to_persisted_state();
        form.url = "https://h/y".into();
        h.update_request(id, &form).unwrap();
        let entry = engine::find_saved(&h, "slow").unwrap();
        assert_eq!((entry.url.as_str(), entry.options.timeout_secs), ("https://h/y", Some(60)));

        // saving it again from the agent side with no options clears them
        let again = SendParams { url: Some("https://h/y".into()), ..Default::default() };
        requests::save_request_with(&h, &session, "slow", true, &again, None, Source::Cli).unwrap();
        assert!(engine::find_saved(&h, "slow").unwrap().options.is_empty());
    }

    #[test]
    fn a_save_says_which_fields_it_does_not_keep_and_refuses_a_select_that_cannot_work() {
        let h = History::in_memory();
        let session = plain_session();
        let params = SendParams {
            url: Some("https://h/x".into()),
            variables: Some(BTreeMap::from([("a".into(), "1".into())])),
            use_saved_bearer: Some(true),
            max_body_chars: Some(10),
            ..Default::default()
        };
        let saved = requests::save_request_with(&h, &session, "r", false, &params, None, Source::Mcp).unwrap();
        let said = saved.not_saved.join(" | ");
        for field in ["variables", "use_saved_bearer", "max_body_chars"] {
            assert!(said.contains(field), "{said}");
        }
        assert!(saved.options.is_none());

        let bad = SendParams { url: Some("https://h/x".into()), select: Some(vec!["bogus".into()]), ..Default::default() };
        let err = requests::save_request_with(&h, &session, "bad", false, &bad, None, Source::Mcp).unwrap_err();
        assert!(err.contains("select"), "{err}");
        assert!(engine::find_saved(&h, "bad").is_err(), "nothing was saved");
    }

    fn shaped(json: serde_json::Value) -> AgentResponse {
        serde_json::from_value(serde_json::json!({
            "ok": true, "status": 200, "status_text": "OK", "elapsed_ms": 1, "size_bytes": 10, "truncated_at_10mb": false,
            "json": json,
            "headers": [{"name": "server", "value": "x"}, {"name": "location", "value": "/issue/1"}],
            "request": {"method": "POST", "url": "http://h/issue"}
        }))
        .unwrap()
    }

    fn response_with(body: &str) -> ResponseData {
        ResponseData {
            status: 200,
            headers: vec![("Location".into(), "/issue/1".into())],
            body: body.into(),
            ..Default::default()
        }
    }

    #[test]
    fn select_returns_only_what_was_asked_for_and_leaves_the_header_noise_out() {
        let raw = response_with(r#"{"key": "PRJ-1", "fields": {"assignee": null}}"#);
        let mut out = shaped(serde_json::json!({"key": "PRJ-1"}));
        apply_select(&mut out, &raw, &["$.key".into(), "header:Location".into()], &Scrubber::new(&PersistedState::default(), ""));
        assert_eq!(out.selected.as_ref().unwrap()["$.key"], "PRJ-1");
        assert_eq!(out.selected.as_ref().unwrap()["header:Location"], "/issue/1");
        assert!(out.json.is_none() && out.body.is_none());
        assert!(out.headers.is_empty(), "the headers are what made every response long");
        assert!(out.problems.is_empty());
    }

    #[test]
    fn a_select_that_fails_returns_the_whole_response_instead_of_nothing() {
        // the server answered with an error page: select cannot read it, but the request has already been sent
        let raw = response_with("<html>502 Bad Gateway</html>");
        let mut out = shaped(serde_json::Value::Null);
        out.json = None;
        out.body = Some("<html>502 Bad Gateway</html>".into());
        apply_select(&mut out, &raw, &["$.key".into()], &Scrubber::new(&PersistedState::default(), ""));
        assert_eq!(out.body.as_deref(), Some("<html>502 Bad Gateway</html>"), "the body is kept");
        assert!(!out.headers.is_empty(), "so are the headers");
        let said = out.problems.join(" | ");
        assert!(said.contains("not JSON") && said.contains("whole response"), "{said}");
    }

    #[test]
    fn one_bad_path_does_not_cost_the_values_of_the_good_ones() {
        let raw = response_with(r#"{"key": "PRJ-1", "big": "xxxxxxxxxxxxxxxxxxxx"}"#);
        let mut out = shaped(serde_json::json!({"key": "PRJ-1"}));
        apply_select(&mut out, &raw, &["$.key".into(), "$.nope.x".into()], &Scrubber::new(&PersistedState::default(), ""));
        assert_eq!(out.selected.as_ref().unwrap()["$.key"], "PRJ-1");
        assert!(out.json.is_none() && out.body.is_none(), "the whole body is not returned when something was selected");
        let said = out.problems.join(" | ");
        assert!(said.contains("could not select `$.nope.x`") && said.contains("1 of 2"), "{said}");
    }

    #[test]
    fn selecting_below_a_null_is_null_not_a_problem() {
        let raw = response_with(r#"{"fields": {"assignee": null, "summary": "x"}}"#);
        let mut out = shaped(serde_json::json!({"fields": {"assignee": null}}));
        let paths = ["$.fields.assignee".to_string(), "$.fields.assignee.displayName".to_string(), "$.fields.summary".to_string()];
        apply_select(&mut out, &raw, &paths, &Scrubber::new(&PersistedState::default(), ""));
        let selected = out.selected.unwrap();
        assert_eq!(selected["$.fields.assignee"], serde_json::Value::Null);
        assert_eq!(selected["$.fields.assignee.displayName"], serde_json::Value::Null);
        assert_eq!(selected["$.fields.summary"], "x");
        assert!(out.problems.is_empty(), "{:?}", out.problems);
    }

    /// Every field a caller can give to `save_request` is accounted for: stored with the request, kept as
    /// one of its options, or named in `not_saved`. Adding a field to `SendParams` fails this test until
    /// the new field is placed in one of the three, so it can never be accepted and dropped silently.
    #[test]
    fn every_field_of_a_save_is_stored_kept_as_an_option_or_reported() {
        let schema = serde_json::to_value(rmcp::schemars::schema_for!(SendParams)).unwrap();
        let mut fields: Vec<String> = schema["properties"].as_object().unwrap().keys().cloned().collect();
        fields.sort();

        let stored = ["method", "url", "headers", "json", "body", "form"];
        let options = ["timeout_secs", "follow_redirects", "insecure_tls", "select"];
        let reported = ["variables", "use_saved_bearer", "extract", "max_body_chars", "include_headers"];
        let only_when_sending = ["saved_request"];
        let mut known: Vec<&str> = stored.iter().chain(&options).chain(&reported).chain(&only_when_sending).copied().collect();
        known.sort();
        assert_eq!(fields, known, "a SendParams field was added or removed: decide whether a saved request keeps it");

        // and each class does what it says
        let h = History::in_memory();
        let session = plain_session();
        let full: SendParams = serde_json::from_value(serde_json::json!({
            "method": "PUT", "url": "https://h/{{id}}", "headers": {"X-A": "1"}, "json": {"a": 1},
            "variables": {"id": "7"}, "use_saved_bearer": true, "timeout_secs": 45, "follow_redirects": false,
            "insecure_tls": true, "max_body_chars": 100, "include_headers": "none", "select": ["$.a"],
            "extract": [{"name": "n", "from": "json:$.a"}]
        }))
        .unwrap();
        let saved = requests::save_request_with(&h, &session, "all", false, &full, None, Source::Mcp).unwrap();
        assert_eq!((saved.method.as_str(), saved.url.as_str()), ("PUT", "https://h/{{id}}"));
        assert!(saved.headers.iter().any(|x| x.name == "X-A" && x.value == "1"));
        assert_eq!(saved.body.as_deref(), Some("{\"a\":1}"));
        let kept = saved.options.clone().expect("options");
        assert_eq!((kept.timeout_secs, kept.follow_redirects, kept.insecure_tls), (Some(45), Some(false), Some(true)));
        assert_eq!(kept.select, vec!["$.a".to_string()]);
        let said = saved.not_saved.join(" | ");
        for field in reported {
            // use_saved_bearer is only reported when asked for, the others when given
            assert!(said.contains(field), "{field} not reported in: {said}");
        }
        // what get_saved_request returns is the same as what the save said
        let shown = requests::show_saved_request_in(&h, "all").unwrap();
        assert_eq!((shown.method, shown.url, shown.body, shown.options), (saved.method, saved.url, saved.body, saved.options));
    }

    #[test]
    fn what_a_server_can_send_back_never_breaks_the_shaping() {
        // an empty body, plain text, an error page, a JSON null, a scalar and an array at the root
        let bodies = ["", "plain words", "<html>502</html>", "null", "42", "\"just a string\"", "[]", "[1, null, {\"a\": null}]", "{\"a\": {\"b\": null}}"];
        let paths = ["$.a", "$.a.b", "$.a.b.c", "$[0]", "$.items[*].id", "$.items[?(@.k=='x')].id", "header:Location", "header:Missing", "status"];
        let scrubber = Scrubber::new(&PersistedState::default(), "");
        for body in bodies {
            for status in [200u16, 204, 404, 500] {
                let raw = ResponseData {
                    status,
                    headers: vec![("Location".into(), "/x".into())],
                    body: body.into(),
                    json_value: serde_json::from_str(body).ok(),
                    ..Default::default()
                };
                let mut out = shaped(serde_json::Value::Null);
                out.json = raw.json_value.clone();
                out.body = if raw.json_value.is_none() { Some(body.to_string()) } else { None };
                let before = (out.json.clone(), out.body.clone());
                apply_select(&mut out, &raw, &paths.map(String::from), &scrubber);
                let selected = out.selected.as_ref().expect("select always reports");
                // it never panics; when no path could be read the body is still there to look at, and when only
                // some could not, the ones that could are returned
                if !out.problems.is_empty() && selected.is_empty() {
                    assert_eq!((out.json.clone(), out.body.clone()), before, "{body:?} {status}: the body was lost");
                    assert!(out.problems.iter().any(|p| p.contains("whole response")), "{body:?}: {:?}", out.problems);
                } else if !out.problems.is_empty() {
                    assert!(out.json.is_none() && out.body.is_none(), "{body:?} {status}: a partial select returns only values");
                    assert!(out.problems.iter().any(|p| p.contains(" of ")), "{body:?}: {:?}", out.problems);
                } else {
                    assert_eq!(selected.len(), paths.len());
                }
                // extract never panics either, and says why it could not
                for path in paths {
                    let from = if path.starts_with('$') { format!("json:{path}") } else { path.to_string() };
                    let _ = crate::domain::extract::extract(&raw, &from);
                }
            }
        }
    }

    #[test]
    fn the_shapes_an_agent_gets_wrong_are_understood_or_refused_by_name() {
        let h = History::in_memory();
        let session = plain_session();
        let parse = |v: serde_json::Value| serde_json::from_value::<SendParams>(v);

        // headers: an object, a list of name/value objects, a list of "Name: value" strings
        let expected = Some(BTreeMap::from([("Accept".to_string(), "application/json".to_string())]));
        for headers in [
            serde_json::json!({"Accept": "application/json"}),
            serde_json::json!([{"name": "Accept", "value": "application/json"}]),
            serde_json::json!([{"key": "Accept", "value": "application/json"}]),
            serde_json::json!(["Accept: application/json"]),
        ] {
            assert_eq!(parse(serde_json::json!({"url": "http://h", "headers": headers})).unwrap().headers, expected);
        }
        let err = parse(serde_json::json!({"url": "http://h", "headers": "Accept: x"})).unwrap_err().to_string();
        assert!(err.contains("headers"), "{err}");

        // json given as a string that holds an object or array is that body
        let from_string = parse(serde_json::json!({"url": "http://h", "json": "{\"a\": 1}"})).unwrap();
        assert_eq!(from_string.body().unwrap(), Body::Json("{\"a\": 1}".into()));
        // a string that is not JSON stays a JSON string
        let text = parse(serde_json::json!({"url": "http://h", "json": "hello"})).unwrap();
        assert_eq!(text.body().unwrap(), Body::Json("\"hello\"".into()));

        // a field Plunger does not have is named, and nothing is sent
        let unknown = parse(serde_json::json!({"url": "http://h", "retries": 3, "backoff": true})).unwrap();
        let err = unknown.to_state(&session, &h).err().unwrap();
        assert!(err.contains("`retries`") && err.contains("`backoff`") && err.contains("select"), "{err}");

        // a form value that is a file to upload is refused; other text beginning with @ is text
        let upload = parse(serde_json::json!({"url": "http://h", "form": {"f": "@C:/data/report.csv"}})).unwrap();
        assert!(upload.body().unwrap_err().contains("upload"));
        let mention = parse(serde_json::json!({"url": "http://h", "form": {"who": "@ann"}})).unwrap();
        assert!(mention.body().is_ok());
    }

    #[test]
    fn a_request_is_saved_in_a_group_found_by_it_and_moved_without_resending() {
        let h = History::in_memory();
        let session = plain_session();
        let p = |url: &str| SendParams { url: Some(url.into()), ..Default::default() };
        let one = requests::save_request_with(&h, &session, "create issue", false, &p("https://h/issue"), Some("Jira / Issues"), Source::Mcp).unwrap();
        assert_eq!(one.group.as_deref(), Some("Jira/Issues"), "the name is tidied");
        requests::save_request_with(&h, &session, "list projects", false, &p("https://g/projects"), Some("GitLab"), Source::Mcp).unwrap();
        requests::save_request_with(&h, &session, "health", false, &p("https://h/ping"), None, Source::Mcp).unwrap();

        // a bad group name saves nothing
        let err = requests::save_request_with(&h, &session, "bad", false, &p("https://h/x"), Some("a//b"), Source::Mcp).unwrap_err();
        assert!(err.contains("not a group name"), "{err}");
        assert!(engine::find_saved(&h, "bad").is_err());

        // replacing without a group keeps it; an empty group takes the request out of it
        requests::save_request_with(&h, &session, "create issue", true, &p("https://h/issue2"), None, Source::Mcp).unwrap();
        assert_eq!(engine::find_saved(&h, "create issue").unwrap().group.as_deref(), Some("Jira/Issues"));
        let cleared = requests::save_request_with(&h, &session, "create issue", true, &p("https://h/issue3"), Some(""), Source::Mcp).unwrap();
        assert_eq!(cleared.group, None);
        h.set_group(engine::find_saved(&h, "create issue").unwrap().id, Some("Jira/Issues")).unwrap();

        // get_saved_request shows the group, and the list can be narrowed to a group (and what is under it)
        assert_eq!(requests::show_saved_request_in(&h, "create issue").unwrap().group.as_deref(), Some("Jira/Issues"));
        let groups: Vec<Option<String>> = h.list_saved().unwrap().iter().map(|e| e.group.clone()).collect();
        assert_eq!(groups, vec![None, Some("GitLab".to_string()), Some("Jira/Issues".to_string())]);
        let jira: Vec<String> = h
            .list_saved()
            .unwrap()
            .iter()
            .filter(|e| crate::store::history::group_matches(e.group.as_deref(), "Jira"))
            .filter_map(|e| e.name.clone())
            .collect();
        assert_eq!(jira, vec!["create issue".to_string()]);
    }

    fn headers_of(names: &[(&str, &str)]) -> Vec<engine::NameValue> {
        names.iter().map(|(n, v)| engine::NameValue { name: (*n).into(), value: (*v).into() }).collect()
    }

    #[test]
    fn include_headers_chooses_all_none_or_the_relevant_ones() {
        let every = headers_of(&[
            ("server", "nginx"),
            ("date", "today"),
            ("Content-Type", "application/json"),
            ("Location", "/issue/1"),
            ("Retry-After", "3"),
            ("X-RateLimit-Remaining", "9"),
            ("Set-Cookie", "sid=abc123; HttpOnly"),
            ("x-powered-by", "rust"),
        ]);
        let run = |mode: Option<&str>, select: bool| {
            let mut out = shaped(serde_json::json!({"a": 1}));
            let params = SendParams {
                include_headers: mode.map(str::to_string),
                select: select.then(|| vec!["$.a".to_string()]),
                ..Default::default()
            };
            apply_header_mode(&mut out, every.clone(), &params);
            out.headers
        };
        assert_eq!(run(None, false).len(), 8, "left out: everything");
        assert!(run(None, true).is_empty(), "left out with select: none");
        assert!(run(Some("none"), false).is_empty());
        assert_eq!(run(Some("all"), true).len(), 8, "asked for explicitly, select does not take them away");
        let relevant = run(Some("Relevant"), false);
        let names: Vec<&str> = relevant.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["Content-Type", "Location", "Retry-After", "X-RateLimit-Remaining", "Set-Cookie"]);
        let cookie = relevant.iter().find(|h| h.name == "Set-Cookie").unwrap();
        assert_eq!(cookie.value, "sid", "only the cookie's name, never its value");

        // a mode that does not exist is refused before anything is sent
        let bad = SendParams { url: Some("http://h".into()), include_headers: Some("some".into()), ..Default::default() };
        let err = bad.to_state(&plain_session(), &History::in_memory()).err().unwrap();
        assert!(err.contains("include_headers"), "{err}");
    }
}