use serde::{Deserialize, Serialize};

#[derive(PartialEq, Clone, Copy, Serialize, Deserialize, Default)]
pub enum BodyMode {
    #[default]
    None,
    Json,
    UrlEncoded,
    Raw,
}

#[derive(PartialEq, Clone, Copy)]
pub enum RequestTab {
    Headers,
    Body,
}

#[derive(PartialEq, Clone, Copy)]
pub enum ResponseTab {
    Body,
    Headers,
}

/// The subset of app state worth remembering between runs — request config,
/// not transient things like "is a request in flight right now."
#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct PersistedState {
    pub method: String,
    pub url: String,
    pub headers_text: String,
    pub body_mode: BodyMode,
    pub json_body: String,
    pub urlencoded_body: String,
    pub raw_body: String,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            method: "GET".to_string(),
            url: "https://jsonplaceholder.typicode.com/todos/1".to_string(),
            headers_text: String::new(),
            body_mode: BodyMode::None,
            json_body: String::from("{\n  \"key\": \"value\"\n}"),
            urlencoded_body: String::new(),
            raw_body: String::new(),
        }
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
}

/// A parsed request — the common output shape for both curl and HAR import,
/// so the UI only needs one "apply this" code path regardless of source.
pub struct ParsedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}
