//! Workflows in the window: the sidebar list with the last run's result, a window that shows a workflow's
//! steps, runs it with live progress and shows the result of earlier runs (also those an agent made).

use super::*;
use crate::history::{Source, WorkflowRunRow, WORKFLOW_RUNS_KEPT};
use crate::theme::{palette, status_badge};
use crate::workflow::{self, Progress, Step, StoredStep, WorkflowResult};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

/// A workflow as the sidebar lists it.
pub(super) struct WorkflowItem {
    pub name: String,
    pub steps: Vec<Step>,
    pub last: Option<WorkflowRunRow>,
}

/// The open workflow window.
pub(super) struct WorkflowView {
    pub name: String,
    /// `name=value` lines applied to every step of the next run.
    values: String,
    /// Recorded runs, newest first.
    runs: Vec<WorkflowRunRow>,
    /// The id of the run shown under "Last run"; None means the newest.
    selected: Option<i64>,
    /// The final response of a run made from this window (it is not stored for other runs).
    final_response: Option<crate::engine::AgentResponse>,
    error: Option<String>,
}

enum Event {
    Progress(Progress),
    Done(Box<Result<WorkflowResult, String>>),
}

enum StepState {
    Pending,
    Running,
    Done(StoredStep),
}

/// A run in progress, on its own thread.
pub(super) struct WorkflowRun {
    name: String,
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    steps: Vec<StepState>,
}

/// A small status dot, painted (the bundled fonts have no round-bullet glyph).
fn dot(ui: &mut egui::Ui, color: egui::Color32, filled: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    if filled {
        ui.painter().circle_filled(rect.center(), 4.0, color);
    } else {
        ui.painter().circle_stroke(rect.center(), 4.0, egui::Stroke::new(1.2_f32, color));
    }
}

/// The `name=value` lines of the values box, one per line; lines without `=` are ignored.
fn parse_values(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// "POST {{base}}/login" or the saved request's name.
fn step_title(step: &Step) -> (String, String) {
    match (&step.request.saved_request, &step.request.url) {
        (Some(saved), None) => ("SAVED".to_string(), saved.clone()),
        (_, url) => (
            step.request.method.clone().unwrap_or_else(|| "GET".to_string()).to_ascii_uppercase(),
            url.clone().unwrap_or_default(),
        ),
    }
}

/// The sidebar section. Returns the name of the workflow that was clicked.
pub(super) fn sidebar_section(ui: &mut egui::Ui, items: &[WorkflowItem], open: &mut bool, selected: Option<&str>) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    ui.add_space(8.0);
    super::sidebar::section_header(ui, "WORKFLOWS", items.len(), open, |_| {});
    let mut clicked = None;
    if *open {
        for item in items {
            let p = palette();
            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let is_selected = selected == Some(item.name.as_str());
            if is_selected {
                ui.painter().rect_filled(rect, egui::Rounding::same(5.0), p.accent_soft);
            } else if response.hovered() {
                ui.painter().rect_filled(rect, egui::Rounding::same(5.0), p.hover);
            }
            let dot = match &item.last {
                Some(run) if run.ok => p.ok,
                Some(run) if run.cancelled => p.amber,
                Some(_) => p.error,
                None => ui.visuals().weak_text_color(),
            };
            ui.painter().circle_filled(egui::pos2(rect.left() + 12.0, rect.center().y), 4.0, dot);
            let when = item.last.as_ref().map(|r| format!(" \u{b7} {}", crate::timefmt::ago(&r.started_at))).unwrap_or_default();
            let meta = format!("{} step{}{}", item.steps.len(), if item.steps.len() == 1 { "" } else { "s" }, when);
            let meta_galley = ui.painter().layout_no_wrap(meta, egui::FontId::proportional(11.0), ui.visuals().weak_text_color());
            let meta_pos = egui::pos2(rect.right() - meta_galley.size().x - 8.0, rect.center().y - meta_galley.size().y / 2.0);
            let name_width = (meta_pos.x - rect.left() - 28.0).max(20.0);
            let name_galley = crate::theme::one_line(ui, &item.name, egui::FontId::proportional(13.0), p.text, name_width);
            ui.painter().galley(egui::pos2(rect.left() + 24.0, rect.center().y - name_galley.size().y / 2.0), name_galley, p.text);
            ui.painter().galley(meta_pos, meta_galley, ui.visuals().weak_text_color());
            let tip = match &item.last {
                Some(run) => format!(
                    "{}\nLast run {} by {}: {}",
                    item.name,
                    crate::timefmt::full(&run.started_at),
                    run.source.as_str(),
                    if run.cancelled { "cancelled" } else if run.ok { "all steps passed" } else { "a step failed" }
                ),
                None => format!("{}\nNever run", item.name),
            };
            if response.on_hover_text(tip).clicked() {
                clicked = Some(item.name.clone());
            }
        }
    }
    clicked
}

