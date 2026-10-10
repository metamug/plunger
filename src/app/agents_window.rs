//! File > Set up AI agents: adds Plunger's MCP server to Claude Code, Cursor, Kiro, Codex, Windsurf, VS Code
//! or Gemini CLI, and writes the steering that tells the agent to use Plunger instead of curl. It shows
//! exactly which files would change before anything is written.

use super::ApiTesterApp;
use crate::headless::install::{self, Agent, Change, Options, Scope, Via};
use crate::ui::theme::palette;
use eframe::egui;
use std::path::PathBuf;

/// The open dialog.
pub(super) struct AgentsDialog {
    chosen: [bool; install::ALL.len()],
    found: [bool; install::ALL.len()],
    pub(super) scope: Scope,
    project: Option<PathBuf>,
    via_uvx: bool,
    /// What the current choices would change; recomputed when a choice changes.
    preview: Vec<Change>,
    /// What the Install button did.
    done: Option<Vec<Change>>,
}

impl AgentsDialog {
    fn options(&self, dry_run: bool) -> Options {
        Options {
            scope: self.scope,
            via: if self.via_uvx { Via::Uvx } else { install::default_via_exe() },
            mcp: true,
            steering: true,
            dry_run,
        }
    }

    fn agents(&self) -> Vec<Agent> {
        install::ALL.iter().enumerate().filter(|(i, _)| self.chosen[*i]).map(|(_, a)| *a).collect()
    }

    pub(super) fn plan(&mut self) {
        let Some(home) = install::home_dir() else {
            self.preview.clear();
            return;
        };
        let project = self.project.clone().unwrap_or_default();
        // A project setup needs a folder first.
        self.preview = if self.scope == Scope::Project && self.project.is_none() {
            Vec::new()
        } else {
            install::install(&self.agents(), &home, &project, &self.options(true))
        };
    }
}

impl ApiTesterApp {
    pub(super) fn open_agents_dialog(&mut self) {
        let home = install::home_dir();
        let found = std::array::from_fn(|i| home.as_deref().is_some_and(|h| install::ALL[i].detected(h)));
        let mut dialog = AgentsDialog {
            chosen: found,
            found,
            scope: Scope::User,
            project: None,
            via_uvx: false,
            preview: Vec::new(),
            done: None,
        };
        dialog.plan();
        self.agents_dialog = Some(dialog);
    }

    pub(super) fn render_agents_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.agents_dialog.take() else { return };
        let mut open = true;
        let (mut install_now, mut close, mut changed) = (false, false, false);
        let mut note = None;
        egui::Window::new("Set up AI agents")
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Adds Plunger's MCP server to the tool, and writes steering that tells the agent to send HTTP requests through Plunger, not curl or Invoke-RestMethod.",
                    )
                    .weak(),
                );
                ui.add_space(6.0);
                for (i, agent) in install::ALL.iter().enumerate() {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut dialog.chosen[i], agent.label()).changed();
                        if dialog.found[i] {
                            ui.label(egui::RichText::new("found").small().color(palette().ok));
                        }
                    });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    changed |= ui.radio_value(&mut dialog.scope, Scope::User, "Every project").changed();
                    changed |= ui.radio_value(&mut dialog.scope, Scope::Project, "One project folder").changed();
                    if dialog.scope == Scope::Project && ui.button("Choose folder\u{2026}").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            dialog.project = Some(dir);
                            changed = true;
                        }
                    }
                });
                if dialog.scope == Scope::Project {
                    ui.label(
                        egui::RichText::new(dialog.project.as_ref().map_or("No folder chosen yet".to_string(), |p| p.display().to_string())).weak().small(),
                    );
                }
                ui.horizontal(|ui| {
                    ui.label("Start the server from");
                    changed |= ui.radio_value(&mut dialog.via_uvx, false, "this copy of Plunger").changed();
                    changed |= ui
                        .radio_value(&mut dialog.via_uvx, true, "uvx plunger-cli")
                        .on_hover_text("Needs Python's uv. The tool downloads Plunger the first time it starts the server, so there is nothing to keep in place.")
                        .changed();
                });
                ui.add_space(6.0);
                ui.separator();
                ui.label(egui::RichText::new(if dialog.done.is_some() { "Done" } else { "This will change" }).weak().small());
                let shown = dialog.done.as_ref().unwrap_or(&dialog.preview);
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    if shown.is_empty() {
                        ui.label(egui::RichText::new("Nothing selected yet.").weak());
                    }
                    for c in shown {
                        ui.horizontal_wrapped(|ui| {
                            let color = match c.action.as_str() {
                                "skipped" => palette().amber,
                                "unchanged" => palette().text_widget,
                                _ => palette().ok,
                            };
                            ui.label(egui::RichText::new(&c.action).color(color));
                            if c.file.is_empty() {
                                ui.label(format!("{} {}", c.agent, c.what));
                            } else {
                                ui.label(egui::RichText::new(&c.file).monospace().small());
                            }
                            if let Some(n) = &c.note {
                                ui.label(egui::RichText::new(n).weak().small());
                            }
                        });
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ready = dialog.done.is_none() && dialog.preview.iter().any(|c| c.action == "created" || c.action == "updated");
                    if ui.add_enabled(ready, egui::Button::new("Install")).clicked() {
                        install_now = true;
                    }
                    close = ui.button("Close").clicked();
                });
            });
        if changed {
            dialog.done = None;
            dialog.plan();
        }
        if install_now {
            if let Some(home) = install::home_dir() {
                let project = dialog.project.clone().unwrap_or_default();
                let results = install::install(&dialog.agents(), &home, &project, &dialog.options(false));
                let wrote = results.iter().filter(|c| c.action == "created" || c.action == "updated").count();
                note = Some(format!("Set up {wrote} file{}. Restart the tool to pick up the server.", if wrote == 1 { "" } else { "s" }));
                dialog.done = Some(results);
            }
        }
        if let Some(note) = note {
            self.notify(note);
        }
        if !(close || !open) {
            self.agents_dialog = Some(dialog);
        }
    }
}
