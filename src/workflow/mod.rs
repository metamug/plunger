//! Workflows: a saved, ordered list of requests where a later one can use values from an earlier
//! response. A step sends a request (inline or a saved one), then `extract`s values from the
//! response into variables (`{{name}}`) for the steps after it. A value named like a credential
//! (`token`, `password`...) is kept as a secret: it is used in later requests but never shown.

pub mod flow;
pub mod junit;
pub mod script;

pub use crate::domain::assertion::{Assert, AssertOutcome};
pub use crate::domain::extract::{self, Extract};
pub use flow::{Paginate, Poll, Retry};

#[cfg(test)]
use crate::domain::condition::condition_holds;
use crate::agent::{self, SendFailure, SendParams};
use crate::engine::{AgentResponse, VariableInfo};
use crate::store::history::Source;
use crate::domain::model::ResponseData;
use crate::domain::transform;
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

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
    /// Checks on the response (`{"from": "json:$.id", "equals": 7}`, `{"from": "time", "lt": 500}`). Every one is
    /// run and reported; the step fails if any does. They run after the status check and before `extract`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asserts: Vec<Assert>,
    /// Skip this step (it counts as passed) when the condition holds. A condition is `{{a}} == b`, `{{a}} != b`,
    /// or a single value that is true when it is not empty, `false`, `0` or `null`. A variable with no value is
    /// empty. `{{previous_status}}` is the status of the step before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_if: Option<String>,
    /// End the workflow here, successfully, when the condition holds (same form as `skip_if`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_if: Option<String>,
    /// Send this step once for every element of a list: the name of a variable that holds a JSON array (one a
    /// `collect` filled), or an array written out. The element is `{{item}}` (or the name in `as`), and for an
    /// object its fields are `{{item.key}}`, `{{item.fields.name}}`. `{{loop.index}}` counts from 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub for_each: Option<String>,
    /// The name of the element in a `for_each` (default `item`).
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub item_name: Option<String>,
    /// Page through a list: send the request again for the next page and gather the items. See `Paginate`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paginate: Option<Paginate>,
    /// Send the request again until a condition holds, for a job that finishes later. See `Poll`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<Poll>,
    /// Try again after a 429, a 5xx or a dropped connection. See `Retry`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Retry>,
    /// Keep what the step fetched as the run's value `{{name}}` for the later steps (and for `return`): the items
    /// of every page for a `paginate`, the response body otherwise; with `for_each`, a list with one entry per
    /// element. It lives for this run only and is never stored. Known secrets in it are masked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect: Option<String>,
    /// With `for_each`: an element that fails is recorded (as null in the collected list) and the step goes on,
    /// instead of ending the run. The step fails only if every element fails.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub continue_on_error: bool,
    /// Wait this long between the requests of one step (pages, elements), to stay under a rate limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
    /// A jq program that turns the run's values into the answer the run returns, e.g.
    /// `{count: (.issues|length), points: ([.issues[].fields.points]|add)}`. The input is an object of the run's
    /// values (`collect`ed lists, `scope: run` extracts, the variables passed in), without secrets. A step may have
    /// only this and no request. The last `return` of the run is its `output`. jq here has no file, network or
    /// environment access.
    #[serde(default, rename = "return", skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl Step {
    /// Whether the step sends anything (a step may only `return`).
    pub fn has_request(&self) -> bool {
        self.request.saved_request.is_some() || self.request.url.as_deref().is_some_and(|u| !u.trim().is_empty())
    }
}

