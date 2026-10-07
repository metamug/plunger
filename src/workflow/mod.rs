//! Workflows: a saved, ordered list of requests where a later one can use values from an earlier
//! response. A step sends a request (inline or a saved one), then `extract`s values from the
//! response into variables (`{{name}}`) for the steps after it. A value named like a credential
//! (`token`, `password`...) is kept as a secret: it is used in later requests but never shown.

mod extract;

use crate::agent::{self, SendFailure, SendParams};
use crate::engine::{AgentResponse, VariableInfo};
use crate::history::Source;
use crate::model::ResponseData;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

const MAX_STEPS: usize = 50;

/// A value to take from a step's response and keep as a variable.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Extract {
    /// The variable to set, used in later steps as {{name}}.
    pub name: String,
    /// Where to read it: `json:$.data.token` (a JSON path), `header:Name`, or `status`.
    pub from: String,
    /// Keep it as a secret (masked, in the credential store). A name like token or password is always secret.
    #[serde(default)]
    pub secret: Option<bool>,
}

/// One request in a workflow.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Step {
    /// A label shown in the results, e.g. "log in".
    #[serde(default)]
    pub label: Option<String>,
    /// The request, described like send_request does: saved_request, method, url, headers, json / body / form, options.
    #[serde(flatten)]
    pub request: SendParams,
    /// Values to keep from the response for the steps after this one.
    #[serde(default)]
    pub extract: Vec<Extract>,
    /// The status this step must return. Without it any 2xx passes; a step that fails stops the workflow.
    #[serde(default)]
    pub expect_status: Option<u16>,
}

/// What one step did.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct StepResult {
    /// 1-based position in the workflow.
    pub step: usize,
    pub label: String,
    /// False when the step failed: not sent, no response, an unexpected status, or a value that could not be extracted.
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// The variables this step set (secrets by name only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set: Vec<VariableInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct WorkflowResult {
    pub workflow: String,
    /// True when every step passed.
    pub ok: bool,
    /// How many steps ran (a failed step stops the rest).
    pub steps_run: usize,
    pub steps: Vec<StepResult>,
    /// The last response that came back, in full.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_response: Option<AgentResponse>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct WorkflowInfo {
    pub name: String,
    pub steps: Vec<Step>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct WorkflowList {
    pub workflows: Vec<WorkflowInfo>,
}

fn check(steps: &[Step]) -> Result<(), String> {
    if steps.is_empty() {
        return Err("A workflow needs at least one step.".into());
    }
    if steps.len() > MAX_STEPS {
        return Err(format!("A workflow can have at most {MAX_STEPS} steps."));
    }
    for (i, step) in steps.iter().enumerate() {
        if step.request.saved_request.is_none() && step.request.url.as_deref().is_none_or(|u| u.trim().is_empty()) {
            return Err(format!("Step {} needs a `url` or a `saved_request`.", i + 1));
        }
        for e in &step.extract {
            if e.name.trim().is_empty() || e.from.trim().is_empty() {
                return Err(format!("Step {}: every `extract` needs a `name` and a `from`.", i + 1));
            }
        }
    }
    Ok(())
}

fn valid_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err("A workflow needs a name (at most 100 characters).".into());
    }
    Ok(name)
}

/// Saves a workflow under `name`. An existing name is refused unless `overwrite` is set.
pub fn save(name: &str, steps: Vec<Step>, overwrite: bool) -> Result<WorkflowInfo, String> {
    let name = valid_name(name)?.to_string();
    check(&steps)?;
    let history = agent::open_history()?;
    if !overwrite && history.get_workflow(&name).map_err(|e| e.to_string())?.is_some() {
        return Err(format!("A workflow named '{name}' already exists. Pass `overwrite: true` to replace it."));
    }
    let json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
    history.save_workflow(&name, &json).map_err(|e| e.to_string())?;
    Ok(WorkflowInfo { name, steps })
}

fn parse_steps(json: &str) -> Result<Vec<Step>, String> {
    serde_json::from_str(json).map_err(|e| format!("The stored workflow is unreadable: {e}"))
}

pub fn get(name: &str) -> Result<WorkflowInfo, String> {
    let history = agent::open_history()?;
    let json = history
        .get_workflow(name.trim())
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No workflow named '{}'. See list_workflows.", name.trim()))?;
    Ok(WorkflowInfo { name: name.trim().to_string(), steps: parse_steps(&json)? })
}

