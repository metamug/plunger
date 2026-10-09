//! The request engine for agents: what the command line and the MCP server
//! call. It sends through exactly the same path as the window's Ctrl+Enter
//! (`prepare_to_send` then `http::execute`), so an undefined `{{variable}}` is
//! refused the same way, and every send lands in the same history.
//!
//! On top of that it shapes results for an agent: structured fields, a body
//! cut to a size a model can read, and no secret value anywhere in the output.

mod response;
mod session;

pub use response::*;
pub use session::*;

use crate::history::{History, HistoryEntry, Source};
use crate::http;
use crate::model::{BodyMode, PersistedState, ResponseData, Variable};
use crate::request::{headers_to_text, prepare_to_send};

/// Default cap on body text handed to an agent, in characters. Big enough for
/// typical API responses, small enough not to flood a model's context.
pub const DEFAULT_MAX_BODY_CHARS: usize = 50_000;
/// Secret values shorter than this are not masked in output: masking "1" or
/// "ab" would mangle unrelated text.
const MIN_MASKED_LEN: usize = 4;

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
    fn has_json_content_type(&self) -> bool {
        self.headers.iter().any(|(name, value)| {
            let value = value.to_ascii_lowercase();
            name.eq_ignore_ascii_case("content-type") && (value.contains("/json") || value.contains("+json"))
        })
    }

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
            // A body the caller marked as JSON (Content-Type) and that parses as JSON opens in the
            // window's JSON editor, the same as a curl import; anything else stays raw text.
            Body::Text(text) if self.has_json_content_type() && serde_json::from_str::<serde_json::Value>(text).is_ok() => {
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
    // An exact match still has to check for more than one: nothing stops two
    // saved requests sharing the exact same name (a duplicate save, or the
    // GUI's rename), and picking the first one silently would send whichever
    // is oldest instead of the one the caller meant.
    let exact: Vec<&HistoryEntry> = saved.iter().filter(|e| e.name.as_deref() == Some(wanted)).collect();
    match exact.as_slice() {
        [one] => return Ok((*one).clone()),
        [] => {}
        _ => return Err(format!("More than one saved request is named \"{wanted}\". Rename one of them to tell them apart.")),
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
pub(super) fn percent_encode_all(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;
    use crate::request::build_request;
    use crate::secrets::test_support::MemoryStore;
    use crate::test_server::serve_echo;

    fn session(vars: Vec<Variable>) -> Session {
        Session {
            state: PersistedState { variables: vars, ..Default::default() },
            bearer: String::new(),
            problems: Vec::new(),
            agent_variables: Default::default(),
        }
    }

    fn var(name: &str, value: &str, secret: bool) -> Variable {
        Variable { name: name.into(), value: value.into(), secret, remember: false }
    }

    fn spec(headers: &[(&str, &str)], body: Body) -> RequestSpec {
        RequestSpec {
            method: Some("POST".into()),
            url: "http://h/x".into(),
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            body,
            variables: vec![],
            timeout_secs: None,
            follow_redirects: None,
            insecure_tls: None,
        }
    }

    #[test]
    fn a_json_typed_text_body_opens_in_the_json_editor_and_other_text_stays_raw() {
        let s = session(vec![]);
        let json_ct = [("Content-Type", "application/json; charset=utf-8")];
        let state = spec(&json_ct, Body::Text("{\"a\":1}".into())).to_state(&s);
        assert!(state.body_mode == BodyMode::Json);
        assert_eq!(state.json_body, "{\"a\":1}");
        assert!(spec(&[("content-type", "application/vnd.api+json")], Body::Text("[1]".into())).to_state(&s).body_mode == BodyMode::Json);

        let raw = |headers: &[(&str, &str)], text: &str| spec(headers, Body::Text(text.into())).to_state(&s).body_mode == BodyMode::Raw;
        assert!(raw(&json_ct, "not json"), "invalid JSON stays raw");
        assert!(raw(&[("Content-Type", "text/plain")], "123"), "plain text that happens to parse stays raw");
        assert!(raw(&[("Content-Type", "application/xml")], "<a/>"));
        assert!(raw(&[], "{\"a\":1}"), "no Content-Type stays raw");
    }

    #[test]
    fn an_undefined_variable_is_refused_and_nothing_is_recorded() {
        let history = History::in_memory();
        let s = session(vec![]);
        let spec = RequestSpec { url: "http://127.0.0.1:9/{{missing}}".into(), ..Default::default() };
        let err = send(spec.to_state(&s), "", Some(&history), Source::Mcp).err().unwrap();
        let SendError::Refused(message) = err else { panic!("expected a refusal") };
        assert!(message.contains("{{missing}}"), "{message}");
        assert!(history.search_recent("", 5).unwrap().is_empty());
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
        let rows = history.search_recent("", 5).unwrap();
        assert_eq!(rows[0].source, Source::Mcp);
        assert!(!rows[0].url.contains("SUPER-SECRET-VALUE") && !rows[0].headers_text.contains("SUPER-SECRET-VALUE"));
        assert_eq!(out.request.history_id, Some(rows[0].id));
    }

    #[test]
    fn overriding_a_secret_variable_sends_and_masks_the_override_not_the_stored_secret() {
        let base = serve_echo(1);
        let history = History::in_memory();
        let s = session(vec![var("apiToken", "STORED-SECRET-VALUE", true)]);
        let spec = RequestSpec {
            url: format!("{base}/echo?token={{{{apiToken}}}}"),
            variables: vec![("apiToken".into(), "OVERRIDE-TOKEN-1234".into())],
            ..Default::default()
        };
        let state = spec.to_state(&s);
        // The override replaces the value in place and keeps the flag.
        assert!(state.variables[0].secret, "an override must not un-secret a variable");
        let scrubber = Scrubber::new(&state, "");
        let sent = send(state, "", Some(&history), Source::Mcp).unwrap();
        let out = AgentResponse::from_sent(&sent, &scrubber, DEFAULT_MAX_BODY_CHARS);
        let json = serde_json::to_string(&out).unwrap();
        // The override is what went out, and it is what gets masked.
        let echoed = out.json.as_ref().and_then(|j| j.get("request")).and_then(|v| v.as_str()).unwrap();
        assert!(echoed.contains("token=[redacted:apiToken]"), "{echoed}");
        assert!(!json.contains("OVERRIDE-TOKEN-1234"), "{json}");
        // The stale stored secret is neither sent nor echoed anywhere.
        assert!(!json.contains("STORED-SECRET-VALUE"), "{json}");
        assert_eq!(out.redacted, vec!["apiToken".to_string()]);
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
        let s = Session::load_with(&file, &store, None);
        assert_eq!(s.state.variables[1].value, "T0KEN-VALUE");

        let listed = serde_json::to_string(&s.variables()).unwrap();
        assert!(listed.contains("http://h") && listed.contains("\"token\""));
        assert!(!listed.contains("T0KEN-VALUE"), "{listed}");
        assert!(s.variables()[1].remembered && s.variables()[1].has_value);
    }

    #[test]
    fn a_missing_state_file_is_just_an_empty_session() {
        let s = Session::load_with(Path::new("Z:/nope/app.ron"), &MemoryStore::default(), None);
        assert!(s.state.variables.is_empty());
    }

    #[test]
    fn existing_exact_name_duplicates_are_reported_as_ambiguous() {
        let path = std::env::temp_dir().join(format!(
            "plunger-legacy-duplicates-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE requests (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    created_at TEXT NOT NULL,
                    method TEXT NOT NULL,
                    url TEXT NOT NULL,
                    headers_text TEXT NOT NULL,
                    body_mode TEXT NOT NULL,
                    json_body TEXT NOT NULL,
                    urlencoded_body TEXT NOT NULL,
                    raw_body TEXT NOT NULL,
                    status INTEGER,
                    elapsed_ms INTEGER,
                    name TEXT
                );
                INSERT INTO requests
                    (created_at, method, url, headers_text, body_mode, json_body,
                     urlencoded_body, raw_body, name)
                VALUES
                    ('t', 'GET', 'http://a/1', '', 'None', '', '', '', 'Dup'),
                    ('t', 'GET', 'http://a/2', '', 'None', '', '', '', 'Dup');",
            )
            .unwrap();
        }

        let h = History::open_at(&path);
        let Err(err) = find_saved(&h, "Dup") else { panic!("expected an ambiguity error") };
        assert!(err.contains("More than one"), "{err}");
        drop(h);
        let _ = std::fs::remove_file(path);
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
        assert_eq!(info.variables_used, vec!["base", "id", "token"]);
    }

    #[test]
    fn environment_variables_are_listed_and_secret_looking_ones_are_masked() {
        let state = PersistedState {
            url: "{{base}}/x?e={{$env:PLUNGER_TEST_API_TOKEN}}&n={{$env:PLUNGER_TEST_REGION}}&u={{$uuid}}".into(),
            ..Default::default()
        };
        assert_eq!(variables_used(&state), vec!["base", "$env:PLUNGER_TEST_API_TOKEN", "$env:PLUNGER_TEST_REGION"]);

        std::env::set_var("PLUNGER_TEST_API_TOKEN", "tok-12345678");
        std::env::set_var("PLUNGER_TEST_REGION", "eu-west-1");
        let scrubber = Scrubber::new(&state, "");
        assert_eq!(scrubber.text("a tok-12345678 b eu-west-1"), "a [redacted:$env:PLUNGER_TEST_API_TOKEN] b eu-west-1");
    }

    #[test]
    fn form_values_are_encoded_but_variables_are_left_for_the_send_step() {
        assert_eq!(encode_keeping_variables("a&b c=d"), "a%26b%20c=d");
        assert_eq!(encode_keeping_variables("Bearer {{tok}}&x"), "Bearer%20{{tok}}%26x");
        assert_eq!(encode_keeping_variables("{{a}}{{b}}"), "{{a}}{{b}}");
        assert_eq!(encode_keeping_variables("open {{ never closed"), "open%20{{%20never%20closed");
    }

    #[test]
    fn a_binary_response_is_flagged_and_its_bytes_are_not_returned() {
        let sent = Sent {
            response: ResponseData {
                sent_at: String::new(),
                status: 200,
                status_text: "OK".into(),
                ttfb_ms: 1,
                elapsed_ms: 1,
                size_bytes: 4,
                request_size_bytes: Some(0),
                headers: vec![("content-type".into(), "image/png".into())],
                redirect_chain: vec![],
                body: String::new(),
                raw_text: None,
                json_value: None,
                truncated: false,
                total_size: Some(4),
                binary: Some(vec![0x89, 0, 1, 2]),
                json_display: None,
                json_nodes: 0,
            },
            history_id: None,
            state: PersistedState { url: "http://h/x.png".into(), ..Default::default() },
        };
        let out = AgentResponse::from_sent(&sent, &Scrubber::new(&sent.state, ""), 50_000);
        assert!(out.binary && out.body.is_none() && out.json.is_none());
        assert_eq!(out.size_bytes, 4);
        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(json["binary"], true);
        // Text responses don't carry the flag at all.
        let mut text = sent;
        text.response.binary = None;
        text.response.body = "hi".into();
        assert!(serde_json::to_value(AgentResponse::from_sent(&text, &Scrubber::new(&text.state, ""), 50_000)).unwrap().get("binary").is_none());
    }
}