/// What one step did.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
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
    /// What each of the step's `asserts` found.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asserts: Vec<AssertOutcome>,
    /// How many requests the step sent (pages, elements, polling attempts and retries each count). Shown when it is not 1.
    #[serde(default, skip_serializing_if = "is_zero_or_one")]
    pub requests: usize,
    /// Something that did not fail the step but should be known: paging stopped at `max_pages`, some elements failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn is_zero_or_one(n: &usize) -> bool {
    *n <= 1
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
    /// What the last `return` step produced (see `Step::return`): the answer the run was written to give.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
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
        if !step.has_request() && step.output.is_none() {
            return Err(format!("Step {} needs a `url` or a `saved_request` (or only a `return`).", i + 1));
        }
        check_flow(step).map_err(|m| format!("Step {}: {m}", i + 1))?;
        agent::check_reading_instructions(&step.request).map_err(|m| format!("Step {}: {m}", i + 1))?;
        if matches!(&step.expect_status, Some(ExpectStatus::Any(list)) if list.is_empty()) {
            return Err(format!("Step {}: `expect_status` is an empty list.", i + 1));
        }
        for (field, condition) in [("skip_if", &step.skip_if), ("stop_if", &step.stop_if)] {
            if condition.as_deref().is_some_and(|c| c.trim().is_empty()) {
                return Err(format!("Step {}: `{field}` is empty.", i + 1));
            }
        }
        for (n, a) in step.asserts.iter().enumerate() {
            a.validate().map_err(|m| format!("Step {}: assert {}: {m}", i + 1, n + 1))?;
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

/// The mistakes in a step's `for_each`, `paginate`, `poll`, `retry`, `collect` and `return`.
fn check_flow(step: &Step) -> Result<(), String> {
    if let Some(p) = &step.paginate {
        p.validate().map_err(|m| format!("`paginate`: {m}"))?;
        if p.style() == "link" && (step.request.saved_request.is_some() || !step.has_request()) {
            return Err("`paginate` style `link` needs an inline `url`, not a saved request".into());
        }
    }
    if let Some(p) = &step.poll {
        p.validate().map_err(|m| format!("`poll`: {m}"))?;
    }
    if let Some(r) = &step.retry {
        r.validate().map_err(|m| format!("`retry`: {m}"))?;
    }
    if step.paginate.is_some() && step.poll.is_some() {
        return Err("`paginate` and `poll` cannot be used in one step".into());
    }
    if !step.has_request() {
        let sends = step.paginate.is_some() || step.poll.is_some() || step.retry.is_some() || step.for_each.is_some() || step.collect.is_some();
        if sends {
            return Err("this step has no `url` or `saved_request` to send".into());
        }
    }
    if step.for_each.as_deref().is_some_and(|f| f.trim().is_empty()) {
        return Err("`for_each` is empty".into());
    }
    if let Some(name) = step.item_name.as_deref() {
        if step.for_each.is_none() {
            return Err("`as` names the element of a `for_each`, which this step does not have".into());
        }
        if !flow::valid_name(name) {
            return Err(format!("`as` is `{name}`; use letters, digits, - and _"));
        }
    }
    if let Some(name) = step.collect.as_deref() {
        if !flow::valid_name(name) {
            return Err(format!("`collect` is `{name}`; use letters, digits, - and _ (at most 64 characters)"));
        }
    }
    if step.continue_on_error && step.for_each.is_none() {
        return Err("`continue_on_error` goes with a `for_each`".into());
    }
    if step.delay_ms.is_some_and(|d| d > flow::MAX_WAIT_MS) {
        return Err(format!("`delay_ms` is at most {}", flow::MAX_WAIT_MS));
    }
    if let Some(program) = step.output.as_deref() {
        transform::check(program).map_err(|m| format!("`return`: {m}"))?;
    }
    Ok(())
}

/// Checks steps without sending anything: the mistakes `save` would refuse.
pub fn validate(steps: &[Step]) -> Result<(), String> {
    check(steps)
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
    variables: &BTreeMap<String, String>,
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
    variables: &BTreeMap<String, String>,
    source: Source,
    cancel: &std::sync::atomic::AtomicBool,
    report: &mut dyn FnMut(Progress),
) -> Result<WorkflowResult, String> {
    let info = get(name)?;
    Ok(run_loaded(&info.name, &info.steps, variables, source, cancel, report))
}

/// Runs steps that are not saved (a script file), checking them first. Recorded like any run.
pub fn run_inline(
    name: &str,
    steps: &[Step],
    variables: &BTreeMap<String, String>,
    source: Source,
) -> Result<WorkflowResult, String> {
    check(steps)?;
    Ok(run_loaded(name, steps, variables, source, &std::sync::atomic::AtomicBool::new(false), &mut |_| {}))
}

fn run_loaded(
    name: &str,
    steps: &[Step],
    variables: &BTreeMap<String, String>,
    source: Source,
    cancel: &std::sync::atomic::AtomicBool,
    report: &mut dyn FnMut(Progress),
) -> WorkflowResult {
    let started_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let result = run_steps_reporting(
        name,
        steps,
        variables,
        |params| agent::send_request_raw(params, source),
        |name, value, secret| agent::set_variable(name, value, secret, source),
        &mut |expression, values| agent::evaluate_condition(expression, values),
        &mut |ms| sleep_unless_cancelled(ms, cancel),
        &mut agent::scrub_text,
        cancel,
        report,
    );
    record(&result, &started_at, source);
    result
}

/// Waits, in short pieces so a cancel is noticed.
fn sleep_unless_cancelled(ms: u64, cancel: &std::sync::atomic::AtomicBool) {
    let mut left = ms;
    while left > 0 && !cancel.load(std::sync::atomic::Ordering::Relaxed) {
        let piece = left.min(100);
        std::thread::sleep(std::time::Duration::from_millis(piece));
        left -= piece;
    }
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
    variables: &BTreeMap<String, String>,
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
        &mut |_| {},
        &mut |text| text.to_string(),
        &std::sync::atomic::AtomicBool::new(false),
        &mut |_| {},
    )
}

/// Decides whether a condition holds, given the run's variables.
type Evaluate<'a> = dyn FnMut(&str, &BTreeMap<String, String>) -> Result<bool, String> + 'a;
/// Sends one request.
type SendFn<'a> = dyn FnMut(&SendParams) -> Result<(AgentResponse, ResponseData), SendFailure> + 'a;
/// Stores a variable for good (`extract` without `scope: run`).
type SetVar<'a> = dyn FnMut(&str, &str, Option<bool>) -> Result<VariableInfo, String> + 'a;

/// Everything a running workflow needs from the outside, so it can be tested without a network or a clock.
struct Hooks<'a> {
    send: &'a mut SendFn<'a>,
    set: &'a mut SetVar<'a>,
    evaluate: &'a mut Evaluate<'a>,
    /// Waits this many milliseconds.
    sleep: &'a mut dyn FnMut(u64),
    /// Masks the secrets Plunger knows in some text.
    scrub: &'a mut dyn FnMut(&str) -> String,
    cancel: &'a std::sync::atomic::AtomicBool,
}

/// What lives for one run: the values `scope: run` extracted or a step collected.
#[derive(Default)]
struct RunState {
    values: BTreeMap<String, String>,
    /// Names of values that are secrets: kept out of what `return` can see.
    secret_names: BTreeSet<String>,
    /// The secret values themselves, to mask them in what `return` produces.
    secret_values: Vec<(String, String)>,
}

/// What one step has done so far.
#[derive(Default)]
struct Tally {
    /// The last response that came back.
    last: Option<AgentResponse>,
    status: Option<u16>,
    requests: usize,
    elapsed_ms: u64,
    set: Vec<VariableInfo>,
    asserts: Vec<AssertOutcome>,
    warning: Option<String>,
    output: Option<Value>,
}

impl Tally {
    fn record(&mut self, shaped: AgentResponse) {
        self.status = Some(shaped.status);
        self.elapsed_ms += shaped.elapsed_ms;
        self.last = Some(shaped);
    }

    fn remember(&mut self, info: VariableInfo) {
        match self.set.iter_mut().find(|v| v.name == info.name) {
            Some(existing) => *existing = info,
            None => self.set.push(info),
        }
    }
}

/// What a unit of requests (one request, all the pages, or all the polling attempts) leaves to check.
struct Unit {
    raw: ResponseData,
    /// The items of every page, for a paginated step.
    items: Option<Vec<Value>>,
    /// `extract` already ran (polling reads each response).
    extracted: bool,
}

fn status_ok(step: &Step, status: u16) -> Result<(), String> {
    let passed = match &step.expect_status {
        Some(want) => want.matches(status),
        None => (200..300).contains(&status),
    };
    if passed {
        return Ok(());
    }
    let want = step.expect_status.as_ref().map(ExpectStatus::describe).unwrap_or_else(|| "2xx".into());
    Err(format!("expected {want}, got {status}"))
}

