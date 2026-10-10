//! What an agent is told about a response: secrets masked, the body cut to a readable size,
//! and the structured shapes the CLI and MCP tools return.

use super::{percent_encode_all, Sent, VariableInfo, MIN_MASKED_LEN};
use crate::store::history::HistoryEntry;
use crate::domain::model::{BodyMode, PersistedState};
use crate::domain::redact::{is_sensitive_header, redact_url};
use crate::domain::request::parse_headers;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

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
        // An environment variable that looks like a credential, used by this request.
        for name in variables_used(state) {
            let Some(env_name) = name.strip_prefix("$env:") else { continue };
            if !crate::domain::redact::is_secret_env_name(env_name) {
                continue;
            }
            if let Ok(value) = std::env::var(env_name) {
                if value.trim().len() >= MIN_MASKED_LEN {
                    secrets.push((name.clone(), value.trim().to_string()));
                }
            }
        }
        // Servers often echo values URL-encoded (a query string, a form body),
        // so mask those spellings too.
        let encoded: Vec<(String, String)> = secrets
            .iter()
            .flat_map(|(name, value)| {
                [crate::domain::query::encode_value(value), percent_encode_all(value)]
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

    /// A scrubber for one value that became a secret after the scrubber for the request was built
    /// (a token the response itself carried and `extract` just stored).
    pub fn only(name: &str, value: &str) -> Option<Self> {
        if value.trim().len() < MIN_MASKED_LEN {
            return None;
        }
        let value = value.trim().to_string();
        let mut secrets = vec![(name.to_string(), value.clone())];
        for encoded in [crate::domain::query::encode_value(&value), percent_encode_all(&value)] {
            if encoded != value {
                secrets.push((name.to_string(), encoded));
            }
        }
        secrets.dedup();
        secrets.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
        Some(Self { secrets })
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
    /// The body isn't text, so it is not included; see `size_bytes` and the Content-Type header.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
    pub headers: Vec<NameValue>,
    pub request: SentRequest,
    /// Secrets whose values appeared in the response and were replaced with
    /// `[redacted:<name>]`. Names only, never values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redacted: Vec<String>,
    /// With `select`: just the values asked for, by the path given. `json` and `body` are then left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    /// With `extract`: the variables that were set from this response (secrets by name only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables_set: Vec<VariableInfo>,
    /// Things that did not work in `select` or `extract`, such as a path that is not in the response.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// A JSON body too long to return whole: its shape (keys, types, array lengths, one example each)
    /// instead of cut-off text. Ask for the values you need with `select`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<serde_json::Value>,
    /// What to do next, when the response was too big to return whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct SentRequest {
    pub method: String,
    /// With credential-looking query values blanked.
    pub url: String,
    /// Row in the shared history, visible in the Plunger window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_id: Option<i64>,
    /// When the request was fired, RFC 3339 in UTC.
    #[serde(default)]
    pub sent_at: String,
}

impl AgentResponse {
    /// Masks one more secret in everything an agent will read, and records its name.
    pub fn mask_with(&mut self, scrubber: &Scrubber) {
        let mut found = Vec::new();
        if let Some(json) = &self.json {
            found.extend(scrubber.found_in(&json.to_string()));
            self.json = Some(scrubber.json(json));
        }
        if let Some(body) = &self.body {
            found.extend(scrubber.found_in(body));
            self.body = Some(scrubber.text(body));
        }
        if let Some(selected) = &mut self.selected {
            for value in selected.values_mut() {
                found.extend(scrubber.found_in(&value.to_string()));
                *value = scrubber.json(value);
            }
        }
        for header in &mut self.headers {
            found.extend(scrubber.found_in(&header.value));
            header.value = scrubber.text(&header.value);
        }
        found.sort();
        found.dedup();
        for name in found {
            if !self.redacted.contains(&name) {
                self.redacted.push(name);
            }
        }
    }

    pub fn from_sent(sent: &Sent, scrubber: &Scrubber, max_body_chars: usize) -> Self {
        let r = &sent.response;
        let mut redacted = scrubber.found_in(&r.body);
        for (_, v) in &r.headers {
            redacted.extend(scrubber.found_in(v));
        }
        redacted.sort();
        redacted.dedup();

        let body_chars = r.body.chars().count();
        let mut outline = None;
        let mut hint = None;
        let (json, body, body_cut_from_chars) = match &r.json_value {
            _ if r.binary.is_some() => (None, None, None),
            Some(value) if body_chars <= max_body_chars => (Some(scrubber.json(value)), None, None),
            Some(value) => {
                // Too long to return whole: the shape is worth more than the first part of it.
                outline = Some(crate::domain::outline::outline(&scrubber.json(value)));
                hint = Some(format!(
                    "The JSON body is {body_chars} characters, so only its outline is returned. Send again with `select` (for example [\"$.data[0].id\", \"$.items[*].name\"]) for the values you need, or a larger `max_body_chars` for the whole body."
                ));
                (None, None, Some(body_chars))
            }
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
            binary: r.binary.is_some(),
            headers,
            request: SentRequest {
                method: sent.state.method.clone(),
                url: scrubber.text(&redact_url(&sent.state.url)),
                history_id: sent.history_id,
                sent_at: r.sent_at.clone(),
            },
            redacted,
            selected: None,
            variables_set: Vec::new(),
            problems: Vec::new(),
            outline,
            hint,
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

/// `{{names}}` referenced anywhere in the request, in order of first use. The built-ins that fill
/// themselves in (`$uuid`, ...) are left out; an environment variable is listed as `$env:NAME`
/// because it must be set when the request is sent.
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
        let is_env = name.strip_prefix("$env:").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        let valid = is_env
            || (!name.is_empty()
                && !name.starts_with('$')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'));
        if valid && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        rest = &after[end + 2..];
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_the_response_carried_is_masked_once_it_becomes_a_secret() {
        let mut shaped: AgentResponse = serde_json::from_value(serde_json::json!({
            "ok": true, "status": 200, "status_text": "OK", "elapsed_ms": 1, "size_bytes": 10, "truncated_at_10mb": false,
            "json": {"token": "tok_secret_1", "user": {"id": 7}},
            "selected": {"$.token": "tok_secret_1", "$.user.id": 7},
            "headers": [{"name": "X-Echo", "value": "tok_secret_1"}],
            "request": {"method": "POST", "url": "http://h/login"}
        }))
        .unwrap();
        shaped.mask_with(&Scrubber::only("token", "tok_secret_1").unwrap());
        let text = serde_json::to_string(&shaped).unwrap();
        assert!(!text.contains("tok_secret_1"), "{text}");
        assert!(text.contains("[redacted:token]") && text.contains("\"user\":{\"id\":7}"));
        assert_eq!(shaped.redacted, vec!["token".to_string()]);
        assert!(Scrubber::only("t", "ab").is_none(), "a value too short to mask is left alone");
    }
}
