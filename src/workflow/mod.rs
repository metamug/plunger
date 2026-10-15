//! Workflows: a saved, ordered list of requests where a later one can use values from an earlier
//! response. A step sends a request (inline or a saved one), then `extract`s values from the
//! response into variables (`{{name}}`) for the steps after it. A value named like a credential
//! (`token`, `password`...) is kept as a secret: it is used in later requests but never shown.

pub use crate::domain::extract::{self, Extract};

#[cfg(test)]
use crate::domain::condition::condition_holds;
use crate::agent::{self, SendFailure, SendParams};
use crate::engine::{AgentResponse, VariableInfo};
use crate::store::history::Source;
use crate::domain::model::ResponseData;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

const MAX_STEPS: usize = 50;

/// The statuses a step may return: one, or a list.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum ExpectStatus {
    One(u16),
    Any(Vec<u16>),
}

impl ExpectStatus {
    pub fn matches(&self, status: u16) -> bool {
        match self {
            ExpectStatus::One(want) => *want == status,
            ExpectStatus::Any(list) => list.contains(&status),
        }
    }

    /// "200" or "200, 404".
    pub fn describe(&self) -> String {
        match self {
            ExpectStatus::One(want) => want.to_string(),
            ExpectStatus::Any(list) => list.iter().map(u16::to_string).collect::<Vec<_>>().join(", "),
        }
    }
}

/// One request in a workflow.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct Step {
    /// A label shown in the results, e.g. "log in".
    #[serde(default)]
    pub label: Option<String>,
    /// The request, described like send_request does: saved_request, method, url, headers, json / body / form, options.
    #[serde(flatten)]
    pub request: SendParams,
    /// The status this step must return, or a list of acceptable ones (`[200, 404]`). Without it any 2xx
    /// passes; a step that fails stops the workflow.
    #[serde(default)]
    pub expect_status: Option<ExpectStatus>,
    /// Skip this step (it counts as passed) when the condition holds. A condition is `{{a}} == b`, `{{a}} != b`,
    /// or a single value that is true when it is not empty, `false`, `0` or `null`. A variable with no value is
    /// empty. `{{previous_status}}` is the status of the step before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_if: Option<String>,
    /// End the workflow here, successfully, when the condition holds (same form as `skip_if`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_if: Option<String>,
}

/// What one step did.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct StepResult {
    /// 1-based position in the workflow.
    pub step: usize,
    pub label: String,
    /// False when the step failed: not sent, no response, an unexpected status, or a value that could not be extracted.
    pub ok: bool,
    /// True when `skip_if` held and the step was not sent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
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

/// One step as it is stored with a recorded run: the variables it set by name, never by value.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StoredStep {
    pub step: usize,
    pub label: String,
    pub ok: bool,
    #[serde(default)]
    pub skipped: bool,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub set: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl From<&StepResult> for StoredStep {
    fn from(s: &StepResult) -> Self {
        StoredStep {
            step: s.step,
            label: s.label.clone(),
            ok: s.ok,
            skipped: s.skipped,
            status: s.status,
            elapsed_ms: s.elapsed_ms,
            set: s.set.iter().map(|v| v.name.clone()).collect(),
            error: s.error.clone(),
        }
    }
}