pub fn list() -> Result<WorkflowList, String> {
    let history = agent::open_history()?;
    let mut workflows = Vec::new();
    for (name, json) in history.list_workflows().map_err(|e| e.to_string())? {
        workflows.push(WorkflowInfo { name, steps: parse_steps(&json)? });
    }
    Ok(WorkflowList { workflows })
}

pub fn delete(name: &str) -> Result<WorkflowInfo, String> {
    let info = get(name)?;
    agent::open_history()?.delete_workflow(&info.name).map_err(|e| e.to_string())?;
    Ok(info)
}

/// Runs a workflow by name. `variables` apply to every step, on top of what each step sets.
pub fn run(name: &str, variables: &std::collections::BTreeMap<String, String>, source: Source) -> Result<WorkflowResult, String> {
    let info = get(name)?;
    Ok(run_steps(
        &info.name,
        &info.steps,
        variables,
        |params| agent::send_request_raw(params, source),
        |name, value, secret| agent::set_variable(name, value, secret, source),
    ))
}

/// The runner, with sending and variable-setting passed in so it can be tested without a network.
fn run_steps(
    workflow: &str,
    steps: &[Step],
    variables: &std::collections::BTreeMap<String, String>,
    mut send: impl FnMut(&SendParams) -> Result<(AgentResponse, ResponseData), SendFailure>,
    mut set: impl FnMut(&str, &str, Option<bool>) -> Result<VariableInfo, String>,
) -> WorkflowResult {
    let mut results: Vec<StepResult> = Vec::new();
    let mut last_response = None;
    let mut ok = true;
    for (i, step) in steps.iter().enumerate() {
        let label = step.label.clone().unwrap_or_else(|| {
            step.request.saved_request.clone().unwrap_or_else(|| format!("{} {}", step.request.method.as_deref().unwrap_or("GET"), step.request.url.as_deref().unwrap_or("")))
        });
        let mut result = StepResult { step: i + 1, label, ok: false, status: None, elapsed_ms: None, set: Vec::new(), error: None };

        let mut params = step.request.clone();
        if !variables.is_empty() {
            let mut merged = params.variables.take().unwrap_or_default();
            merged.extend(variables.iter().map(|(k, v)| (k.clone(), v.clone())));
            params.variables = Some(merged);
        }

        match send(&params) {
            Err(failure) => result.error = Some(failure.to_string()),
            Ok((shaped, raw)) => {
                result.status = Some(shaped.status);
                result.elapsed_ms = Some(shaped.elapsed_ms);
                let passed = match step.expect_status {
                    Some(want) => shaped.status == want,
                    None => (200..300).contains(&shaped.status),
                };
                if !passed {
                    let want = step.expect_status.map(|w| w.to_string()).unwrap_or_else(|| "2xx".into());
                    result.error = Some(format!("expected {want}, got {}", shaped.status));
                } else {
                    result.ok = true;
                    for e in &step.extract {
                        let stored = extract::extract(&raw, &e.from).and_then(|value| set(e.name.trim(), &value, e.secret));
                        match stored {
                            Ok(info) => result.set.push(info),
                            Err(err) => {
                                result.ok = false;
                                result.error = Some(format!("could not set `{}` from `{}`: {err}", e.name, e.from));
                                break;
                            }
                        }
                    }
                }
                last_response = Some(shaped);
            }
        }

        let stop = !result.ok;
        results.push(result);
        if stop {
            ok = false;
            break;
        }
    }
    WorkflowResult { workflow: workflow.to_string(), ok, steps_run: results.len(), steps: results, last_response }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn step(url: &str) -> Step {
        Step { label: None, request: SendParams { url: Some(url.into()), ..Default::default() }, extract: vec![], expect_status: None }
    }

    fn reply(status: u16, body: &str) -> (AgentResponse, ResponseData) {
        let raw = ResponseData { status, body: body.into(), ..Default::default() };
        let shaped = AgentResponse {
            ok: (200..300).contains(&status),
            status,
            status_text: String::new(),
            elapsed_ms: 3,
            size_bytes: body.len(),
            json: None,
            body: None,
            body_cut_from_chars: None,
            truncated_at_10mb: false,
            binary: false,
            headers: vec![],
            request: crate::engine::SentRequest { method: "GET".into(), url: "http://x".into(), history_id: None },
            redacted: vec![],
        };
        (shaped, raw)
    }

    fn info(name: &str) -> VariableInfo {
        VariableInfo { name: name.into(), secret: false, has_value: true, remembered: false, value: None, source: "agent".into() }
    }

    #[test]
    fn a_value_from_one_response_is_set_for_the_next_request() {
        let mut login = step("http://h/login");
        login.extract = vec![Extract { name: "token".into(), from: "json:$.token".into(), secret: None }];
        let steps = vec![login, step("http://h/me")];
        let mut set_calls: Vec<(String, String)> = Vec::new();
        let mut sent = Vec::new();
        let result = run_steps(
            "auth",
            &steps,
            &BTreeMap::new(),
            |p| {
                sent.push(p.url.clone().unwrap());
                Ok(reply(200, r#"{"token": "abc123"}"#))
            },
            |n, v, _| {
                set_calls.push((n.into(), v.into()));
                Ok(info(n))
            },
        );
        assert!(result.ok);
        assert_eq!(result.steps_run, 2);
        assert_eq!(set_calls, vec![("token".to_string(), "abc123".to_string())]);
        assert_eq!(sent, vec!["http://h/login", "http://h/me"]);
        assert_eq!(result.steps[0].set[0].name, "token");
    }

    #[test]
    fn a_failing_step_stops_the_workflow() {
        let steps = vec![step("http://h/a"), step("http://h/b"), step("http://h/c")];
        let mut n = 0;
        let result = run_steps("w", &steps, &BTreeMap::new(), |_| {
            n += 1;
            Ok(reply(if n == 2 { 500 } else { 200 }, "{}"))
        }, |n, _, _| Ok(info(n)));
        assert!(!result.ok);
        assert_eq!(result.steps_run, 2, "the third step never ran");
        assert_eq!(result.steps[1].error.as_deref(), Some("expected 2xx, got 500"));
        assert_eq!(result.last_response.unwrap().status, 500);
    }

    #[test]
    fn expect_status_replaces_the_2xx_rule() {
        let mut s = step("http://h/missing");
        s.expect_status = Some(404);
        let result = run_steps("w", &[s], &BTreeMap::new(), |_| Ok(reply(404, "{}")), |n, _, _| Ok(info(n)));
        assert!(result.ok);
    }

    #[test]
    fn a_value_that_is_not_there_fails_the_step_and_says_why() {
        let mut s = step("http://h/a");
        s.extract = vec![Extract { name: "id".into(), from: "json:$.data.id".into(), secret: None }];
        let result = run_steps("w", &[s], &BTreeMap::new(), |_| Ok(reply(200, r#"{"data": {}}"#)), |n, _, _| Ok(info(n)));
        assert!(!result.ok);
        let error = result.steps[0].error.as_deref().unwrap();
        assert!(error.contains("could not set `id`") && error.contains("no `id`"), "{error}");
    }

    #[test]
    fn run_variables_reach_every_step() {
        let mut seen = Vec::new();
        let vars = BTreeMap::from([("user".to_string(), "ann".to_string())]);
        run_steps("w", &[step("http://h/a"), step("http://h/b")], &vars, |p| {
            seen.push(p.variables.clone().unwrap());
            Ok(reply(200, "{}"))
        }, |n, _, _| Ok(info(n)));
        assert!(seen.iter().all(|v| v.get("user").map(String::as_str) == Some("ann")));
    }

    #[test]
    fn a_workflow_without_a_request_is_refused() {
        assert!(check(&[]).is_err());
        let empty = Step { label: None, request: SendParams::default(), extract: vec![], expect_status: None };
        assert!(check(&[empty]).unwrap_err().contains("Step 1"));
        assert!(check(&[step("http://h")]).is_ok());
    }

    #[test]
    fn steps_round_trip_as_json_with_the_request_fields_inline() {
        let json = r#"[{"label":"log in","method":"POST","url":"http://h/login","json":{"u":"a"},"extract":[{"name":"token","from":"json:$.token"}],"expect_status":200}]"#;
        let steps = parse_steps(json).unwrap();
        assert_eq!(steps[0].request.method.as_deref(), Some("POST"));
        assert_eq!(steps[0].extract[0].from, "json:$.token");
        assert_eq!(steps[0].expect_status, Some(200));
        let again = parse_steps(&serde_json::to_string(&steps).unwrap()).unwrap();
        assert_eq!(again[0].request.url.as_deref(), Some("http://h/login"));
    }
}