fn is_cancelled(hooks: &Hooks) -> bool {
    hooks.cancel.load(std::sync::atomic::Ordering::Relaxed)
}

fn with_variables(params: &SendParams, scope: &BTreeMap<String, String>) -> SendParams {
    let mut params = params.clone();
    if !scope.is_empty() {
        // the step's own `variables` are the most specific, so they win over the run's
        let mut merged = scope.clone();
        merged.extend(params.variables.take().unwrap_or_default());
        params.variables = Some(merged);
    }
    params
}

/// Sends the request, trying again as `retry` says.
fn send_retrying(step: &Step, params: &SendParams, hooks: &mut Hooks, tally: &mut Tally) -> Result<(AgentResponse, ResponseData), String> {
    let (retry, attempts) = match &step.retry {
        Some(r) => (r.clone(), r.attempts()),
        None => (Retry::default(), 1),
    };
    let statuses = retry.statuses();
    for attempt in 1..=attempts {
        tally.requests += 1;
        match (hooks.send)(params) {
            Ok((shaped, raw)) => {
                if attempt < attempts && step.retry.is_some() && statuses.contains(&shaped.status) {
                    let after = raw.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("retry-after")).map(|(_, v)| v.as_str());
                    (hooks.sleep)(retry.wait_ms(attempt, after));
                    continue;
                }
                return Ok((shaped, raw));
            }
            Err(SendFailure::Failed(_)) if attempt < attempts && step.retry.is_some() => (hooks.sleep)(retry.wait_ms(attempt, None)),
            Err(failure) => return Err(failure.to_string()),
        }
    }
    Err("no request was sent".into())
}

/// The items of one page.
fn page_items(raw: &ResponseData, paginate: &Paginate) -> Result<Vec<Value>, String> {
    let spec = paginate.items.as_deref().unwrap_or("$");
    match extract::select(raw, spec)? {
        Value::Array(items) => Ok(items),
        _ => Err(format!("`items` ({spec}) is not an array in the response; point it at the list, e.g. json:$.issues")),
    }
}

fn number_at(raw: &ResponseData, spec: &str) -> Option<u64> {
    match extract::select(raw, spec).ok()? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn single_unit(step: &Step, scope: &BTreeMap<String, String>, hooks: &mut Hooks, tally: &mut Tally) -> Result<Unit, String> {
    let params = with_variables(&step.request, scope);
    let (shaped, raw) = send_retrying(step, &params, hooks, tally)?;
    tally.record(shaped);
    status_ok(step, raw.status)?;
    Ok(Unit { raw, items: None, extracted: false })
}

fn paginate_unit(
    step: &Step,
    paginate: &Paginate,
    scope: &BTreeMap<String, String>,
    hooks: &mut Hooks,
    tally: &mut Tally,
) -> Result<Unit, String> {
    let style = paginate.style();
    let max_pages = paginate.max_pages();
    let mut items: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut next_url: Option<String> = None;
    for page in 1..=max_pages {
        if is_cancelled(hooks) {
            return Err("the run was cancelled".into());
        }
        let mut vars = scope.clone();
        vars.insert("page.number".into(), page.to_string());
        vars.insert("page.offset".into(), items.len().to_string());
        if let Some(c) = &cursor {
            vars.insert("page.cursor".into(), c.clone());
        }
        let mut params = with_variables(&step.request, &vars);
        if let Some(url) = &next_url {
            params.url = Some(url.clone());
        }
        let (shaped, raw) = send_retrying(step, &params, hooks, tally)?;
        tally.record(shaped);
        status_ok(step, raw.status).map_err(|m| format!("page {page}: {m}"))?;
        let fetched = page_items(&raw, paginate).map_err(|m| format!("page {page}: {m}"))?;
        let count = fetched.len();
        items.extend(fetched);
        if items.len() > flow::MAX_ITEMS {
            return Err(format!("more than {} items; narrow the request", flow::MAX_ITEMS));
        }
        let more = match style {
            "cursor" => {
                let spec = paginate.next.as_deref().unwrap_or("");
                cursor = extract::select(&raw, spec).ok().map(|v| flow::text_of(&v)).filter(|c| !c.is_empty());
                count > 0 && cursor.is_some()
            }
            "link" => {
                let link = raw.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("link")).and_then(|(_, v)| flow::next_link(v));
                next_url = match link {
                    Some(next) => {
                        let written = params.url.clone().unwrap_or_default();
                        Some(flow::resolve_link(&next, &written).map_err(|m| format!("page {page}: {m}"))?)
                    }
                    None => None,
                };
                next_url.is_some()
            }
            "page" => count > 0 && paginate.page_size.is_none_or(|size| count >= size),
            _ => {
                let by_total = paginate.total.as_deref().and_then(|spec| number_at(&raw, spec)).map(|total| (items.len() as u64) < total);
                count > 0 && by_total.unwrap_or_else(|| paginate.page_size.is_none_or(|size| count >= size))
            }
        };
        if !more {
            return Ok(Unit { raw, items: Some(items), extracted: false });
        }
        if page == max_pages {
            tally.warning = Some(format!("stopped after {max_pages} pages with more to fetch (raise `max_pages`, at most {})", flow::MAX_PAGES));
            return Ok(Unit { raw, items: Some(items), extracted: false });
        }
        if let Some(ms) = step.delay_ms {
            (hooks.sleep)(ms.min(flow::MAX_WAIT_MS));
        }
    }
    Err("no page was fetched".into())
}