/// What a running workflow reports, so a window can show progress.
#[derive(Debug, Clone)]
pub enum Progress {
    /// Step `step` (1-based) is about to be sent.
    Started { step: usize },
    /// A step is done (passed or failed).
    Finished(StepResult),
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct WorkflowResult {
    pub workflow: String,
    /// True when every step passed.
    pub ok: bool,
    /// True when the run was cancelled before it finished.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    /// With `cleanup`: the variables the run stored and then removed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cleaned_up: Vec<String>,
    /// True when a `stop_if` ended the run before the last step.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped_early: bool,
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
    /// The group it is filed under (`Jira`, `Jira/Issues`), when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
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
        agent::check_reading_instructions(&step.request).map_err(|m| format!("Step {}: {m}", i + 1))?;
        if matches!(&step.expect_status, Some(ExpectStatus::Any(list)) if list.is_empty()) {
            return Err(format!("Step {}: `expect_status` is an empty list.", i + 1));
        }
        for (field, condition) in [("skip_if", &step.skip_if), ("stop_if", &step.stop_if)] {
            if condition.as_deref().is_some_and(|c| c.trim().is_empty()) {
                return Err(format!("Step {}: `{field}` is empty.", i + 1));
            }
        }
        for e in &step.request.extract {
            if let Some(scope) = e.scope.as_deref() {
                if !matches!(scope, "run" | "global") {
                    return Err(format!("Step {}: `scope` of `{}` is `{scope}`; use `run` or `global`.", i + 1, e.name.trim()));
                }
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
pub fn save(name: &str, steps: Vec<Step>, overwrite: bool, group: Option<&str>) -> Result<WorkflowInfo, String> {
    let name = valid_name(name)?.to_string();
    check(&steps)?;
    let group = group.map(crate::store::history::normalize_group).transpose()?;
    let history = agent::open_history()?;
    if !overwrite && history.get_workflow(&name).map_err(|e| e.to_string())?.is_some() {
        return Err(format!("A workflow named '{name}' already exists. Pass `overwrite: true` to replace it."));
    }
    let json = serde_json::to_string(&steps).map_err(|e| e.to_string())?;
    history.save_workflow(&name, &json).map_err(|e| e.to_string())?;
    // None leaves a replaced workflow in its group, "" takes it out
    if let Some(group) = &group {
        history.set_workflow_group(&name, group.as_deref()).map_err(|e| e.to_string())?;
    }
    let group = history.workflow_groups().map_err(|e| e.to_string())?.remove(&name);
    Ok(WorkflowInfo { name, steps, group })
}

pub(crate) fn parse_steps(json: &str) -> Result<Vec<Step>, String> {
    serde_json::from_str(json).map_err(|e| format!("The stored workflow is unreadable: {e}"))
}

pub fn get(name: &str) -> Result<WorkflowInfo, String> {
    let history = agent::open_history()?;
    let json = history
        .get_workflow(name.trim())
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No workflow named '{}'. See list_workflows.", name.trim()))?;
    let group = history.workflow_groups().map_err(|e| e.to_string())?.remove(name.trim());
    Ok(WorkflowInfo { name: name.trim().to_string(), steps: parse_steps(&json)?, group })
}

/// The workflows, optionally only those in `group` (or under it).
pub fn list(group: Option<&str>) -> Result<WorkflowList, String> {
    let history = agent::open_history()?;
    let groups = history.workflow_groups().map_err(|e| e.to_string())?;
    let mut workflows = Vec::new();
    for (name, json) in history.list_workflows().map_err(|e| e.to_string())? {
        let filed = groups.get(&name).cloned();
        if group.is_some_and(|g| !crate::store::history::group_matches(filed.as_deref(), g)) {
            continue;
        }
        workflows.push(WorkflowInfo { name, steps: parse_steps(&json)?, group: filed });
    }
    Ok(WorkflowList { workflows })
}

pub fn delete(name: &str) -> Result<WorkflowInfo, String> {
    let info = get(name)?;
    agent::open_history()?.delete_workflow(&info.name).map_err(|e| e.to_string())?;
    Ok(info)
}

/// Runs a workflow by name. `variables` apply to every step, on top of what each step sets.
/// The run is recorded in the history database (who, when, each step's result). With `cleanup`, the
/// variables the run stored are removed again afterwards (the run's own steps have seen them by then).
pub fn run(
    name: &str,
    variables: &std::collections::BTreeMap<String, String>,
    source: Source,
    cleanup: bool,
) -> Result<WorkflowResult, String> {
    let mut result = run_reporting(name, variables, source, &std::sync::atomic::AtomicBool::new(false), &mut |_| {})?;
    if cleanup {
        let mut removed = Vec::new();
        for step in &result.steps {
            for info in &step.set {
                if !removed.contains(&info.name) && agent::delete_variable(&info.name).is_ok() {
                    removed.push(info.name.clone());
                }
            }
        }
        result.cleaned_up = removed;
    }
    Ok(result)
}

/// Like [`run`], reporting each step as it starts and finishes, and stopping before the next step when
/// `cancel` is set. This is what the window uses to show progress.
pub fn run_reporting(
    name: &str,
    variables: &std::collections::BTreeMap<String, String>,
    source: Source,
    cancel: &std::sync::atomic::AtomicBool,
    report: &mut dyn FnMut(Progress),
) -> Result<WorkflowResult, String> {
    let info = get(name)?;
    let started_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let result = run_steps_reporting(
        &info.name,
        &info.steps,
        variables,
        |params| agent::send_request_raw(params, source),
        |name, value, secret| agent::set_variable(name, value, secret, source),
        &mut |expression, values| agent::evaluate_condition(expression, values),
        cancel,
        report,
    );
    record(&result, &started_at, source);
    Ok(result)
}

/// Writes the run to the history database; a failure to do so never fails the run.
fn record(result: &WorkflowResult, started_at: &str, source: Source) {
    let stored: Vec<StoredStep> = result.steps.iter().map(StoredStep::from).collect();
    let Ok(steps) = serde_json::to_string(&stored) else { return };
    if let Ok(history) = agent::open_history() {
        let _ = history.record_workflow_run(&crate::store::history::NewWorkflowRun {
            workflow: &result.workflow,
            started_at,
            source,
            ok: result.ok,
            cancelled: result.cancelled,
            steps: &steps,
        });
    }
}

/// The runner, with sending and variable-setting passed in so it can be tested without a network.
#[cfg(test)]
fn run_steps(
    workflow: &str,
    steps: &[Step],
    variables: &std::collections::BTreeMap<String, String>,
    send: impl FnMut(&SendParams) -> Result<(AgentResponse, ResponseData), SendFailure>,
    set: impl FnMut(&str, &str, Option<bool>) -> Result<VariableInfo, String>,
) -> WorkflowResult {
    run_steps_reporting(
        workflow,
        steps,
        variables,
        send,
        set,
        &mut |expression, values| Ok(condition_holds(&substitute_plain(expression, values))),
        &std::sync::atomic::AtomicBool::new(false),
        &mut |_| {},
    )
}

/// Decides whether a condition holds, given the run's variables.
type Evaluate = dyn FnMut(&str, &std::collections::BTreeMap<String, String>) -> Result<bool, String>;

#[allow(clippy::too_many_arguments)]
fn run_steps_reporting(
    workflow: &str,
    steps: &[Step],
    variables: &std::collections::BTreeMap<String, String>,
    mut send: impl FnMut(&SendParams) -> Result<(AgentResponse, ResponseData), SendFailure>,
    mut set: impl FnMut(&str, &str, Option<bool>) -> Result<VariableInfo, String>,
    evaluate: &mut Evaluate,
    cancel: &std::sync::atomic::AtomicBool,
    report: &mut dyn FnMut(Progress),
) -> WorkflowResult {
    let mut results: Vec<StepResult> = Vec::new();
    let mut last_response = None;
    let mut ok = true;
    let mut cancelled = false;
    let mut stopped_early = false;
    // values that live for this run only: what `scope: run` extracted, and the status of the step before
    let mut run_values: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (i, step) in steps.iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            ok = false;
            cancelled = true;
            break;
        }
        let label = step.label.clone().unwrap_or_else(|| {
            step.request.saved_request.clone().unwrap_or_else(|| format!("{} {}", step.request.method.as_deref().unwrap_or("GET"), step.request.url.as_deref().unwrap_or("")))
        });
        report(Progress::Started { step: i + 1 });
        let mut result =
            StepResult { step: i + 1, label, ok: false, skipped: false, status: None, elapsed_ms: None, set: Vec::new(), error: None };

        // what a condition and the request itself can see: the run's variables, then the run-scoped values
        let mut visible = variables.clone();
        visible.extend(run_values.iter().map(|(k, v)| (k.clone(), v.clone())));

        // conditions come before the request, so a step that is skipped or ends the run is never sent
        let mut verdict: Result<Option<&'static str>, String> = Ok(None);
        for (kind, condition) in [("stop_if", &step.stop_if), ("skip_if", &step.skip_if)] {
            if let (Some(expression), Ok(None)) = (condition, &verdict) {
                match evaluate(expression, &visible) {
                    Ok(true) => verdict = Ok(Some(kind)),
                    Ok(false) => {}
                    Err(err) => verdict = Err(format!("`{kind}` could not be evaluated: {err}")),
                }
            }
        }
        match verdict {
            Err(message) => {
                result.error = Some(message);
                report(Progress::Finished(result.clone()));
                results.push(result);
                ok = false;
                break;
            }
            Ok(Some("stop_if")) => {
                result.ok = true;
                result.skipped = true;
                report(Progress::Finished(result.clone()));
                results.push(result);
                stopped_early = i + 1 < steps.len();
                break;
            }
            Ok(Some(_)) => {
                result.ok = true;
                result.skipped = true;
                report(Progress::Finished(result.clone()));
                results.push(result);
                continue;
            }
            Ok(None) => {}
        }

        let mut params = step.request.clone();
        if !visible.is_empty() {
            // the step's own `variables` are the most specific, so they win over the run's
            let mut merged: std::collections::BTreeMap<String, String> = visible.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            merged.extend(params.variables.take().unwrap_or_default());
            params.variables = Some(merged);
        }

        match send(&params) {
            Err(failure) => result.error = Some(failure.to_string()),
            Ok((shaped, raw)) => {
                result.status = Some(shaped.status);
                result.elapsed_ms = Some(shaped.elapsed_ms);
                run_values.insert("previous_status".to_string(), shaped.status.to_string());
                let passed = match &step.expect_status {
                    Some(want) => want.matches(shaped.status),
                    None => (200..300).contains(&shaped.status),
                };
                if !passed {
                    let want = step.expect_status.as_ref().map(ExpectStatus::describe).unwrap_or_else(|| "2xx".into());
                    result.error = Some(format!("expected {want}, got {}", shaped.status));
                } else {
                    result.ok = true;
                    for e in &step.request.extract {
                        if e.scope.as_deref() == Some("run") {
                            match extract::extract(&raw, &e.from) {
                                Ok(value) => {
                                    run_values.insert(e.name.trim().to_string(), value);
                                    result.set.push(VariableInfo {
                                        name: e.name.trim().to_string(),
                                        secret: false,
                                        has_value: true,
                                        remembered: false,
                                        value: None,
                                        source: "run".to_string(),
                                    });
                                    continue;
                                }
                                Err(err) => {
                                    result.ok = false;
                                    result.error = Some(format!("could not set `{}` from `{}`: {err}", e.name, e.from));
                                    break;
                                }
                            }
                        }
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
        report(Progress::Finished(result.clone()));
        results.push(result);
        if stop {
            ok = false;
            break;
        }
    }
    WorkflowResult {
        workflow: workflow.to_string(),
        ok,
        cancelled,
        cleaned_up: Vec::new(),
        stopped_early,
        steps_run: results.len(),
        steps: results,
        last_response,
    }
}

/// `{{name}}` replaced by the value in `values` (an unknown name becomes empty). The runner uses the full
/// variable resolver; this is the same thing for tests, with no session behind it.
#[cfg(test)]
fn substitute_plain(expression: &str, values: &std::collections::BTreeMap<String, String>) -> String {
    let mut out = expression.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    while let (Some(start), Some(end)) = (out.find("{{"), out.find("}}")) {
        if end < start {
            break;
        }
        out.replace_range(start..end + 2, "");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn step(url: &str) -> Step {
        Step { label: None, request: SendParams { url: Some(url.into()), ..Default::default() }, ..Default::default() }
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
            request: crate::engine::SentRequest { method: "GET".into(), url: "http://x".into(), history_id: None, sent_at: String::new() },
            redacted: vec![],
            selected: None,
            variables_set: Vec::new(),
            problems: Vec::new(),
            outline: None,
            hint: None,
            ttfb_ms: None,
            final_url: None,
            redirects: Vec::new(),
            sha256: None,
            detected_type: None,
            body_file: None,
        };
        (shaped, raw)
    }

    fn info(name: &str) -> VariableInfo {
        VariableInfo { name: name.into(), secret: false, has_value: true, remembered: false, value: None, source: "agent".into() }
    }

    #[test]
    fn a_value_from_one_response_is_set_for_the_next_request() {
        let mut login = step("http://h/login");
        login.request.extract = vec![Extract { name: "token".into(), from: "json:$.token".into(), secret: None, scope: None }];
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
    fn progress_is_reported_and_a_cancel_stops_before_the_next_step() {
        let steps = vec![step("http://h/a"), step("http://h/b"), step("http://h/c")];
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut events = Vec::new();
        let result = run_steps_reporting(
            "w",
            &steps,
            &BTreeMap::new(),
            |_| Ok(reply(200, "{}")),
            |n, _, _| Ok(info(n)),
            &mut |_, _| Ok(false),
            &cancel,
            &mut |p| {
                if let Progress::Finished(r) = &p {
                    if r.step == 2 {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                events.push(match p {
                    Progress::Started { step } => format!("start {step}"),
                    Progress::Finished(r) => format!("done {}", r.step),
                });
            },
        );
        assert_eq!(events, ["start 1", "done 1", "start 2", "done 2"]);
        assert!(result.cancelled && !result.ok);
        assert_eq!(result.steps_run, 2, "the third step never started");
    }

    #[test]
    fn a_recorded_step_names_its_variables_but_never_their_values() {
        let result = StepResult {
            step: 1,
            label: "log in".into(),
            ok: true,
            skipped: false,
            status: Some(200),
            elapsed_ms: Some(12),
            set: vec![VariableInfo { name: "token".into(), secret: true, has_value: true, remembered: false, value: Some("tok_SECRET".into()), source: "agent".into() }],
            error: None,
        };
        let json = serde_json::to_string(&StoredStep::from(&result)).unwrap();
        assert!(json.contains("\"token\"") && !json.contains("tok_SECRET"), "{json}");
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
        s.expect_status = Some(ExpectStatus::One(404));
        let result = run_steps("w", &[s], &BTreeMap::new(), |_| Ok(reply(404, "{}")), |n, _, _| Ok(info(n)));
        assert!(result.ok);
    }

    #[test]
    fn a_value_that_is_not_there_fails_the_step_and_says_why() {
        let mut s = step("http://h/a");
        s.request.extract = vec![Extract { name: "id".into(), from: "json:$.data.id".into(), secret: None, scope: None }];
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
    fn a_steps_own_variables_win_over_the_runs() {
        let mut seen = Vec::new();
        let vars = BTreeMap::from([("env".to_string(), "prod".to_string()), ("user".to_string(), "ann".to_string())]);
        let mut own = step("http://h/a");
        own.request.variables = Some(BTreeMap::from([("env".to_string(), "staging".to_string())]));
        run_steps("w", &[own], &vars, |p| {
            seen.push(p.variables.clone().unwrap());
            Ok(reply(200, "{}"))
        }, |n, _, _| Ok(info(n)));
        assert_eq!(seen[0].get("env").map(String::as_str), Some("staging"), "the step's override stays");
        assert_eq!(seen[0].get("user").map(String::as_str), Some("ann"), "the run's other values still arrive");
    }

    #[test]
    fn a_workflow_without_a_request_is_refused() {
        assert!(check(&[]).is_err());
        let empty = Step::default();
        assert!(check(&[empty]).unwrap_err().contains("Step 1"));
        assert!(check(&[step("http://h")]).is_ok());
    }

    #[test]
    fn steps_round_trip_as_json_with_the_request_fields_inline() {
        let json = r#"[{"label":"log in","method":"POST","url":"http://h/login","json":{"u":"a"},"extract":[{"name":"token","from":"json:$.token"}],"expect_status":200}]"#;
        let steps = parse_steps(json).unwrap();
        assert_eq!(steps[0].request.method.as_deref(), Some("POST"));
        assert_eq!(steps[0].request.extract[0].from, "json:$.token");
        assert_eq!(steps[0].expect_status, Some(ExpectStatus::One(200)));
        let again = parse_steps(&serde_json::to_string(&steps).unwrap()).unwrap();
        assert_eq!(again[0].request.url.as_deref(), Some("http://h/login"));
    }

    fn with_status(mut s: Step, want: ExpectStatus) -> Step {
        s.expect_status = Some(want);
        s
    }

    #[test]
    fn a_step_may_accept_several_statuses() {
        let steps = vec![with_status(step("http://h/find"), ExpectStatus::Any(vec![200, 404])), with_status(step("http://h/next"), ExpectStatus::Any(vec![200, 404]))];
        for (status, passes) in [(200u16, true), (404, true), (500, false)] {
            let mut sent = 0;
            let result = run_steps("w", &steps, &BTreeMap::new(), |_| {
                sent += 1;
                Ok(reply(status, "{}"))
            }, |n, _, _| Ok(info(n)));
            assert_eq!(result.ok, passes, "status {status}");
            assert_eq!(sent, if passes { 2 } else { 1 });
            if !passes {
                assert_eq!(result.steps[0].error.as_deref(), Some("expected 200, 404, got 500"));
            }
        }
        // a number is still a number in JSON, and a list is a list
        let json = r#"[{"url":"http://h/a","expect_status":201},{"url":"http://h/b","expect_status":[200,404]}]"#;
        let parsed = parse_steps(json).unwrap();
        assert_eq!(parsed[0].expect_status, Some(ExpectStatus::One(201)));
        assert_eq!(parsed[1].expect_status, Some(ExpectStatus::Any(vec![200, 404])));
        assert_eq!(serde_json::to_value(&parsed[0].expect_status).unwrap(), serde_json::json!(201));
        assert_eq!(serde_json::to_value(&parsed[1].expect_status).unwrap(), serde_json::json!([200, 404]));
        assert!(check(&[with_status(step("http://h"), ExpectStatus::Any(vec![]))]).unwrap_err().contains("empty list"));
    }

    #[test]
    fn conditions_compare_filled_in_values() {
        for (text, holds) in [
            ("yes == yes", true),
            ("yes == no", false),
            ("'a b' == \"a b\"", true),
            ("404 != 200", true),
            ("200 != 200", false),
            ("something", true),
            ("", false),
            ("  ", false),
            ("false", false),
            ("FALSE", false),
            ("0", false),
            ("null", false),
            ("1", true),
            (" == ", true),
        ] {
            assert_eq!(condition_holds(text), holds, "{text:?}");
        }
    }

    #[test]
    fn skip_if_leaves_a_step_unsent_and_the_run_goes_on() {
        let mut second = step("http://h/second");
        second.skip_if = Some("{{have}} == yes".into());
        let steps = vec![step("http://h/first"), second, step("http://h/third")];
        for (have, expected) in [("yes", vec!["http://h/first", "http://h/third"]), ("no", vec!["http://h/first", "http://h/second", "http://h/third"])] {
            let mut sent = Vec::new();
            let vars = BTreeMap::from([("have".to_string(), have.to_string())]);
            let result = run_steps("w", &steps, &vars, |p| {
                sent.push(p.url.clone().unwrap());
                Ok(reply(200, "{}"))
            }, |n, _, _| Ok(info(n)));
            assert!(result.ok, "{have}");
            assert_eq!(sent, expected, "{have}");
            assert_eq!(result.steps[1].skipped, have == "yes");
            assert_eq!(result.steps_run, 3);
        }
    }

    #[test]
    fn stop_if_ends_the_run_successfully_and_the_status_before_is_visible() {
        let mut look_up = step("http://h/exists");
        look_up.expect_status = Some(ExpectStatus::Any(vec![200, 404]));
        let mut create = step("http://h/create");
        create.skip_if = Some("{{previous_status}} == 200".into());
        let mut finish = step("http://h/finish");
        finish.stop_if = Some("{{previous_status}} == 404".into());
        let after = step("http://h/after");
        let steps = vec![look_up, create, finish, after];

        // found: the create is skipped, the run goes to the end
        let mut sent = Vec::new();
        let result = run_steps("w", &steps, &BTreeMap::new(), |p| {
            sent.push(p.url.clone().unwrap());
            Ok(reply(200, "{}"))
        }, |n, _, _| Ok(info(n)));
        assert!(result.ok && !result.stopped_early);
        assert_eq!(sent, vec!["http://h/exists", "http://h/finish", "http://h/after"]);

        // not found: it is created (201), so finish does not stop, and the run completes
        let mut sent = Vec::new();
        let mut statuses = vec![404u16, 201, 200, 200].into_iter();
        let result = run_steps("w", &steps, &BTreeMap::new(), |p| {
            sent.push(p.url.clone().unwrap());
            Ok(reply(statuses.next().unwrap(), "{}"))
        }, |n, _, _| Ok(info(n)));
        assert!(result.ok);
        assert_eq!(sent.len(), 4);

        // a stop_if that holds ends the run there: ok, and the steps after it are not sent
        let mut stop_early = step("http://h/stop");
        stop_early.stop_if = Some("{{previous_status}} == 404".into());
        let steps = vec![with_status(step("http://h/exists"), ExpectStatus::Any(vec![404])), stop_early, step("http://h/never")];
        let mut sent = Vec::new();
        let result = run_steps("w", &steps, &BTreeMap::new(), |p| {
            sent.push(p.url.clone().unwrap());
            Ok(reply(404, "{}"))
        }, |n, _, _| Ok(info(n)));
        assert!(result.ok && result.stopped_early);
        assert_eq!(sent, vec!["http://h/exists"]);
        assert_eq!(result.steps_run, 2, "the stopping step is recorded, as skipped");
        assert!(result.steps[1].skipped);
    }

    #[test]
    fn a_run_scoped_value_reaches_the_next_steps_and_is_never_stored() {
        let mut login = step("http://h/login");
        login.request.extract = vec![
            Extract { name: "session".into(), from: "json:$.session".into(), secret: None, scope: Some("run".into()) },
            Extract { name: "keep".into(), from: "json:$.keep".into(), secret: None, scope: None },
        ];
        let steps = vec![login, step("http://h/next")];
        let mut stored = Vec::new();
        let mut next_saw = None;
        let mut n = 0;
        let result = run_steps("w", &steps, &BTreeMap::new(), |p| {
            n += 1;
            if n == 2 {
                next_saw = p.variables.clone();
            }
            Ok(reply(200, r#"{"session": "s-1", "keep": "k-1"}"#))
        }, |name, value, _| {
            stored.push((name.to_string(), value.to_string()));
            Ok(info(name))
        });
        assert!(result.ok);
        assert_eq!(stored, vec![("keep".to_string(), "k-1".to_string())], "only the global value is stored");
        let saw = next_saw.expect("the second step was sent");
        assert_eq!(saw.get("session").map(String::as_str), Some("s-1"));
        assert_eq!(result.steps[0].set.iter().map(|v| (v.name.as_str(), v.source.as_str())).collect::<Vec<_>>(), vec![("session", "run"), ("keep", "agent")]);
        assert!(result.steps[0].set.iter().all(|v| v.value.is_none()), "values are not reported");
    }

    #[test]
    fn a_bad_scope_or_an_empty_condition_is_refused_before_saving() {
        let mut bad_scope = step("http://h");
        bad_scope.request.extract = vec![Extract { name: "a".into(), from: "status".into(), secret: None, scope: Some("forever".into()) }];
        assert!(check(&[bad_scope]).unwrap_err().contains("run` or `global"));
        let mut blank = step("http://h");
        blank.skip_if = Some("  ".into());
        assert!(check(&[blank]).unwrap_err().contains("skip_if"));
    }
}