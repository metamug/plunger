//! `plunger mcp`: a Model Context Protocol server over stdio, so an agent can
//! send HTTP requests through Plunger instead of running curl. Each tool is a
//! thin wrapper over `agent`, the same code the command line uses.
//!
//! Tool descriptions are what the agent's model reads when choosing a tool,
//! so they say when to reach for Plunger rather than raw curl.

use crate::agent::{self, ImportedRequest, SendParams, VariablesResult};
use crate::engine::{AgentResponse, HistoryItem, StoredRequestInfo, VariableInfo};
use crate::history::Source;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{tool, tool_handler, tool_router, Json, ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};

const INSTRUCTIONS: &str = "\
Plunger sends HTTP requests on the user's machine and shows every one of them in the Plunger window, \
so the user can see what you sent. Use it instead of running curl.

TOOLS
- list_saved_requests: the requests the user saved, by name. Check it before building a request from scratch.
- list_variables: the {{variables}} in use (secrets by name only; `source` says whether the user defined it in the window or an agent set it), the built-ins ($uuid, $timestamp, $randomInt, $env:NAME) and whether a Bearer token is saved.
- set_variable / delete_variable: keep a value for later requests as {{name}} (for example a token from a login response). It persists, and the user sees it in the window. A secret (or a name like token, password, api_key) goes to the system credential store and is masked in results. You cannot change or delete variables the user defined.
- send_request: send a request, or a saved one by name (`saved_request`) with overrides. Use `json`, `body` or `form` for the body; `headers` is an object like {\"Accept\": \"application/json\"}.
- save_request / get_saved_request / delete_saved_request: save a request without sending it (same fields as send_request, plus `name`; `overwrite: true` replaces an existing one), read one back in full, or remove one. {{placeholders}} are kept, so Authorization: Bearer {{token}} works when it is sent later.
- import_curl: parse a curl command into a request, and with `save_as` keep it in the user's Saved list.
- export_curl: a saved request or a history entry as a curl command.
- get_history: recent requests, who sent them (gui, cli, mcp), status and time; `search` filters.

HOW IT BEHAVES
- {{variables}} work in the URL, headers and body. Secret values are filled in for you and never shown; if a server echoes one back it appears as [redacted:name]. A request with an undefined {{variable}} is refused, not sent.
- `variables` on send_request sets values for that one request only; set_variable keeps them. {{$env:NAME}} reads an environment variable of the Plunger process when the request is sent (a name like API_TOKEN is masked in results); an agent cannot set one, because the environment is fixed when the server starts.
- A saved request keeps its {{placeholders}}, including in credential headers such as Authorization: Bearer {{token}}. Literal credentials are blanked when a request is saved.
- Redirects are followed unless `follow_redirects` is false. `timeout_secs` defaults to the user's setting.
- A response comes back as structured fields: status, timing, size, headers and the parsed JSON (`json`) or text (`body`). A body over `max_body_chars` (default 50000) is cut and marked with `body_cut_from_chars`. A binary body is not returned (`binary: true`, with its size). Set-Cookie and similar response headers are shown as [redacted].
- Failures are reported, not hidden: a request that could not be sent returns an error and is not recorded; one that got no response (DNS, refused, timeout) is recorded in the history.

NOT SUPPORTED YET
- Cookies are not carried from one request to the next, and multipart/form-data file uploads are not possible (`form` is url-encoded only).
- Values from a response are not extracted for you: read the value, then keep it with set_variable (use the name `token` or similar to make it a hidden secret).";

#[derive(Debug, Clone)]
pub struct PlungerMcp {
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ImportCurlParams {
    /// The full curl command, e.g. "curl -X POST https://api.example.com/items -H 'Content-Type: application/json' -d '{...}'".
    pub curl: String,
    /// Also save it in Plunger's Saved list under this name.
    #[serde(default)]
    pub save_as: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetVariableParams {
    /// The name, used in requests as {{name}}. Letters, digits, _ - and . only, at most 64 characters.
    pub name: String,
    /// The value. For a secret it is stored in the system credential store and never returned.
    pub value: String,
    /// Keep it as a secret: masked in results, and never written to a file. A name that looks like a credential
    /// (token, password, api_key, secret...) is always secret.
    #[serde(default)]
    pub secret: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct NameParams {
    /// The name of a variable an agent set, or of a saved request.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SaveRequestParams {
    /// The name to save it under, shown in Plunger's Saved list.
    pub name: String,
    /// Replace a saved request that already has this name (otherwise an existing name is refused).
    #[serde(default)]
    pub overwrite: Option<bool>,
    /// The request, described like send_request does: method, url, headers, json / body / form, options.
    #[serde(flatten)]
    pub request: SendParams,
}

#[derive(Serialize, JsonSchema)]
pub struct DeletedVariable {
    /// The variable that was removed.
    pub deleted: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct HistoryParams {
    /// How many recent requests to return, newest first (default 20, at most 200).
    #[serde(default)]
    pub limit: Option<i64>,
    /// Only requests whose URL, method, name or status contains this text (case-insensitive), e.g. "orders" or "500".
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ExportCurlParams {
    /// Name of a saved request.
    #[serde(default)]
    pub saved_request: Option<String>,
    /// Or an id from get_history.
    #[serde(default)]
    pub history_id: Option<i64>,
}

/// Runs blocking work (network, SQLite, credential store) off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("Internal error: {e}"))?
}

#[tool_router(router = tool_router)]
impl PlungerMcp {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }

    /// Send an HTTP request through Plunger and get back status, timing, size, headers and the parsed JSON body.
    /// Use this instead of curl for API calls: {{variables}} (including the user's secrets, by name) are filled in
    /// for you, an undefined {{variable}} makes the request fail safely instead of sending the placeholder, secret
    /// values are masked in the result, and the request is recorded in the history the user sees in Plunger.
    /// Send a saved request by name with `saved_request`, or describe one with method, url, headers and a body.
    #[tool(name = "send_request", annotations(title = "Send an HTTP request", open_world_hint = true))]
    async fn send_request(&self, Parameters(params): Parameters<SendParams>) -> Result<Json<AgentResponse>, String> {
        blocking(move || agent::send_request(&params, Source::Mcp).map_err(|e| e.to_string())).await.map(Json)
    }

    /// Parse a curl command into a Plunger request without sending it: method, URL, headers, body and the
    /// {{variables}} it uses. Use it to check what a curl command would do, or with `save_as` to keep it in the
    /// user's Saved list so it can be sent later with send_request by name.
    #[tool(name = "import_curl", annotations(title = "Import a curl command"))]
    async fn import_curl(&self, Parameters(p): Parameters<ImportCurlParams>) -> Result<Json<ImportedRequest>, String> {
        blocking(move || agent::import_curl(&p.curl, p.save_as.as_deref(), Source::Mcp)).await.map(Json)
    }

    /// List the requests the user saved in Plunger, by name, with method, URL, headers and the {{variables}} each
    /// needs. Call this before building a request from scratch: the user may already have the exact call set up.
    #[tool(name = "list_saved_requests", annotations(title = "List saved requests", read_only_hint = true))]
    async fn list_saved_requests(&self) -> Result<Json<Vec<StoredRequestInfo>>, String> {
        blocking(agent::list_saved_requests).await.map(Json)
    }

    /// Recent requests from the shared history, newest first: who sent them (gui = the user, cli or mcp = an
    /// agent), method, URL, status and time. Credentials are already blanked. Use it to see what the user tried,
    /// or to check what was sent earlier.
    #[tool(name = "get_history", annotations(title = "Get request history", read_only_hint = true))]
    async fn get_history(&self, Parameters(p): Parameters<HistoryParams>) -> Result<Json<Vec<HistoryItem>>, String> {
        blocking(move || agent::get_history(p.limit, p.search.as_deref())).await.map(Json)
    }

    /// List the {{variables}} defined in Plunger. Secret variables are listed by name only (their values are
    /// never returned); use them in a request as {{name}} and Plunger fills them in.
    #[tool(name = "list_variables", annotations(title = "List variables", read_only_hint = true))]
    async fn list_variables(&self) -> Result<Json<VariablesResult>, String> {
        blocking(|| Ok(agent::list_variables())).await.map(Json)
    }

    /// Set a variable that later requests use as {{name}}, for example a token you read from a login response.
    /// It persists between calls and runs and is shared with the Plunger window, the CLI and other agents.
    /// A secret (or a credential-looking name such as token or api_key) is kept in the system credential store and
    /// masked in results. A variable the user defined in the window cannot be changed.
    #[tool(name = "set_variable", annotations(title = "Set a variable", idempotent_hint = true, destructive_hint = false))]
    async fn set_variable(&self, Parameters(p): Parameters<SetVariableParams>) -> Result<Json<VariableInfo>, String> {
        blocking(move || agent::set_variable(&p.name, &p.value, p.secret, Source::Mcp)).await.map(Json)
    }

    /// Remove a variable an agent set (with set_variable). Variables the user defined in the window can only be
    /// removed there.
    #[tool(name = "delete_variable", annotations(title = "Delete a variable", destructive_hint = true, idempotent_hint = true))]
    async fn delete_variable(&self, Parameters(p): Parameters<NameParams>) -> Result<Json<DeletedVariable>, String> {
        blocking(move || agent::delete_variable(&p.name).map(|()| DeletedVariable { deleted: p.name.trim().to_string() })).await.map(Json)
    }

    /// Save a request in the user's Saved list without sending it, so it can be re-run by name with send_request
    /// (`saved_request`). Describe it like send_request. {{placeholders}} are kept as written, including in
    /// Authorization headers. An existing name is refused unless `overwrite` is true, which replaces it, so a
    /// mistake can be fixed.
    #[tool(name = "save_request", annotations(title = "Save a request", idempotent_hint = true, destructive_hint = false))]
    async fn save_request(&self, Parameters(p): Parameters<SaveRequestParams>) -> Result<Json<ImportedRequest>, String> {
        blocking(move || agent::save_request(&p.name, p.overwrite.unwrap_or(false), &p.request, Source::Mcp)).await.map(Json)
    }

    /// One saved request in full: method, URL, headers, body, and the {{variables}} it needs.
    #[tool(name = "get_saved_request", annotations(title = "Get a saved request", read_only_hint = true))]
    async fn get_saved_request(&self, Parameters(p): Parameters<NameParams>) -> Result<Json<ImportedRequest>, String> {
        blocking(move || agent::show_saved_request(&p.name)).await.map(Json)
    }

    /// Remove a saved request by name (its history entries stay). Returns what was removed.
    #[tool(name = "delete_saved_request", annotations(title = "Delete a saved request", destructive_hint = true))]
    async fn delete_saved_request(&self, Parameters(p): Parameters<NameParams>) -> Result<Json<ImportedRequest>, String> {
        blocking(move || agent::delete_saved_request(&p.name)).await.map(Json)
    }

    /// Turn a saved request (by name) or a history entry (by id) into a curl command, for CI scripts or tools
    /// that only speak curl. {{variables}} stay as placeholders, so no secret is written out.
    #[tool(name = "export_curl", annotations(title = "Export as curl", read_only_hint = true))]
    async fn export_curl(&self, Parameters(p): Parameters<ExportCurlParams>) -> Result<String, String> {
        blocking(move || agent::export_curl(p.saved_request.as_deref(), p.history_id)).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PlungerMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("plunger", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Serves MCP on stdin/stdout until the client disconnects.
pub fn run() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let service = PlungerMcp::new()
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| format!("MCP startup failed: {e}"))?;
        service.waiting().await.map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_listed_with_input_schemas_and_hints() {
        let tools = PlungerMcp::new().tool_router.list_all();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "delete_saved_request", "delete_variable", "export_curl", "get_history", "get_saved_request", "import_curl",
                "list_saved_requests", "list_variables", "save_request", "send_request", "set_variable"
            ]
        );
        let send = tools.iter().find(|t| t.name == "send_request").unwrap();
        let schema = serde_json::to_string(&send.input_schema).unwrap();
        for field in ["saved_request", "url", "headers", "json", "variables", "use_saved_bearer"] {
            assert!(schema.contains(field), "send_request schema lacks {field}: {schema}");
        }
        assert!(send.description.as_deref().unwrap_or("").contains("instead of curl"));
        assert!(send.output_schema.is_some());
        let read_only = tools.iter().find(|t| t.name == "get_history").unwrap().annotations.clone().unwrap();
        assert_eq!(read_only.read_only_hint, Some(true));
    }

    #[test]
    fn the_instructions_mention_every_tool() {
        for tool in PlungerMcp::new().tool_router.list_all() {
            assert!(INSTRUCTIONS.contains(tool.name.as_ref()), "INSTRUCTIONS does not mention {}", tool.name);
        }
    }

    /// Keys the tool's advertised output schema requires but the serialized output leaves out.
    /// A strict MCP client rejects such a result ("Structured content does not match the tool's
    /// output schema"), so this must stay empty even when every optional list is empty.
    fn missing_required_keys<T: JsonSchema + serde::Serialize>(output: &T) -> Vec<String> {
        let schema = serde_json::to_value(rmcp::schemars::schema_for!(T)).unwrap();
        let value = serde_json::to_value(output).unwrap();
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        required.iter().filter_map(|k| k.as_str()).filter(|k| value.get(*k).is_none()).map(String::from).collect()
    }

    #[test]
    fn structured_output_has_every_key_its_schema_requires() {
        use crate::engine::{AgentResponse, SentRequest};
        let response = AgentResponse {
            ok: true,
            status: 200,
            status_text: "OK".into(),
            elapsed_ms: 1,
            size_bytes: 0,
            json: None,
            body: None,
            body_cut_from_chars: None,
            truncated_at_10mb: false,
            binary: false,
            headers: vec![],
            request: SentRequest { method: "GET".into(), url: "http://h/".into(), history_id: None },
            redacted: vec![],
        };
        assert_eq!(missing_required_keys(&response), Vec::<String>::new(), "send_request");

        let imported = crate::agent::import_curl("curl http://h/x", None, Source::Mcp).unwrap();
        assert!(imported.form_fields.is_empty());
        assert_eq!(missing_required_keys(&imported), Vec::<String>::new(), "import_curl");

        let variables = crate::agent::VariablesResult {
            variables: vec![],
            built_in: vec![],
            saved_bearer_available: false,
            problems: vec![],
        };
        assert_eq!(missing_required_keys(&variables), Vec::<String>::new(), "list_variables");
    }
}
