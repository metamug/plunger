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
                [crate::domain::query::encode_value(value), percent_encode_all(value), crate::domain::vars::json_escape(value)]
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
        for encoded in [crate::domain::query::encode_value(&value), percent_encode_all(&value), crate::domain::vars::json_escape(&value)] {
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
    /// Milliseconds until the response headers arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttfb_ms: Option<u64>,
    /// Where the request ended up, when it was redirected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    /// The redirects that were followed, in order (absent when there were none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<RedirectHop>,
    /// For a binary body: its SHA-256, so the agent can check it without receiving the bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// For a binary body: what its first bytes say it is (`png`, `pdf`, `zip`, ...), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_type: Option<String>,
    /// With `--out` on the command line: the file the whole body was written to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_file: Option<String>,
}

/// One redirect the request followed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RedirectHop {
    pub status: u16,
    /// Where it pointed.
    pub url: String,
}

/// What the first bytes of a binary body say it is.
pub fn sniff_type(bytes: &[u8]) -> Option<&'static str> {
    const SIGNATURES: [(&[u8], &str); 12] = [
        (b"\x89PNG\r\n\x1a\n", "png"),
        (b"\xff\xd8\xff", "jpeg"),
        (b"GIF87a", "gif"),
        (b"GIF89a", "gif"),
        (b"%PDF-", "pdf"),
        (b"PK\x03\x04", "zip"),
        (b"\x1f\x8b", "gzip"),
        (b"\x00asm", "wasm"),
        (b"RIFF", "riff (wav, webp or avi)"),
        (b"ID3", "mp3"),
        (b"\x7fELF", "elf executable"),
        (b"MZ", "windows executable"),
    ];
    SIGNATURES.iter().find(|(magic, _)| bytes.starts_with(magic)).map(|(_, name)| *name)
}

/// The SHA-256 of `bytes`, in lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().fold(String::with_capacity(64), |mut out, b| {
        out.push_str(&format!("{b:02x}"));
        out
    })
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
            ttfb_ms: Some(r.ttfb_ms as u64),
            final_url: r.redirect_chain.last().map(|(_, to)| scrubber.text(&redact_url(to))),
            redirects: r
                .redirect_chain
                .iter()
                .map(|(status, to)| RedirectHop { status: *status, url: scrubber.text(&redact_url(to)) })
                .collect(),
            sha256: r.binary.as_deref().map(sha256_hex),
            detected_type: r.binary.as_deref().and_then(sniff_type).map(str::to_string),
            body_file: None,
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
    /// none, json, xml, form-data, x-www-form-urlencoded or raw.
    pub body_type: String,
    /// `{{variables}}` this request needs. Built-ins like `{{$uuid}}` are filled in automatically.
    pub variables_used: Vec<String>,
    /// The group it is filed under (`Jira`, `Jira/Issues`), when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
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
        BodyMode::Xml => "xml",
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
            group: e.group.clone(),
        }
    }
}

pub use crate::domain::vars::variables_used;

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

    #[test]
    fn a_binary_body_is_identified_and_checked_but_never_returned() {
        // PNG signature, then bytes
        let bytes: Vec<u8> = [&b"\x89PNG\r\n\x1a\n"[..], &[0u8; 32][..]].concat();
        assert_eq!(sniff_type(&bytes), Some("png"));
        assert_eq!(sniff_type(b"%PDF-1.7 ..."), Some("pdf"));
        assert_eq!(sniff_type(b"PK\x03\x04rest"), Some("zip"));
        assert_eq!(sniff_type(b"plain"), None);
        // the well-known SHA-256 of "abc"
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");

        let response = crate::domain::model::ResponseData { status: 200, binary: Some(bytes.clone()), size_bytes: bytes.len(), ..Default::default() };
        let sent = crate::engine::Sent { response, history_id: None, state: PersistedState::default() };
        let shaped = AgentResponse::from_sent(&sent, &Scrubber::new(&PersistedState::default(), ""), 50_000);
        assert!(shaped.binary);
        assert_eq!(shaped.detected_type.as_deref(), Some("png"));
        assert_eq!(shaped.sha256.as_deref().map(str::len), Some(64));
        assert!(shaped.json.is_none() && shaped.body.is_none(), "the bytes are not in the result");
    }

    #[test]
    fn a_response_says_how_fast_it_started_and_where_a_redirect_went() {
        let plain = crate::domain::model::ResponseData { status: 200, ttfb_ms: 12, ..Default::default() };
        let sent = crate::engine::Sent { response: plain, history_id: None, state: PersistedState::default() };
        let scrubber = Scrubber::new(&PersistedState::default(), "");
        let shaped = AgentResponse::from_sent(&sent, &scrubber, 50_000);
        assert_eq!(shaped.ttfb_ms, Some(12));
        assert!(shaped.final_url.is_none() && shaped.redirects.is_empty(), "nothing about redirects when there were none");
        let text = serde_json::to_string(&shaped).unwrap();
        assert!(!text.contains("redirects") && !text.contains("final_url"), "{text}");

        let moved = crate::domain::model::ResponseData {
            status: 200,
            redirect_chain: vec![(301, "http://h/new?api_key=SECRETKEY1".into()), (302, "http://h/final".into())],
            ..Default::default()
        };
        let sent = crate::engine::Sent { response: moved, history_id: None, state: PersistedState::default() };
        let shaped = AgentResponse::from_sent(&sent, &scrubber, 50_000);
        assert_eq!(shaped.final_url.as_deref(), Some("http://h/final"));
        assert_eq!(shaped.redirects.iter().map(|r| r.status).collect::<Vec<_>>(), vec![301, 302]);
        assert!(!shaped.redirects[0].url.contains("SECRETKEY1"), "a credential in a redirect is blanked");
    }
}