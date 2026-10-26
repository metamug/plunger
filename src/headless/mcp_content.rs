//! What the MCP server offers besides tools: resources an agent can read (variables, saved
//! requests, workflows, history, a guide) and prompts that start a common job. They are plain
//! data and text, so they can be tested without a client; `mcp` adapts them to the protocol.

use crate::{agent, workflow};

pub const GUIDE_URI: &str = "plunger://guide";
const VARIABLES_URI: &str = "plunger://variables";
const SAVED_URI: &str = "plunger://saved-requests";
const WORKFLOWS_URI: &str = "plunger://workflows";
const HISTORY_URI: &str = "plunger://history";
pub const SCRIPTING_URI: &str = "plunger://scripting";
/// The scripting guide: the same page as `docs/scripting.md`, so the docs and what an agent reads cannot differ.
pub const SCRIPTING_GUIDE: &str = include_str!("../../docs/scripting.md");
const SAVED_PREFIX: &str = "plunger://saved-requests/";
const WORKFLOW_PREFIX: &str = "plunger://workflows/";

/// A resource that always exists.
pub struct Listed {
    pub uri: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub mime: &'static str,
}

pub const RESOURCES: &[Listed] = &[
    Listed { uri: GUIDE_URI, name: "guide", description: "How to use Plunger from an agent: the tools, how variables and secrets behave, workflows.", mime: "text/markdown" },
    Listed { uri: VARIABLES_URI, name: "variables", description: "The {{variables}} in use (secrets by name only) and the built-ins.", mime: "application/json" },
    Listed { uri: SAVED_URI, name: "saved-requests", description: "The requests the user saved, with method, URL, headers and the variables each needs.", mime: "application/json" },
    Listed { uri: WORKFLOWS_URI, name: "workflows", description: "Saved workflows: ordered requests where one response feeds the next.", mime: "application/json" },
    Listed { uri: SCRIPTING_URI, name: "scripting", description: "How to do many requests, paging, loops, waiting and a short answer with workflow steps (paginate, for_each, poll, retry, collect, return), with recipes.", mime: "text/markdown" },
    Listed { uri: HISTORY_URI, name: "history", description: "The 20 most recent requests, who sent them, status and time.", mime: "application/json" },
];

/// Resources that take a name: (URI template, name, description).
pub const TEMPLATES: &[(&str, &str, &str)] = &[
    ("plunger://saved-requests/{name}", "saved-request", "One saved request in full (URL-encode the name)."),
    ("plunger://workflows/{name}", "workflow", "One workflow with its steps (URL-encode the name)."),
];

fn json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| e.to_string())
}

/// Percent-encodes a name for use in a resource URI.
pub fn encode_name(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode_name(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3).ok_or("bad %-escape in the resource URI")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| "bad %-escape in the resource URI")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "the resource URI is not valid text".to_string())
}

/// The text of a resource, or why there is none.
pub fn read(uri: &str, guide: &str) -> Result<String, String> {
    match uri {
        GUIDE_URI => Ok(guide.to_string()),
        SCRIPTING_URI => Ok(SCRIPTING_GUIDE.to_string()),
        VARIABLES_URI => json(&agent::list_variables()),
        SAVED_URI => json(&agent::list_saved_requests(None)?),
        WORKFLOWS_URI => json(&workflow::list(None)?),
        HISTORY_URI => json(&agent::get_history(Some(20), None)?),
        _ => {
            if let Some(name) = uri.strip_prefix(SAVED_PREFIX) {
                return json(&agent::show_saved_request(&decode_name(name)?)?);
            }
            if let Some(name) = uri.strip_prefix(WORKFLOW_PREFIX) {
                return json(&workflow::get(&decode_name(name)?)?);
            }
            Err(format!("No resource {uri}"))
        }
    }
}

/// The saved requests and workflows, as resources of their own (so a client can list and open them).
pub fn dynamic() -> Vec<(String, String, &'static str)> {
    let mut out = Vec::new();
    if let Ok(saved) = agent::list_saved_requests(None) {
        for s in saved.into_iter().filter(|s| s.name.is_some()).take(200) {
            let name = s.name.clone().unwrap_or_default();
            out.push((format!("{SAVED_PREFIX}{}", encode_name(&name)), format!("saved request: {name}"), "application/json"));
        }
    }
    if let Ok(list) = workflow::list(None) {
        for w in list.workflows {
            out.push((format!("{WORKFLOW_PREFIX}{}", encode_name(&w.name)), format!("workflow: {}", w.name), "application/json"));
        }
    }
    out
}

/// A prompt an agent's user can pick: its name, what it does and its arguments (name, description, required).
pub struct PromptInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub arguments: &'static [(&'static str, &'static str, bool)],
}