fn poll_unit(
    step: &Step,
    poll: &Poll,
    scope: &BTreeMap<String, String>,
    state: &mut RunState,
    hooks: &mut Hooks,
    tally: &mut Tally,
) -> Result<Unit, String> {
    let attempts = poll.max_attempts();
    for attempt in 1..=attempts {
        if is_cancelled(hooks) {
            return Err("the run was cancelled".into());
        }
        let params = with_variables(&step.request, scope);
        let (shaped, raw) = send_retrying(step, &params, hooks, tally)?;
        tally.record(shaped);
        status_ok(step, raw.status).map_err(|m| format!("attempt {attempt}: {m}"))?;
        apply_extracts(step, &raw, state, hooks, tally)?;
        let mut visible = scope.clone();
        visible.extend(state.values.iter().map(|(k, v)| (k.clone(), v.clone())));
        if (hooks.evaluate)(&poll.until, &visible).map_err(|m| format!("`until` could not be evaluated: {m}"))? {
            return Ok(Unit { raw, items: None, extracted: true });
        }
        if attempt == attempts {
            return Err(format!("gave up after {attempts} attempts: `{}` never held", poll.until));
        }
        (hooks.sleep)(poll.every_ms());
    }
    Err("no request was sent".into())
}

/// Reads the step's `extract` values from a response: into the run's values (`scope: run`) or into variables
/// that stay.
fn apply_extracts(step: &Step, raw: &ResponseData, state: &mut RunState, hooks: &mut Hooks, tally: &mut Tally) -> Result<(), String> {
    for e in &step.request.extract {
        let name = e.name.trim().to_string();
        let fail = |err: String| format!("could not set `{}` from `{}`: {err}", e.name, e.from);
        let value = extract::extract(raw, &e.from).map_err(fail)?;
        if e.scope.as_deref() == Some("run") {
            if e.secret == Some(true) || crate::domain::redact::is_sensitive_header(&name) {
                state.secret_names.insert(name.clone());
                state.secret_values.push((name.clone(), value.clone()));
            }
            state.values.insert(name.clone(), value);
            tally.remember(VariableInfo { name, secret: false, has_value: true, remembered: false, value: None, source: "run".to_string() });
        } else {
            let info = (hooks.set)(&name, &value, e.secret).map_err(|err| format!("could not set `{}` from `{}`: {err}", e.name, e.from))?;
            tally.remember(info);
        }
    }
    Ok(())
}