impl ApiTesterApp {
    /// Re-reads the workflows and their latest runs (called whenever the database changes).
    pub(super) fn refresh_workflows(&mut self) {
        let Some(h) = &self.history else { return };
        let rows = h.list_workflows().unwrap_or_default();
        let latest = h.latest_workflow_runs().unwrap_or_default();
        let runs = self.workflow_view.as_ref().map(|v| h.workflow_runs(&v.name, WORKFLOW_RUNS_KEPT).unwrap_or_default());
        self.workflows = rows
            .into_iter()
            .filter_map(|(name, json)| {
                let steps = workflow::parse_steps(&json).ok()?;
                let last = latest.iter().find(|r| r.workflow == name).cloned();
                Some(WorkflowItem { name, steps, last })
            })
            .collect();
        if let (Some(view), Some(runs)) = (&mut self.workflow_view, runs) {
            view.runs = runs;
        }
        // a workflow that was deleted (by an agent, say) closes its window
        if self.workflow_view.as_ref().is_some_and(|v| !self.workflows.iter().any(|w| w.name == v.name)) {
            self.workflow_view = None;
        }
    }

    pub(super) fn open_workflow(&mut self, name: &str) {
        let runs = self
            .history
            .as_ref()
            .and_then(|h| h.workflow_runs(name, WORKFLOW_RUNS_KEPT).ok())
            .unwrap_or_default();
        self.workflow_view = Some(WorkflowView {
            name: name.to_string(),
            values: String::new(),
            runs,
            selected: None,
            final_response: None,
            error: None,
        });
    }

    pub(super) fn workflow_is_running(&self) -> bool {
        self.workflow_run.is_some()
    }