pub const PROMPTS: &[PromptInfo] = &[
    PromptInfo {
        name: "test_endpoint",
        description: "Call an endpoint with Plunger and check the response against what you expect.",
        arguments: &[("url", "The endpoint to call", true), ("expect", "What a correct response looks like, e.g. 'status 200 and a list of orders'", false)],
    },
    PromptInfo {
        name: "login_workflow",
        description: "Build and save a workflow that logs in and keeps the token for later requests.",
        arguments: &[
            ("login_url", "The URL that returns a token", true),
            ("token_path", "Where the token is in the response, e.g. $.access_token (default $.token)", false),
            ("username_variable", "Name of the variable holding the username (default username)", false),
            ("password_variable", "Name of the variable holding the password (default password)", false),
        ],
    },
    PromptInfo {
        name: "debug_failed_request",
        description: "Find out why a request failed, using the history and saved requests.",
        arguments: &[("search", "Part of the URL, or a status like 500, to find the request", false)],
    },
    PromptInfo {
        name: "build_script",
        description: "Build a reusable workflow that pages, loops or waits and returns a short answer (a report, a count, a list of ids), instead of writing a script.",
        arguments: &[
            ("goal", "What the workflow should find out or do, e.g. 'points per assignee for sprint 12'", true),
            ("api", "Which API and where, e.g. 'Jira at https://example.atlassian.net'", false),
        ],
    },
    PromptInfo {
        name: "record_workflow",
        description: "Turn the requests you just made into a reusable workflow.",
        arguments: &[("name", "A name for the workflow", true), ("goal", "What the workflow achieves", false)],
    },
];

fn arg<'a>(args: &'a serde_json::Map<String, serde_json::Value>, name: &str) -> Option<&'a str> {
    args.get(name).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