/// The response body as JSON, or as text when it is not JSON.
fn body_value(raw: &ResponseData) -> Value {
    let text = raw.raw_text.as_deref().unwrap_or(&raw.body);
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

/// One element of a `for_each` (or the only one): its requests, its checks, its `extract`. Returns what `collect` keeps.
fn run_one(
    step: &Step,
    scope: &BTreeMap<String, String>,
    state: &mut RunState,
    hooks: &mut Hooks,
    tally: &mut Tally,
) -> Result<Value, String> {
    let unit = if let Some(p) = &step.paginate {
        paginate_unit(step, p, scope, hooks, tally)?
    } else if let Some(p) = &step.poll {
        poll_unit(step, p, scope, state, hooks, tally)?
    } else {
        single_unit(step, scope, hooks, tally)?
    };
    let elapsed = tally.last.as_ref().map_or(0, |l| l.elapsed_ms);
    let outcomes: Vec<AssertOutcome> = step.asserts.iter().map(|a| crate::domain::assertion::evaluate(a, &unit.raw, elapsed)).collect();
    let failed: Vec<String> =
        outcomes.iter().filter(|o| !o.ok).map(|o| format!("{}: {}", o.name, o.message.as_deref().unwrap_or("failed"))).collect();
    let total = outcomes.len();
    if !outcomes.is_empty() {
        tally.asserts = outcomes;
    }
    if !failed.is_empty() {
        return Err(format!("{} of {total} asserts failed: {}", failed.len(), failed.join("; ")));
    }
    if !unit.extracted {
        apply_extracts(step, &unit.raw, state, hooks, tally)?;
    }
    Ok(match unit.items {
        Some(items) => Value::Array(items),
        None => body_value(&unit.raw),
    })
}

/// A step's requests: once, or once for every element of its `for_each`; then `collect`.
fn run_requests(step: &Step, visible: &BTreeMap<String, String>, state: &mut RunState, hooks: &mut Hooks, tally: &mut Tally) -> Result<(), String> {
    let elements: Vec<Option<Value>> = match &step.for_each {
        Some(source) => flow::list_for_each(source, visible)?.into_iter().map(Some).collect(),
        None => vec![None],
    };
    let many = step.for_each.is_some();
    let name = step.item_name.as_deref().unwrap_or("item").trim().to_string();
    let total = elements.len();
    // a big collected list stays out of the requests' variables: it is there for `for_each` and `return`
    let base: BTreeMap<String, String> =
        visible.iter().filter(|(_, v)| v.len() <= flow::MAX_REQUEST_VARIABLE_BYTES).map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut collected: Vec<Value> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for (index, element) in elements.into_iter().enumerate() {
        if is_cancelled(hooks) {
            return Err("the run was cancelled".into());
        }
        let mut scope = base.clone();
        if let Some(item) = &element {
            flow::bind_item(&mut scope, &name, item, index);
        }
        match run_one(step, &scope, state, hooks, tally) {
            Ok(value) => collected.push(value),
            Err(message) if many && step.continue_on_error => {
                failures.push(format!("{name} {}: {message}", index + 1));
                collected.push(Value::Null);
            }
            Err(message) if many => return Err(format!("{name} {} of {total}: {message}", index + 1)),
            Err(message) => return Err(message),
        }
        if many && index + 1 < total {
            if let Some(ms) = step.delay_ms {
                (hooks.sleep)(ms.min(flow::MAX_WAIT_MS));
            }
        }
    }
    if !failures.is_empty() {
        if failures.len() == total {
            return Err(format!("every element failed; the first: {}", failures[0]));
        }
        tally.warning = Some(format!("{} of {total} elements failed (continue_on_error): {}", failures.len(), failures[0]));
    }
    if let Some(collect) = step.collect.as_deref() {
        let value = if many { Value::Array(collected) } else { collected.into_iter().next().unwrap_or(Value::Null) };
        let text = (hooks.scrub)(&serde_json::to_string(&value).map_err(|e| e.to_string())?);
        if text.len() > flow::MAX_COLLECTED_BYTES {
            return Err(format!("`collect` {collect}: {} bytes is more than the limit of {}", text.len(), flow::MAX_COLLECTED_BYTES));
        }
        let name = collect.trim().to_string();
        state.values.insert(name.clone(), text);
        tally.remember(VariableInfo { name, secret: false, has_value: true, remembered: false, value: None, source: "run".to_string() });
    }
    Ok(())
}

/// The data a `return` program sees: the values of the run and the variables it was started with, minus secrets.
fn return_input(variables: &BTreeMap<String, String>, state: &RunState) -> Value {
    let mut map = serde_json::Map::new();
    for (name, text) in variables.iter().chain(state.values.iter()) {
        if state.secret_names.contains(name) || crate::domain::redact::is_sensitive_header(name) {
            continue;
        }
        let trimmed = text.trim_start();
        let value = if trimmed.starts_with('[') || trimmed.starts_with('{') {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
        } else {
            Value::String(text.clone())
        };
        map.insert(name.clone(), value);
    }
    Value::Object(map)
}

/// Runs the step's `return` program and masks any secret in what it produced.
fn run_return(program: &str, variables: &BTreeMap<String, String>, state: &RunState, hooks: &mut Hooks) -> Result<Value, String> {
    let output = transform::run(program, &return_input(variables, state))?;
    let mut text = (hooks.scrub)(&serde_json::to_string(&output).map_err(|e| e.to_string())?);
    for (name, value) in &state.secret_values {
        if let Some(scrubber) = crate::engine::Scrubber::only(name, value) {
            text = scrubber.text(&text);
        }
    }
    if text.chars().count() > flow::MAX_OUTPUT_CHARS {
        return Err(format!("the result is {} characters; the limit is {}. Make the jq program return less.", text.chars().count(), flow::MAX_OUTPUT_CHARS));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// One step that was not skipped: its requests, then its `return`.
fn execute_step(
    step: &Step,
    variables: &BTreeMap<String, String>,
    visible: &BTreeMap<String, String>,
    state: &mut RunState,
    hooks: &mut Hooks,
    tally: &mut Tally,
) -> Result<(), String> {
    if step.has_request() {
        run_requests(step, visible, state, hooks, tally)?;
    }
    if let Some(program) = step.output.as_deref() {
        tally.output = Some(run_return(program, variables, state, hooks)?);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_steps_reporting(
    workflow: &str,
    steps: &[Step],
    variables: &BTreeMap<String, String>,
    mut send: impl FnMut(&SendParams) -> Result<(AgentResponse, ResponseData), SendFailure>,
    mut set: impl FnMut(&str, &str, Option<bool>) -> Result<VariableInfo, String>,
    evaluate: &mut Evaluate,
    sleep: &mut dyn FnMut(u64),
    scrub: &mut dyn FnMut(&str) -> String,
    cancel: &std::sync::atomic::AtomicBool,
    report: &mut dyn FnMut(Progress),
) -> WorkflowResult {
    let mut results: Vec<StepResult> = Vec::new();
    let mut last_response = None;
    let mut output = None;
    let mut ok = true;
    let mut cancelled = false;
    let mut stopped_early = false;
    // values that live for this run only: what `scope: run` extracted, what a step collected, the status before
    let mut state = RunState::default();
    for (i, step) in steps.iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            ok = false;
            cancelled = true;
            break;
        }
        let label = step.label.clone().unwrap_or_else(|| {
            if !step.has_request() {
                return "return".to_string();
            }
            step.request.saved_request.clone().unwrap_or_else(|| format!("{} {}", step.request.method.as_deref().unwrap_or("GET"), step.request.url.as_deref().unwrap_or("")))
        });
        report(Progress::Started { step: i + 1 });
        let mut result = StepResult { step: i + 1, label, ..Default::default() };

        // what a condition and the request itself can see: the run's variables, then the run-scoped values
        let mut visible = variables.clone();
        visible.extend(state.values.iter().map(|(k, v)| (k.clone(), v.clone())));

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

        let mut tally = Tally::default();
        let outcome = {
            let mut hooks = Hooks { send: &mut send, set: &mut set, evaluate: &mut *evaluate, sleep: &mut *sleep, scrub: &mut *scrub, cancel };
            execute_step(step, variables, &visible, &mut state, &mut hooks, &mut tally)
        };
        result.status = tally.status;
        result.elapsed_ms = tally.status.map(|_| tally.elapsed_ms);
        result.requests = tally.requests;
        result.set = std::mem::take(&mut tally.set);
        result.asserts = std::mem::take(&mut tally.asserts);
        result.warning = tally.warning.take();
        if let Some(status) = tally.status {
            state.values.insert("previous_status".to_string(), status.to_string());
        }
        if let Some(shaped) = tally.last.take() {
            last_response = Some(shaped);
        }
        match outcome {
            Ok(()) => {
                result.ok = true;
                if tally.output.is_some() {
                    output = tally.output.take();
                }
            }
            Err(message) => result.error = Some(message),
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
        output,
        last_response,
    }
}

/// `{{name}}` replaced by the value in `values` (an unknown name becomes empty). The runner uses the full
/// variable resolver; this is the same thing for tests, with no session behind it.
#[cfg(test)]
fn substitute_plain(expression: &str, values: &BTreeMap<String, String>) -> String {
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
    fn every_assert_is_reported_and_one_failure_fails_the_step() {
        use serde_json::json;
        let mut s = step("http://h/a");
        s.asserts = vec![
            Assert { from: "json:$.id".into(), equals: Some(json!(7)), ..Default::default() },
            Assert { from: "$.name".into(), equals: Some(json!("Bob")), ..Default::default() },
            Assert { from: "$.nope".into(), exists: Some(true), ..Default::default() },
        ];
        s.request.extract = vec![Extract { name: "id".into(), from: "json:$.id".into(), secret: None, scope: Some("run".into()) }];
        let result = run_steps("w", &[s, step("http://h/b")], &BTreeMap::new(), |_| Ok(reply(200, r#"{"id": 7, "name": "Ada"}"#)), |n, _, _| Ok(info(n)));
        assert!(!result.ok);
        assert_eq!(result.steps_run, 1, "the failed step stops the run");
        let step = &result.steps[0];
        assert_eq!(step.asserts.iter().map(|a| a.ok).collect::<Vec<_>>(), [true, false, false]);
        let error = step.error.as_deref().unwrap();
        assert!(error.starts_with("2 of 3 asserts failed"), "{error}");
        assert!(error.contains("expected \"Bob\", got \"Ada\""), "{error}");
        assert!(step.set.is_empty(), "nothing is extracted from a failed step");
    }

    #[test]
    fn passing_asserts_let_the_extract_run() {
        use serde_json::json;
        let mut s = step("http://h/a");
        s.asserts = vec![Assert { from: "status".into(), equals: Some(json!(200)), ..Default::default() }];
        s.request.extract = vec![Extract { name: "id".into(), from: "json:$.id".into(), secret: None, scope: Some("run".into()) }];
        let result = run_steps("w", &[s], &BTreeMap::new(), |_| Ok(reply(200, r#"{"id": 7}"#)), |n, _, _| Ok(info(n)));
        assert!(result.ok);
        assert_eq!(result.steps[0].asserts.len(), 1);
        assert_eq!(result.steps[0].set[0].name, "id");
    }

    #[test]
    fn an_assert_that_checks_nothing_is_refused_before_anything_is_sent() {
        let mut s = step("http://h/a");
        s.asserts = vec![Assert { from: "$.id".into(), ..Default::default() }];
        let err = check(&[s]).unwrap_err();
        assert!(err.contains("Step 1") && err.contains("checks nothing"), "{err}");
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
            &mut |_| {},
            &mut |text| text.to_string(),
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
            asserts: vec![],
            ..Default::default()
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

    // ---- control flow: paging, loops, polling, retries, return ----

    fn reply_with(status: u16, body: &str, headers: &[(&str, &str)]) -> (AgentResponse, ResponseData) {
        let (shaped, mut raw) = reply(status, body);
        raw.headers = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        (shaped, raw)
    }

    /// Runs steps with a fake clock: returns the result and the waits the run asked for.
    fn run_timed(
        steps: &[Step],
        variables: &BTreeMap<String, String>,
        send: impl FnMut(&SendParams) -> Result<(AgentResponse, ResponseData), SendFailure>,
    ) -> (WorkflowResult, Vec<u64>) {
        let mut sleeps = Vec::new();
        let result = run_steps_reporting(
            "w",
            steps,
            variables,
            send,
            |n, _, _| Ok(info(n)),
            &mut |expression, values| Ok(condition_holds(&substitute_plain(expression, values))),
            &mut |ms| sleeps.push(ms),
            &mut |text| text.replace("s3cr3t-value", "[masked]"),
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_| {},
        );
        (result, sleeps)
    }

    fn var(p: &SendParams, name: &str) -> Option<String> {
        p.variables.as_ref().and_then(|v| v.get(name).cloned())
    }

    #[test]
    fn offset_paging_gathers_every_page_and_stops_at_the_total() {
        let mut s = step("http://h/search?startAt={{page.offset}}");
        s.paginate = Some(Paginate { items: Some("json:$.issues".into()), total: Some("json:$.total".into()), ..Default::default() });
        s.collect = Some("issues".into());
        let mut offsets = Vec::new();
        let (result, _) = run_timed(&[s], &BTreeMap::new(), |p| {
            let offset: usize = var(p, "page.offset").unwrap().parse().unwrap();
            offsets.push(offset);
            let issues: Vec<usize> = (offset..(offset + 2).min(5)).collect();
            Ok(reply(200, &serde_json::json!({"total": 5, "issues": issues}).to_string()))
        });
        assert!(result.ok, "{:?}", result.steps[0].error);
        assert_eq!(offsets, [0, 2, 4], "the last page has one item and the total says that is all");
        assert_eq!(result.steps[0].requests, 3);
        assert_eq!(result.steps[0].set[0].name, "issues");
    }

    #[test]
    fn collected_pages_reach_a_return_program() {
        let mut fetch = step("http://h/search");
        fetch.paginate = Some(Paginate { items: Some("json:$.issues".into()), page_size: Some(2), ..Default::default() });
        fetch.collect = Some("issues".into());
        let summary = Step { output: Some("{count: (.issues|length), points: ([.issues[].p]|add), sprint: .sprint}".into()), ..Default::default() };
        let mut vars = BTreeMap::new();
        vars.insert("sprint".to_string(), "12".to_string());
        let mut page = 0;
        let (result, _) = run_timed(&[fetch, summary], &vars, |_| {
            page += 1;
            let body = match page {
                1 => r#"{"issues": [{"p": 3}, {"p": 5}]}"#,
                _ => r#"{"issues": [{"p": 2}]}"#,
            };
            Ok(reply(200, body))
        });
        assert!(result.ok, "{:?}", result.steps);
        assert_eq!(result.output, Some(serde_json::json!({"count": 3, "points": 10, "sprint": "12"})));
        assert_eq!(result.steps[1].label, "return");
    }

    #[test]
    fn cursor_paging_follows_the_cursor_until_it_runs_out() {
        let mut s = step("http://h/items?after={{page.cursor?}}");
        s.paginate = Some(Paginate { style: Some("cursor".into()), items: Some("json:$.data".into()), next: Some("json:$.next".into()), ..Default::default() });
        s.collect = Some("all".into());
        let mut cursors = Vec::new();
        let (result, _) = run_timed(&[s, Step { output: Some(".all".into()), ..Default::default() }], &BTreeMap::new(), |p| {
            cursors.push(var(p, "page.cursor"));
            Ok(match var(p, "page.cursor").as_deref() {
                None => reply(200, r#"{"data": [1, 2], "next": "c2"}"#),
                Some("c2") => reply(200, r#"{"data": [3], "next": "c3"}"#),
                _ => reply(200, r#"{"data": [4], "next": null}"#),
            })
        });
        assert_eq!(cursors, [None, Some("c2".to_string()), Some("c3".to_string())]);
        assert_eq!(result.output, Some(serde_json::json!([1, 2, 3, 4])));
    }

    #[test]
    fn link_paging_follows_the_next_url() {
        let mut s = step("http://h/repos");
        s.paginate = Some(Paginate { style: Some("link".into()), ..Default::default() });
        s.collect = Some("repos".into());
        let mut urls = Vec::new();
        let (result, _) = run_timed(&[s, Step { output: Some(".repos|length".into()), ..Default::default() }], &BTreeMap::new(), |p| {
            urls.push(p.url.clone().unwrap());
            Ok(if p.url.as_deref() == Some("http://h/repos") {
                reply_with(200, "[1, 2]", &[("Link", r#"<http://h/repos?page=2>; rel="next", <http://h/repos?page=2>; rel="last""#)])
            } else {
                reply_with(200, "[3]", &[("Link", r#"<http://h/repos?page=1>; rel="prev""#)])
            })
        });
        assert_eq!(urls, ["http://h/repos", "http://h/repos?page=2"]);
        assert_eq!(result.output, Some(serde_json::json!(3)));
    }

    #[test]
    fn paging_stops_at_max_pages_and_says_so() {
        let mut s = step("http://h/forever?p={{page.number}}");
        s.paginate = Some(Paginate { max_pages: Some(3), ..Default::default() });
        let (result, _) = run_timed(&[s], &BTreeMap::new(), |_| Ok(reply(200, "[1]")));
        assert!(result.ok);
        assert_eq!(result.steps[0].requests, 3);
        assert!(result.steps[0].warning.as_deref().unwrap().contains("3 pages"), "{:?}", result.steps[0].warning);
    }

    #[test]
    fn a_page_that_fails_names_the_page() {
        let mut s = step("http://h/x");
        s.paginate = Some(Paginate::default());
        let mut n = 0;
        let (result, _) = run_timed(&[s], &BTreeMap::new(), |_| {
            n += 1;
            Ok(if n == 1 { reply(200, "[1]") } else { reply(500, "[]") })
        });
        assert!(!result.ok);
        assert_eq!(result.steps[0].error.as_deref(), Some("page 2: expected 2xx, got 500"));
    }

    #[test]
    fn for_each_sends_once_per_element_with_its_fields() {
        let mut list = step("http://h/search");
        list.paginate = Some(Paginate { items: Some("json:$.issues".into()), page_size: Some(100), ..Default::default() });
        list.collect = Some("issues".into());
        let mut per = step("http://h/issue/{{issue.key}}/worklog");
        per.for_each = Some("issues".into());
        per.item_name = Some("issue".into());
        per.collect = Some("worklogs".into());
        let report = Step { output: Some("{keys: [.issues[].key], hours: ([.worklogs[].hours]|add)}".into()), ..Default::default() };
        let mut sent = Vec::new();
        let (result, _) = run_timed(&[list, per, report], &BTreeMap::new(), |p| {
            let url = p.url.clone().unwrap();
            let key = var(p, "issue.key");
            sent.push((key.clone(), var(p, "loop.number")));
            Ok(if url.ends_with("/search") {
                reply(200, r#"{"issues": [{"key": "A-1"}, {"key": "A-2"}]}"#)
            } else if key.as_deref() == Some("A-1") {
                reply(200, r#"{"hours": 2}"#)
            } else {
                reply(200, r#"{"hours": 5}"#)
            })
        });
        assert!(result.ok, "{:?}", result.steps);
        assert_eq!(sent[1], (Some("A-1".to_string()), Some("1".to_string())));
        assert_eq!(sent[2], (Some("A-2".to_string()), Some("2".to_string())));
        assert_eq!(result.output, Some(serde_json::json!({"keys": ["A-1", "A-2"], "hours": 7})));
        assert_eq!(result.steps[1].requests, 2);
    }

    #[test]
    fn a_failing_element_ends_the_run_unless_told_to_continue() {
        let make = |continue_on_error: bool| {
            let mut per = step("http://h/x/{{id}}");
            per.for_each = Some("[1, 2, 3]".into());
            per.item_name = Some("id".into());
            per.collect = Some("got".into());
            per.continue_on_error = continue_on_error;
            vec![per, Step { output: Some(".got".into()), ..Default::default() }]
        };
        let send = |p: &SendParams| Ok(if var(p, "id").as_deref() == Some("2") { reply(404, "{}") } else { reply(200, r#"{"ok": true}"#) });
        let (strict, _) = run_timed(&make(false), &BTreeMap::new(), send);
        assert!(!strict.ok);
        assert_eq!(strict.steps[0].error.as_deref(), Some("id 2 of 3: expected 2xx, got 404"));
        let (lenient, _) = run_timed(&make(true), &BTreeMap::new(), send);
        assert!(lenient.ok);
        assert!(lenient.steps[0].warning.as_deref().unwrap().starts_with("1 of 3 elements failed"));
        assert_eq!(lenient.output, Some(serde_json::json!([{"ok": true}, null, {"ok": true}])));
    }

    #[test]
    fn polling_repeats_until_the_condition_holds() {
        let mut s = step("http://h/jobs/1");
        s.request.extract = vec![Extract { name: "state".into(), from: "json:$.state".into(), secret: None, scope: Some("run".into()) }];
        s.poll = Some(Poll { until: "{{state}} == DONE".into(), every_ms: Some(250), max_attempts: Some(5) });
        let mut n = 0;
        let (result, sleeps) = run_timed(&[s], &BTreeMap::new(), |_| {
            n += 1;
            Ok(reply(200, if n < 3 { r#"{"state": "RUNNING"}"# } else { r#"{"state": "DONE"}"# }))
        });
        assert!(result.ok);
        assert_eq!(result.steps[0].requests, 3);
        assert_eq!(sleeps, [250, 250]);
    }

    #[test]
    fn polling_gives_up_and_says_what_never_held() {
        let mut s = step("http://h/jobs/1");
        s.request.extract = vec![Extract { name: "state".into(), from: "json:$.state".into(), secret: None, scope: Some("run".into()) }];
        s.poll = Some(Poll { until: "{{state}} == DONE".into(), every_ms: Some(1), max_attempts: Some(3) });
        let (result, _) = run_timed(&[s], &BTreeMap::new(), |_| Ok(reply(200, r#"{"state": "RUNNING"}"#)));
        assert!(!result.ok);
        assert_eq!(result.steps[0].error.as_deref(), Some("gave up after 3 attempts: `{{state}} == DONE` never held"));
        assert_eq!(result.steps[0].requests, 3);
    }

    #[test]
    fn a_rate_limit_is_waited_out_and_retried() {
        let mut s = step("http://h/limited");
        s.retry = Some(Retry { attempts: Some(3), backoff_ms: Some(100), ..Default::default() });
        let mut n = 0;
        let (result, sleeps) = run_timed(&[s], &BTreeMap::new(), |_| {
            n += 1;
            Ok(match n {
                1 => reply_with(429, "{}", &[("Retry-After", "2")]),
                2 => reply(503, "{}"),
                _ => reply(200, "{}"),
            })
        });
        assert!(result.ok);
        assert_eq!(result.steps[0].requests, 3);
        assert_eq!(sleeps, [2000, 200], "Retry-After first, then the back-off for the second try");
    }

    #[test]
    fn retries_run_out_and_the_last_answer_counts() {
        let mut s = step("http://h/down");
        s.retry = Some(Retry { attempts: Some(2), backoff_ms: Some(10), ..Default::default() });
        let (result, sleeps) = run_timed(&[s], &BTreeMap::new(), |_| Ok(reply(503, "{}")));
        assert!(!result.ok);
        assert_eq!(result.steps[0].error.as_deref(), Some("expected 2xx, got 503"));
        assert_eq!(sleeps.len(), 1);
        // no retry configured: no second request
        let (plain, _) = run_timed(&[step("http://h/down")], &BTreeMap::new(), |_| Ok(reply(503, "{}")));
        assert_eq!(plain.steps[0].requests, 1);
    }

    #[test]
    fn a_dropped_connection_is_retried_but_a_refusal_to_send_is_not() {
        let mut s = step("http://h/flaky");
        s.retry = Some(Retry { attempts: Some(3), backoff_ms: Some(5), ..Default::default() });
        let mut n = 0;
        let (result, _) = run_timed(&[s.clone()], &BTreeMap::new(), |_| {
            n += 1;
            if n == 1 { Err(SendFailure::Failed("connection reset".into())) } else { Ok(reply(200, "{}")) }
        });
        assert!(result.ok);
        let mut calls = 0;
        let (refused, _) = run_timed(&[s], &BTreeMap::new(), |_| {
            calls += 1;
            Err(SendFailure::NotSent("Undefined variable".into()))
        });
        assert!(!refused.ok);
        assert_eq!(calls, 1, "an undefined variable cannot be fixed by trying again");
    }

    #[test]
    fn return_never_sees_or_shows_a_secret() {
        let mut login = step("http://h/login");
        login.request.extract = vec![Extract { name: "token".into(), from: "json:$.token".into(), secret: None, scope: Some("run".into()) }];
        login.collect = Some("session".into());
        let out = Step { output: Some("{has_token: (.token != null), session: .session, token: .token}".into()), ..Default::default() };
        let (result, _) = run_timed(&[login, out], &BTreeMap::new(), |_| Ok(reply(200, r#"{"token": "s3cr3t-value", "user": "ada"}"#)));
        assert!(result.ok);
        let shown = serde_json::to_string(&result.output).unwrap();
        assert!(!shown.contains("s3cr3t-value"), "{shown}");
        assert!(shown.contains("[masked]"), "a secret inside collected data is masked: {shown}");
        assert!(shown.contains(r#""has_token":false"#), "a secret-named value is not part of the input: {shown}");
    }

    #[test]
    fn a_return_that_fails_fails_the_step_with_the_reason() {
        let mut s = step("http://h/a");
        s.output = Some(".nope | error(\"no data\")".into());
        let (result, _) = run_timed(&[s], &BTreeMap::new(), |_| Ok(reply(200, "{}")));
        assert!(!result.ok);
        assert!(result.steps[0].error.as_deref().unwrap().contains("no data"));
    }

    #[test]
    fn flow_settings_are_checked_before_saving() {
        let bad = |edit: &dyn Fn(&mut Step)| {
            let mut s = step("http://h/a");
            edit(&mut s);
            check(&[s]).unwrap_err()
        };
        assert!(bad(&|s| { s.paginate = Some(Paginate::default()); s.poll = Some(Poll { until: "{{a}}".into(), ..Default::default() }); }).contains("cannot be used in one step"));
        assert!(bad(&|s| s.item_name = Some("x".into())).contains("`as`"));
        assert!(bad(&|s| s.continue_on_error = true).contains("for_each"));
        assert!(bad(&|s| s.collect = Some("a b".into())).contains("`collect`"));
        assert!(bad(&|s| s.output = Some(".a | map(".into())).contains("`return`"));
        assert!(bad(&|s| s.paginate = Some(Paginate { style: Some("cursor".into()), ..Default::default() })).contains("needs `next`"));
        assert!(bad(&|s| s.retry = Some(Retry { attempts: Some(99), ..Default::default() })).contains("`retry`"));
        let mut link = step("http://h/a");
        link.request.url = None;
        link.request.saved_request = Some("saved".into());
        link.paginate = Some(Paginate { style: Some("link".into()), ..Default::default() });
        assert!(check(&[link]).unwrap_err().contains("inline"));
        // a step with only a return is fine; one with nothing is not
        assert!(check(&[step("http://h/a"), Step { output: Some(".".into()), ..Default::default() }]).is_ok());
        assert!(check(&[Step::default()]).is_err());
        assert!(check(&[Step { for_each: Some("[1]".into()), output: Some(".".into()), ..Default::default() }]).unwrap_err().contains("no `url`"));
    }

    #[test]
    fn the_new_fields_round_trip_in_a_stored_workflow() {
        let json = r#"[{"url": "http://h/a", "paginate": {"style": "page", "page_size": 50}, "collect": "rows", "retry": {"attempts": 4}, "delay_ms": 100},
                       {"for_each": "rows", "as": "row", "url": "http://h/{{row.id}}", "continue_on_error": true},
                       {"return": "{n: (.rows|length)}"}]"#;
        let steps = parse_steps(json).unwrap();
        assert_eq!(steps[1].item_name.as_deref(), Some("row"));
        assert_eq!(steps[2].output.as_deref(), Some("{n: (.rows|length)}"));
        let back = serde_json::to_string(&steps).unwrap();
        assert!(back.contains("\"as\":\"row\"") && back.contains("\"return\":"), "{back}");
        assert!(check(&steps).is_ok());
    }
}