    /// Runs the open workflow on its own thread, with the same engine, variables and history as an
    /// agent's run.
    pub(super) fn start_workflow_run(&mut self, ctx: &egui::Context) {
        if self.workflow_run.is_some() {
            return;
        }
        let Some(view) = &mut self.workflow_view else { return };
        view.error = None;
        view.final_response = None;
        let name = view.name.clone();
        let values = parse_values(&view.values);
        let count = self.workflows.iter().find(|w| w.name == name).map_or(0, |w| w.steps.len());
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (thread_cancel, thread_ctx, thread_name) = (cancel.clone(), ctx.clone(), name.clone());
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = thread_ctx.clone();
            let result = workflow::run_reporting(&thread_name, &values, Source::Gui, &thread_cancel, &mut |p| {
                let _ = progress_tx.send(Event::Progress(p));
                progress_ctx.request_repaint();
            });
            let _ = tx.send(Event::Done(Box::new(result)));
            thread_ctx.request_repaint();
        });
        self.workflow_run = Some(WorkflowRun { name, rx, cancel, steps: (0..count).map(|_| StepState::Pending).collect() });
    }

    pub(super) fn cancel_workflow_run(&mut self) {
        if let Some(run) = &self.workflow_run {
            run.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Takes what the run thread has reported; called every frame.
    pub(super) fn poll_workflow_run(&mut self, ctx: &egui::Context) {
        let Some(run) = &mut self.workflow_run else { return };
        let mut finished = None;
        loop {
            match run.rx.try_recv() {
                Ok(Event::Progress(Progress::Started { step })) => {
                    if let Some(slot) = run.steps.get_mut(step - 1) {
                        *slot = StepState::Running;
                    }
                }
                Ok(Event::Progress(Progress::Finished(result))) => {
                    if let Some(slot) = run.steps.get_mut(result.step - 1) {
                        *slot = StepState::Done(StoredStep::from(&result));
                    }
                }
                Ok(Event::Done(result)) => {
                    finished = Some(*result);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = Some(Err("The run ended unexpectedly.".to_string()));
                    break;
                }
            }
        }
        if finished.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
            return;
        }
        let name = run.name.clone();
        self.workflow_run = None;
        match finished {
            Some(Ok(result)) => {
                let summary = if result.cancelled {
                    format!("Cancelled {name}")
                } else if result.ok {
                    format!("{name}: all {} steps passed", result.steps_run)
                } else {
                    format!("{name}: a step failed")
                };
                if let Some(view) = self.workflow_view.as_mut().filter(|v| v.name == name) {
                    view.final_response = result.last_response;
                    view.error = None;
                    view.selected = None;
                }
                self.notify(summary);
            }
            Some(Err(message)) => {
                if let Some(view) = self.workflow_view.as_mut() {
                    view.error = Some(message);
                }
            }
            None => {}
        }
        self.refresh_lists();
    }

    /// Opens a step as an ordinary request tab, to try by hand.
    fn open_workflow_step(&mut self, step: &Step) {
        let Some(history) = &self.history else { return };
        let session = crate::engine::Session::load();
        match step.request.to_state(&session, history) {
            Ok(state) => {
                let id = self.next_id();
                self.open_in_tab(Tab::new(id, state));
            }
            Err(message) => {
                if let Some(view) = &mut self.workflow_view {
                    view.error = Some(message);
                }
            }
        }
    }

    pub(super) fn render_workflow_window(&mut self, ctx: &egui::Context) {
        let Some(name) = self.workflow_view.as_ref().map(|v| v.name.clone()) else { return };
        let Some(steps) = self.workflows.iter().find(|w| w.name == name).map(|w| w.steps.clone()) else { return };
        let running = self.workflow_run.as_ref().filter(|r| r.name == name);
        let is_running = running.is_some();
        let live: Option<Vec<(Option<StoredStep>, bool)>> = running.map(|r| {
            r.steps
                .iter()
                .map(|s| match s {
                    StepState::Pending => (None, false),
                    StepState::Running => (None, true),
                    StepState::Done(done) => (Some(done.clone()), false),
                })
                .collect()
        });

        let mut open = true;
        let (mut run, mut cancel, mut copy) = (false, false, false);
        let mut open_step: Option<usize> = None;
        let mut select_run: Option<i64> = None;
        egui::Window::new(format!("Workflow: {name}"))
            .id(egui::Id::new("workflow-window"))
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .default_height(560.0)
            .open(&mut open)
            .show(ctx, |ui| {
                let view = self.workflow_view.as_mut().expect("checked above");
                ui.horizontal(|ui| {
                    if is_running {
                        if ui.button("Cancel").on_hover_text("Stop before the next step").clicked() {
                            cancel = true;
                        }
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(egui::RichText::new("Running\u{2026}").weak());
                    } else if ui.button("Run").on_hover_text("Send every step in order; the first failing step stops the run").clicked() {
                        run = true;
                    }
                    if ui.button("Copy as JSON").on_hover_text("The steps, as `plunger workflow save` and `save_workflow` take them").clicked() {
                        copy = true;
                    }
                });
                if let Some(error) = &view.error {
                    ui.colored_label(palette().error, error);
                }
                egui::CollapsingHeader::new("Values for this run").default_open(false).show(ui, |ui| {
                    ui.label(egui::RichText::new("One name=value per line; they apply to every step.").weak().small());
                    ui.add(
                        crate::theme::area(&mut view.values)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("username=demo"),
                    );
                });
                ui.add_space(4.0);
                ui.separator();
                ui.label(egui::RichText::new("STEPS").small().strong().color(ui.visuals().weak_text_color()));
                egui::ScrollArea::vertical().id_salt("workflow-steps").max_height(250.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, step) in steps.iter().enumerate() {
                        let (method, target) = step_title(step);
                        ui.horizontal(|ui| {
                            // a dot for the live state of this step while it runs
                            match live.as_ref().and_then(|l| l.get(i)) {
                                Some((Some(done), _)) => dot(ui, if done.ok { palette().ok } else { palette().error }, true),
                                Some((None, true)) => {
                                    ui.add(egui::Spinner::new().size(10.0));
                                }
                                Some((None, false)) => dot(ui, ui.visuals().weak_text_color(), false),
                                None => {}
                            }
                            ui.label(egui::RichText::new(format!("{}", i + 1)).weak());
                            let label = step.label.clone().unwrap_or_default();
                            if !label.is_empty() {
                                ui.label(egui::RichText::new(label).strong());
                            }
                            ui.label(egui::RichText::new(method).monospace().color(palette().accent_text));
                            ui.add(egui::Label::new(egui::RichText::new(target).monospace()).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Open as request").on_hover_text("Open this step in a tab, to try it by hand").clicked() {
                                    open_step = Some(i);
                                }
                            });
                        });
                        let mut notes: Vec<String> = Vec::new();
                        if let Some(headers) = &step.request.headers {
                            if !headers.is_empty() {
                                notes.push(format!("headers: {}", headers.keys().cloned().collect::<Vec<_>>().join(", ")));
                            }
                        }
                        if step.request.json.is_some() || step.request.body.is_some() || step.request.form.is_some() {
                            notes.push("with a body".to_string());
                        }
                        for e in &step.request.extract {
                            notes.push(format!("keeps {} from {}{}", e.name, e.from, if e.secret == Some(true) { " (secret)" } else { "" }));
                        }
                        if let Some(want) = step.expect_status {
                            notes.push(format!("expects {want}"));
                        }
                        if !notes.is_empty() {
                            ui.horizontal(|ui| {
                                ui.add_space(22.0);
                                ui.add(egui::Label::new(egui::RichText::new(notes.join("  \u{b7}  ")).weak().small()).wrap());
                            });
                        }
                        // the result of this step in the run in progress
                        if let Some((Some(done), _)) = live.as_ref().and_then(|l| l.get(i)) {
                            if let Some(error) = &done.error {
                                ui.horizontal(|ui| {
                                    ui.add_space(22.0);
                                    ui.colored_label(palette().error, error);
                                });
                            }
                        }
                        ui.add_space(4.0);
                    }
                });

                ui.add_space(4.0);
                ui.separator();
                ui.label(egui::RichText::new("LAST RUN").small().strong().color(ui.visuals().weak_text_color()));
                if view.runs.is_empty() {
                    ui.label(egui::RichText::new("Not run yet.").weak());
                } else {
                    // a short list of earlier runs to pick from
                    ui.horizontal_wrapped(|ui| {
                        for r in view.runs.iter().take(8) {
                            let chosen = view.selected.map_or(view.runs.first().map(|n| n.id) == Some(r.id), |id| id == r.id);
                            let mark = if r.cancelled { "cancelled" } else if r.ok { "passed" } else { "failed" };
                            let text = format!("{} \u{b7} {} \u{b7} {mark}", crate::timefmt::ago(&r.started_at), r.source.as_str());
                            if ui.selectable_label(chosen, text).clicked() {
                                select_run = Some(r.id);
                            }
                        }
                    });
                    let shown = view
                        .selected
                        .and_then(|id| view.runs.iter().find(|r| r.id == id))
                        .or_else(|| view.runs.first());
                    if let Some(r) = shown {
                        ui.label(
                            egui::RichText::new(format!("Started {} by {}", crate::timefmt::full(&r.started_at), r.source.as_str()))
                                .weak()
                                .small(),
                        );
                        let stored: Vec<StoredStep> = serde_json::from_str(&r.steps).unwrap_or_default();
                        egui::ScrollArea::vertical().id_salt("workflow-last-run").max_height(170.0).auto_shrink([false, true]).show(ui, |ui| {
                            for s in &stored {
                                ui.horizontal(|ui| {
                                    dot(ui, if s.ok { palette().ok } else { palette().error }, true);
                                    ui.label(format!("{}  {}", s.step, s.label));
                                    if let Some(status) = s.status {
                                        status_badge(ui, status, "");
                                    }
                                    if let Some(ms) = s.elapsed_ms {
                                        ui.label(egui::RichText::new(format!("{ms} ms")).weak());
                                    }
                                    if !s.set.is_empty() {
                                        ui.label(egui::RichText::new(format!("set {}", s.set.join(", "))).weak().small());
                                    }
                                });
                                if let Some(error) = &s.error {
                                    ui.horizontal(|ui| {
                                        ui.add_space(22.0);
                                        ui.colored_label(palette().error, error);
                                    });
                                }
                            }
                            if r.cancelled {
                                ui.label(egui::RichText::new("This run was cancelled before it finished.").weak());
                            }
                        });
                    }
                }
                if let Some(resp) = &view.final_response {
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new("Final response of this run").default_open(false).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            status_badge(ui, resp.status, &resp.status_text);
                            ui.label(egui::RichText::new(format!("{} ms \u{b7} {}", resp.elapsed_ms, crate::app::response_panel::format_bytes(resp.size_bytes))).weak());
                        });
                        let text = match (&resp.json, &resp.outline, &resp.body) {
                            (Some(json), _, _) => serde_json::to_string_pretty(json).unwrap_or_default(),
                            (_, Some(outline), _) => serde_json::to_string_pretty(outline).unwrap_or_default(),
                            (_, _, Some(body)) => body.clone(),
                            _ => String::new(),
                        };
                        let shown: String = text.chars().take(6000).collect();
                        egui::ScrollArea::vertical().id_salt("workflow-final").max_height(160.0).show(ui, |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(shown).monospace()).wrap());
                        });
                    });
                }
            });

        if let Some(id) = select_run {
            if let Some(view) = &mut self.workflow_view {
                view.selected = Some(id);
            }
        }
        if copy {
            let json = serde_json::to_string_pretty(&steps).unwrap_or_default();
            ctx.copy_text(json);
            self.notify("Copied the steps as JSON");
        }
        if let Some(i) = open_step {
            if let Some(step) = steps.get(i) {
                self.open_workflow_step(step);
            }
        }
        if cancel {
            self.cancel_workflow_run();
        }
        if run {
            self.start_workflow_run(ctx);
        }
        if !open {
            self.workflow_view = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_values;

    #[test]
    fn run_values_are_name_equals_value_lines() {
        let values = parse_values("username=demo\n  password = p w \nnot a pair\n=empty name\nurl=http://x/?a=b\n");
        assert_eq!(values.get("username").map(String::as_str), Some("demo"));
        assert_eq!(values.get("password").map(String::as_str), Some("p w"));
        assert_eq!(values.get("url").map(String::as_str), Some("http://x/?a=b"), "only the first = splits");
        assert_eq!(values.len(), 3);
    }
}