/// The text of a prompt for the given arguments.
pub fn prompt(name: &str, args: &serde_json::Map<String, serde_json::Value>) -> Result<(String, String), String> {
    let info = PROMPTS.iter().find(|p| p.name == name).ok_or_else(|| format!("No prompt named '{name}'."))?;
    for (arg_name, _, required) in info.arguments {
        if *required && arg(args, arg_name).is_none() {
            return Err(format!("The prompt '{name}' needs the argument `{arg_name}`."));
        }
    }
    let text = match name {
        "test_endpoint" => {
            let url = arg(args, "url").unwrap_or_default();
            let expect = arg(args, "expect").unwrap_or("a successful response");
            format!(
                "Use Plunger (not curl) to test {url}.\n\
                 1. Call list_saved_requests and list_variables: the user may already have this request and its variables.\n\
                 2. Send it with send_request. Use {{{{variables}}}} for anything secret instead of typing the value.\n\
                 3. Check the response against: {expect}. Look at status, `json`, headers and elapsed_ms.\n\
                 4. Report what you sent, what came back, and whether it matches. If it failed, say why (an undefined variable, a 401, a timeout...)."
            )
        }
        "login_workflow" => {
            let login = arg(args, "login_url").unwrap_or_default();
            let path = arg(args, "token_path").unwrap_or("$.token");
            let user = arg(args, "username_variable").unwrap_or("username");
            let pass = arg(args, "password_variable").unwrap_or("password");
            format!(
                "Build a Plunger workflow that logs in and keeps the token.\n\
                 1. list_variables: check that `{user}` and `{pass}` exist. If not, ask the user to add them in the Plunger window (secrets must not pass through chat).\n\
                 2. Call save_workflow with name \"login\" and a first step: POST {login} with a json body using {{{{{user}}}}} and {{{{{pass}}}}}, extract [{{name: \"token\", from: \"json:{path}\"}}], expect_status 200.\n\
                 3. Add the requests that need the token, with the header Authorization: Bearer {{{{token}}}}.\n\
                 4. Run it with run_workflow and report each step's status. The token is stored as a hidden secret: never print it."
            )
        }
        "debug_failed_request" => {
            let search = arg(args, "search").map(|s| format!(" with search \"{s}\"")).unwrap_or_default();
            format!(
                "Find out why a request failed, using Plunger.\n\
                 1. Call get_history{search} and pick the failing request (status 4xx/5xx, or no status).\n\
                 2. Read its headers and body in the history entry; call get_saved_request if it was saved.\n\
                 3. Re-send it with send_request, changing one thing at a time (a header, a variable, the body).\n\
                 4. Explain the cause and the smallest fix. Do not print secret values; they are shown as [redacted:name]."
            )
        }
        "build_script" => {
            let goal = arg(args, "goal").unwrap_or_default();
            let api = arg(args, "api").map(|a| format!(" The API: {a}.")).unwrap_or_default();
            format!(
                "Use Plunger to build a reusable workflow that does this: {goal}.{api}
                 1. Read the resource plunger://scripting: it lists the step fields (paginate, for_each, poll, retry, collect, return) with recipes.
                 2. Call list_saved_requests and list_variables: the request, its base URL and its auth may already exist. Use {{{{variables}}}} for anything secret; if a credential is missing, ask the user to add it in the Plunger window.
                 3. Send ONE request with send_request first and look at its shape (use `select` or the outline), so the paths in `paginate.items`, `total`, `next` and `return` are right.
                 4. Call save_workflow: a step that fetches (with `paginate` and/or `for_each`, `retry` if the API rate-limits) and `collect`s, then a last step with only a `return` that boils the data down to the answer. The workflow is checked as it is saved; fix what it reports.
                 5. Run it with run_workflow and read `output`. If a step failed, read its `error` and `warning`, fix that step and run again. Tell the user the answer and how to run it again (workflow name and the `variables` it takes).
                 Prefer this to writing a script: you read only the answer, and secrets never pass through you. Write a script only for something these steps cannot do."
            )
        }
        "record_workflow" => {
            let wf = arg(args, "name").unwrap_or_default();
            let goal = arg(args, "goal").map(|g| format!(" It should {g}.")).unwrap_or_default();
            format!(
                "Turn the requests you just made with Plunger into the workflow \"{wf}\".{goal}\n\
                 1. Call get_history to see them in order.\n\
                 2. Call save_workflow: one step per request, with `extract` for each value a later request needs \
                 (json:$.path, header:Name or status) and {{{{name}}}} where it is used.\n\
                 3. Run it with run_workflow to check it works end to end, and fix any step that fails."
            )
        }
        _ => unreachable!("checked above"),
    };
    Ok((info.description.to_string(), text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn names_survive_a_round_trip_through_a_uri() {
        for name in ["login", "my request/2", "ünï & co", "100%"] {
            assert_eq!(decode_name(&encode_name(name)).unwrap(), name);
        }
        assert!(decode_name("%zz").is_err());
        assert!(decode_name("%4").is_err());
    }

    #[test]
    fn the_guide_and_unknown_uris_are_handled_without_the_database() {
        assert_eq!(read(GUIDE_URI, "hello").unwrap(), "hello");
        assert!(read("plunger://nope", "").unwrap_err().contains("No resource"));
    }

    #[test]
    fn every_prompt_renders_and_names_the_tools_it_uses() {
        for p in PROMPTS {
            let mut a = serde_json::Map::new();
            for (n, _, required) in p.arguments {
                if *required {
                    a.insert(n.to_string(), json!("x"));
                }
            }
            let (_, text) = prompt(p.name, &a).unwrap();
            assert!(text.contains("Plunger"), "{} does not mention Plunger", p.name);
        }
    }

    #[test]
    fn the_scripting_guide_is_a_resource_and_the_build_prompt_points_to_it() {
        assert!(RESOURCES.iter().any(|r| r.uri == SCRIPTING_URI));
        let guide = read(SCRIPTING_URI, "").unwrap();
        for needle in ["paginate", "for_each", "poll", "retry", "collect", "return", "asserts", "plunger run"] {
            assert!(guide.contains(needle), "the scripting guide does not mention {needle}");
        }
        let (_, text) = prompt("build_script", &args(json!({"goal": "points per assignee", "api": "Jira at https://j.example"}))).unwrap();
        assert!(text.contains("plunger://scripting") && text.contains("points per assignee") && text.contains("Jira at https://j.example"), "{text}");
        assert!(prompt("build_script", &args(json!({}))).unwrap_err().contains("`goal`"));
    }

    #[test]
    fn a_missing_required_argument_is_named() {
        let err = prompt("test_endpoint", &args(json!({}))).unwrap_err();
        assert!(err.contains("`url`"), "{err}");
        assert!(prompt("nope", &args(json!({}))).is_err());
    }

    #[test]
    fn the_login_prompt_uses_the_given_names_and_never_asks_for_a_password_in_chat() {
        let (_, text) = prompt("login_workflow", &args(json!({"login_url": "http://h/login", "token_path": "$.data.jwt", "username_variable": "user"}))).unwrap();
        assert!(text.contains("http://h/login") && text.contains("json:$.data.jwt") && text.contains("{{user}}") && text.contains("{{password}}"), "{text}");
        assert!(text.contains("secrets must not pass through chat"));
    }

    #[test]
    fn prompts_only_use_tools_that_exist() {
        let tools = ["list_saved_requests", "list_variables", "send_request", "save_workflow", "run_workflow", "get_history", "get_saved_request"];
        let all: String = PROMPTS
            .iter()
            .map(|p| {
                let mut a = serde_json::Map::new();
                for (n, _, _) in p.arguments {
                    a.insert(n.to_string(), json!("x"));
                }
                prompt(p.name, &a).unwrap().1
            })
            .collect();
        for word in all.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if word.starts_with("save_") || word.starts_with("run_") || word.starts_with("list_") || word.starts_with("get_") {
                assert!(tools.contains(&word), "a prompt mentions a tool that does not exist: {word}");
            }
        }
    }
}
