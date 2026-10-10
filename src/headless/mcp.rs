//! `plunger mcp`: a Model Context Protocol server over stdio, so an agent can
//! send HTTP requests through Plunger instead of running curl. Each tool is a
//! thin wrapper over `agent`, the same code the command line uses.
//!
//! Tool descriptions are what the agent's model reads when choosing a tool,
//! so they say when to reach for Plunger rather than raw curl.

use crate::agent::{self, ImportedRequest, SendParams, VariablesResult};
use crate::engine::{AgentResponse, HistoryItem, StoredRequestInfo, VariableInfo};
use crate::store::history::Source;
use crate::headless::mcp_content;
use crate::workflow::{self, Step, WorkflowInfo, WorkflowList, WorkflowResult};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    GetPromptRequestParams, GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, PaginatedRequestParams, Prompt, PromptArgument, PromptMessage, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, Role, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer};
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
- send_request: send a request, or a saved one by name (`saved_request`) with overrides. Use `json`, `body` or `form` for the body; `headers` is an object like {\"Accept\": \"application/json\"}. `select: [\"$.data[0].id\", \"header:Location\"]` returns only those values (under `selected`) instead of the whole body, so a big response costs a few tokens. `extract: [{\"name\": \"token\", \"from\": \"json:$.access_token\"}]` keeps a value from the response as {{token}} for later requests in the same call (hidden if it looks like a credential); `variables_set` lists what was set and `problems` anything that did not work. `request.sent_at` is when it was fired.
- save_request / get_saved_request / delete_saved_request: save a request without sending it (same fields as send_request, plus `name`; `overwrite: true` replaces an existing one), read one back in full, or remove one. {{placeholders}} are kept, so Authorization: Bearer {{token}} works when it is sent later.
- import_curl: parse a curl command into a request, and with `save_as` keep it in the user's Saved list.
- export_curl: a saved request or a history entry as a curl command.
- get_history: recent requests, who sent them (gui, cli, mcp), status and time. Narrow it with `search`, `status` (401, 4xx, 5xx, ok, fail, error), `min_ms` (the slow ones), `source` and `saved_request` (how one saved request has been doing). get_history_entry: one entry in full by id, with the request as sent.
- save_workflow / run_workflow / list_workflows / delete_workflow: a workflow is an ordered list of steps (each is like send_request, plus `extract` and `expect_status`). `extract` takes a value from the response (`json:$.data.token`, `header:Name` or `status`) and keeps it as a variable for the steps after it, so a login token reaches the next request without you ever seeing it. A step that fails (not 2xx, or not `expect_status`) stops the run. run_workflow takes `variables` that apply to every step.

HOW IT BEHAVES
- {{variables}} work in the URL, headers and body. Secret values are filled in for you and never shown; if a server echoes one back it appears as [redacted:name]. A request with an undefined {{variable}} is refused, not sent.
- `variables` on send_request sets values for that one request only; set_variable keeps them. {{$env:NAME}} reads an environment variable of the Plunger process when the request is sent (a name like API_TOKEN is masked in results); an agent cannot set one, because the environment is fixed when the server starts.
- A saved request keeps its {{placeholders}}, including in credential headers such as Authorization: Bearer {{token}}. Literal credentials are blanked when a request is saved.
- Redirects are followed unless `follow_redirects` is false. `timeout_secs` defaults to the user's setting.
- A response comes back as structured fields: status, timing, size, headers and the parsed JSON (`json`) or text (`body`). A body over `max_body_chars` (default 50000) is cut and marked with `body_cut_from_chars`. A binary body is not returned (`binary: true`, with its size). Set-Cookie and similar response headers are shown as [redacted].
- Failures are reported, not hidden: a request that could not be sent returns an error and is not recorded; one that got no response (DNS, refused, timeout) is recorded in the history.

RESOURCES AND PROMPTS
- Resources (plunger://guide, variables, saved-requests, workflows, history, and one per saved request and workflow) can be read without calling a tool. Prompts (test_endpoint, login_workflow, debug_failed_request, record_workflow) start common jobs.

NOT SUPPORTED YET
- Cookies are not carried from one request to the next, and multipart/form-data file uploads are not possible (`form` is url-encoded only).";

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

#[derive(Deserialize, JsonSchema)]
pub struct SaveWorkflowParams {
    /// The workflow's name.
    pub name: String,
    /// Replace a workflow that already has this name.
    #[serde(default)]
    pub overwrite: Option<bool>,
    /// The requests, in order. Each is like send_request, plus `extract` (values to keep for later steps) and
    /// `expect_status` (default: any 2xx).
    pub steps: Vec<Step>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RunWorkflowParams {
    /// The workflow's name (see list_workflows).
    pub name: String,
    /// {{variable}} values for every step of this run.
    #[serde(default)]
    pub variables: Option<std::collections::BTreeMap<String, String>>,
}

/// A tool's structured result is a JSON object, so lists are wrapped. MCP 2025-06-18 and 2025-11-25
/// require an object (a strict client rejects an array schema); only 2026-07-28 allows other types, and
/// rmcp advertises them whatever version was negotiated (modelcontextprotocol/rust-sdk#1337).
#[derive(Serialize, JsonSchema)]
pub struct SavedRequestsResult {
    /// The saved requests, by name.
    pub requests: Vec<StoredRequestInfo>,
}

#[derive(Serialize, JsonSchema)]
pub struct HistoryResult {
    /// Matching requests, newest first.
    pub history: Vec<HistoryItem>,
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
    /// Only this status: `401`, a class like `4xx` or `5xx`, `ok` (2xx), `fail` (4xx, 5xx or no response), or `error` (no response).
    #[serde(default)]
    pub status: Option<String>,
    /// Only requests that took at least this many milliseconds, e.g. 1000 for the slow ones.
    #[serde(default)]
    pub min_ms: Option<i64>,
    /// Only requests sent by `gui` (the user), `cli` or `mcp` (an agent).
    #[serde(default)]
    pub source: Option<String>,
    /// Only sends of this saved request (same method and URL), to see how it has been doing.
    #[serde(default)]
    pub saved_request: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct IdParams {
    /// A history id, from get_history.
    pub id: i64,
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
    async fn list_saved_requests(&self) -> Result<Json<SavedRequestsResult>, String> {
        blocking(agent::list_saved_requests).await.map(|requests| Json(SavedRequestsResult { requests }))
    }

    /// Recent requests from the shared history, newest first: who sent them (gui = the user, cli or mcp = an
    /// agent), method, URL, status and time. Credentials are already blanked. Use it to see what the user tried,
    /// or to check what was sent earlier.
    #[tool(name = "get_history", annotations(title = "Get request history", read_only_hint = true))]
    async fn get_history(&self, Parameters(p): Parameters<HistoryParams>) -> Result<Json<HistoryResult>, String> {
        blocking(move || {
            agent::query_history(&agent::HistoryQuery {
                limit: p.limit,
                search: p.search,
                status: p.status,
                min_ms: p.min_ms,
                source: p.source,
                saved_request: p.saved_request,
            })
        })
        .await
        .map(|history| Json(HistoryResult { history }))
    }

    /// One history entry in full: when it was sent, the status and time, and the request as sent (credentials
    /// blanked, {{placeholders}} kept, so it can be re-sent or saved). The response body is not stored.
    #[tool(name = "get_history_entry", annotations(title = "Get one history entry", read_only_hint = true))]
    async fn get_history_entry(&self, Parameters(p): Parameters<IdParams>) -> Result<Json<agent::HistoryDetail>, String> {
        blocking(move || agent::show_history_entry(p.id)).await.map(Json)
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

    /// Save a workflow: an ordered list of requests where a later one can use values from an earlier response.
    /// Each step is described like send_request, plus `extract` (for example {name: "token", from: "json:$.access_token"},
    /// which keeps the value as {{token}} for the steps after it, hidden if it looks like a credential) and
    /// `expect_status`. Use it for logins, create-then-fetch flows and any sequence you will repeat.
    #[tool(name = "save_workflow", annotations(title = "Save a workflow", idempotent_hint = true, destructive_hint = false))]
    async fn save_workflow(&self, Parameters(p): Parameters<SaveWorkflowParams>) -> Result<Json<WorkflowInfo>, String> {
        blocking(move || workflow::save(&p.name, p.steps, p.overwrite.unwrap_or(false))).await.map(Json)
    }

    /// Run a saved workflow: its requests are sent in order, values are carried from one response to the next
    /// (never shown if secret), and it stops at the first step that fails. Returns each step's status and the
    /// variables it set, plus the last response in full.
    #[tool(name = "run_workflow", annotations(title = "Run a workflow", open_world_hint = true))]
    async fn run_workflow(&self, Parameters(p): Parameters<RunWorkflowParams>) -> Result<Json<WorkflowResult>, String> {
        blocking(move || workflow::run(&p.name, &p.variables.unwrap_or_default(), Source::Mcp)).await.map(Json)
    }

    /// List the saved workflows with their steps.
    #[tool(name = "list_workflows", annotations(title = "List workflows", read_only_hint = true))]
    async fn list_workflows(&self) -> Result<Json<WorkflowList>, String> {
        blocking(workflow::list).await.map(Json)
    }

    /// Remove a workflow by name. Returns what was removed.
    #[tool(name = "delete_workflow", annotations(title = "Delete a workflow", destructive_hint = true))]
    async fn delete_workflow(&self, Parameters(p): Parameters<NameParams>) -> Result<Json<WorkflowInfo>, String> {
        blocking(move || workflow::delete(&p.name)).await.map(Json)
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
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().enable_prompts().build())
            .with_server_info(Implementation::new("plunger", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_resources(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListResourcesResult, ErrorData> {
        let mut resources: Vec<Resource> = mcp_content::RESOURCES
            .iter()
            .map(|r| Resource::new(r.uri, r.name).with_description(r.description).with_mime_type(r.mime))
            .collect();
        let dynamic = tokio::task::spawn_blocking(mcp_content::dynamic).await.unwrap_or_default();
        resources.extend(dynamic.into_iter().map(|(uri, name, mime)| Resource::new(uri, name).with_mime_type(mime)));
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let templates = mcp_content::TEMPLATES
            .iter()
            .map(|(uri, name, description)| ResourceTemplate::new(*uri, *name).with_description(*description).with_mime_type("application/json"))
            .collect();
        Ok(ListResourceTemplatesResult::with_all_items(templates))
    }

    async fn read_resource(&self, request: ReadResourceRequestParams, _: RequestContext<RoleServer>) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri;
        let for_read = uri.clone();
        let text = blocking(move || mcp_content::read(&for_read, INSTRUCTIONS))
            .await
            .map_err(|message| ErrorData::resource_not_found(message, None))?;
        let mime = if uri == mcp_content::GUIDE_URI { "text/markdown" } else { "application/json" };
        Ok(ReadResourceResult::new(vec![ResourceContents::text(text, uri).with_mime_type(mime)]).into())
    }

    async fn list_prompts(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListPromptsResult, ErrorData> {
        let prompts = mcp_content::PROMPTS
            .iter()
            .map(|p| {
                let arguments = p
                    .arguments
                    .iter()
                    .map(|(name, description, required)| PromptArgument::new(*name).with_description(*description).with_required(*required))
                    .collect();
                Prompt::new(p.name, Some(p.description), Some(arguments))
            })
            .collect();
        Ok(ListPromptsResult::with_all_items(prompts))
    }

    async fn get_prompt(&self, request: GetPromptRequestParams, _: RequestContext<RoleServer>) -> Result<GetPromptResponse, ErrorData> {
        let args = request.arguments.unwrap_or_default();
        let (description, text) = mcp_content::prompt(&request.name, &args).map_err(|m| ErrorData::invalid_params(m, None))?;
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)]).with_description(description).into())
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
                "delete_saved_request", "delete_variable", "delete_workflow", "export_curl", "get_history", "get_history_entry", "get_saved_request",
                "import_curl", "list_saved_requests", "list_variables", "list_workflows", "run_workflow", "save_request",
                "save_workflow", "send_request", "set_variable"
            ]
        );
        let send = tools.iter().find(|t| t.name == "send_request").unwrap();
        let schema = serde_json::to_string(&send.input_schema).unwrap();
        for field in ["saved_request", "url", "headers", "json", "variables", "use_saved_bearer"] {
            assert!(schema.contains(field), "send_request schema lacks {field}: {schema}");
        }
        assert!(send.description.as_deref().unwrap_or("").contains("instead of curl"));
        assert!(send.output_schema.is_some());
        // The spec requires a tool's output schema to describe an object; an array breaks strict clients.
        for tool in &tools {
            if let Some(schema) = &tool.output_schema {
                assert_eq!(schema.get("type").and_then(|t| t.as_str()), Some("object"), "{} advertises a non-object output schema", tool.name);
            }
        }
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
            request: SentRequest { method: "GET".into(), url: "http://h/".into(), history_id: None, sent_at: String::new() },
            redacted: vec![],
            selected: None,
            variables_set: vec![],
            problems: vec![],
            outline: None,
            hint: None,
        };
        assert_eq!(missing_required_keys(&response), Vec::<String>::new(), "send_request");

        let imported = crate::agent::import_curl("curl http://h/x", None, Source::Mcp).unwrap();
        assert!(imported.form_fields.is_empty());
        assert_eq!(missing_required_keys(&imported), Vec::<String>::new(), "import_curl");

        assert_eq!(missing_required_keys(&SavedRequestsResult { requests: vec![] }), Vec::<String>::new(), "list_saved_requests");
        assert_eq!(missing_required_keys(&HistoryResult { history: vec![] }), Vec::<String>::new(), "get_history");

        let variables = crate::agent::VariablesResult {
            variables: vec![],
            built_in: vec![],
            saved_bearer_available: false,
            problems: vec![],
        };
        assert_eq!(missing_required_keys(&variables), Vec::<String>::new(), "list_variables");
    }
}
