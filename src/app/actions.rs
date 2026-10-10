//! Everything the window can do from a menu or the keyboard, in one table. The menus, the keyboard
//! handler and Help > Keyboard shortcuts are all built from it, so a shortcut cannot be listed and
//! missing, or work without being in a menu.

use super::*;
use crate::model::RequestTab;
use crate::theme::ThemeChoice;

/// One thing a person can ask the window to do.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Action {
    // File
    NewTab,
    SaveRequest,
    CloseTab,
    ImportCommand,
    ImportHar,
    CopyAsMenu,
    CopyAsLast,
    ExportRequest,
    ClearHistory,
    Exit,
    // Edit
    CopyUrl,
    CopyResponseBody,
    FindInResponse,
    GoToUrl,
    // View
    ThemeMenu,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleSidebar,
    // Request
    Send,
    Cancel,
    RunWorkflow,
    DuplicateTab,
    NextTab,
    PreviousTab,
    ShowParams,
    ShowAuth,
    ShowHeaders,
    ShowBody,
    ShowVariables,
    ShowOptions,
    ExpandRequest,
    ExpandResponse,
    RestoreLayout,
    // Tools
    ShowWorkflows,
    SetUpAgents,
    // Help
    KeyboardShortcuts,
    Documentation,
    ReportIssue,
    About,
}

/// The menus, in order. `None` is a separator.
pub(super) const FILE_MENU: &[Option<Action>] = &[
    Some(Action::NewTab),
    Some(Action::SaveRequest),
    Some(Action::CloseTab),
    None,
    Some(Action::ImportCommand),
    Some(Action::ImportHar),
    None,
    Some(Action::CopyAsLast),
    Some(Action::CopyAsMenu),
    Some(Action::ExportRequest),
    None,
    Some(Action::ClearHistory),
    None,
    Some(Action::Exit),
];
pub(super) const EDIT_MENU: &[Option<Action>] = &[
    Some(Action::CopyUrl),
    Some(Action::CopyResponseBody),
    None,
    Some(Action::FindInResponse),
    Some(Action::GoToUrl),
];
pub(super) const VIEW_MENU: &[Option<Action>] = &[
    Some(Action::ThemeMenu),
    None,
    Some(Action::ZoomIn),
    Some(Action::ZoomOut),
    Some(Action::ZoomReset),
    None,
    Some(Action::ToggleSidebar),
];
pub(super) const REQUEST_MENU: &[Option<Action>] = &[
    Some(Action::Send),
    Some(Action::Cancel),
    Some(Action::RunWorkflow),
    None,
    Some(Action::ShowParams),
    Some(Action::ShowAuth),
    Some(Action::ShowHeaders),
    Some(Action::ShowBody),
    Some(Action::ShowVariables),
    Some(Action::ShowOptions),
    None,
    Some(Action::ExpandRequest),
    Some(Action::ExpandResponse),
    Some(Action::RestoreLayout),
    None,
    Some(Action::DuplicateTab),
    Some(Action::NextTab),
    Some(Action::PreviousTab),
];
pub(super) const TOOLS_MENU: &[Option<Action>] = &[Some(Action::ShowWorkflows), Some(Action::SetUpAgents)];
pub(super) const HELP_MENU: &[Option<Action>] = &[
    Some(Action::KeyboardShortcuts),
    Some(Action::Documentation),
    Some(Action::ReportIssue),
    None,
    Some(Action::About),
];

pub(super) const MENUS: &[(&str, &[Option<Action>])] = &[
    ("File", FILE_MENU),
    ("Edit", EDIT_MENU),
    ("View", VIEW_MENU),
    ("Request", REQUEST_MENU),
    ("Tools", TOOLS_MENU),
    ("Help", HELP_MENU),
];

const DOCS_URL: &str = "https://github.com/metamug/plunger#readme";
const AGENTS_DOCS_URL: &str = "https://github.com/metamug/plunger/blob/main/docs/agents.md";
const ISSUES_URL: &str = "https://github.com/metamug/plunger/issues/new";
const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.6;
const ZOOM_MAX: f32 = 2.5;

impl Action {
    pub(super) fn title(self) -> &'static str {
        match self {
            Action::NewTab => "New tab",
            Action::SaveRequest => "Save request",
            Action::CloseTab => "Close tab",
            Action::ImportCommand => "Import a command\u{2026}",
            Action::ImportHar => "Import a HAR file\u{2026}",
            Action::CopyAsMenu => "Copy as",
            Action::CopyAsLast => "Copy as last format",
            Action::ExportRequest => "Export request\u{2026}",
            Action::ClearHistory => "Clear history",
            Action::Exit => "Exit",
            Action::CopyUrl => "Copy URL",
            Action::CopyResponseBody => "Copy response body",
            Action::FindInResponse => "Find in response",
            Action::GoToUrl => "Go to URL",
            Action::ThemeMenu => "Theme",
            Action::ZoomIn => "Zoom in",
            Action::ZoomOut => "Zoom out",
            Action::ZoomReset => "Reset zoom",
            Action::ToggleSidebar => "Show or hide the sidebar",
            Action::Send => "Send",
            Action::Cancel => "Cancel the request",
            Action::RunWorkflow => "Run the open workflow",
            Action::ShowWorkflows => "Workflows",
            Action::DuplicateTab => "Duplicate tab",
            Action::NextTab => "Next tab",
            Action::PreviousTab => "Previous tab",
            Action::ShowParams => "Params",
            Action::ShowAuth => "Auth",
            Action::ShowHeaders => "Headers",
            Action::ShowBody => "Body",
            Action::ShowVariables => "Variables",
            Action::ShowOptions => "Options",
            Action::ExpandRequest => "Expand the request",
            Action::ExpandResponse => "Expand the response",
            Action::RestoreLayout => "Show both",
            Action::SetUpAgents => "Set up AI agents\u{2026}",
            Action::KeyboardShortcuts => "Keyboard shortcuts",
            Action::Documentation => "Documentation",
            Action::ReportIssue => "Report an issue",
            Action::About => "About Plunger",
        }
    }

    /// The keyboard shortcut, if it has one. `COMMAND` is Ctrl on Windows and Linux, Cmd on a Mac.
    pub(super) fn shortcut(self) -> Option<egui::KeyboardShortcut> {
        use egui::{Key, Modifiers};
        let cmd = Modifiers::COMMAND;
        let (mods, key) = match self {
            Action::NewTab => (cmd, Key::T),
            Action::SaveRequest => (cmd, Key::S),
            Action::CloseTab => (cmd, Key::W),
            Action::CopyAsLast => (cmd | Modifiers::SHIFT, Key::C),
            Action::FindInResponse => (cmd, Key::F),
            Action::GoToUrl => (cmd, Key::L),
            Action::ZoomIn => (cmd, Key::Equals),
            Action::ZoomOut => (cmd, Key::Minus),
            Action::ZoomReset => (cmd, Key::Num0),
            Action::ToggleSidebar => (cmd, Key::B),
            Action::Send => (cmd, Key::Enter),
            Action::RunWorkflow => (cmd | Modifiers::SHIFT, Key::R),
            Action::Cancel => (Modifiers::NONE, Key::Escape),
            Action::ShowParams => (cmd, Key::Num1),
            Action::ShowAuth => (cmd, Key::Num2),
            Action::ShowHeaders => (cmd, Key::Num3),
            Action::ShowBody => (cmd, Key::Num4),
            Action::ShowVariables => (cmd, Key::Num5),
            Action::ShowOptions => (cmd, Key::Num6),
            Action::NextTab => (cmd, Key::Tab),
            Action::PreviousTab => (cmd | Modifiers::SHIFT, Key::Tab),
            Action::KeyboardShortcuts => (Modifiers::NONE, Key::F1),
            _ => return None,
        };
        Some(egui::KeyboardShortcut::new(mods, key))
    }

    /// Every action, for the Help window and the tests.
    pub(super) fn all() -> Vec<Action> {
        MENUS.iter().flat_map(|(_, items)| items.iter().flatten().copied()).collect()
    }
}

/// A shortcut as people write it: `Ctrl+=` and `Esc` rather than egui's `Ctrl+Equals` and `Escape`.
fn shortcut_text(ctx: &egui::Context, shortcut: &egui::KeyboardShortcut) -> String {
    ctx.format_shortcut(shortcut).replace("Equals", "=").replace("Minus", "-").replace("Escape", "Esc")
}

/// `https://github.com/metamug/plunger/issues/new` with the version and system filled in.
fn issue_url() -> String {
    let body = format!(
        "**Plunger version:** {}\n**System:** {} {}\n\n**What happened**\n\n\n**What you expected**\n\n\n**Steps to reproduce**\n\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    format!("{ISSUES_URL}?body={}", crate::query::encode_value(&body))
}

impl ApiTesterApp {
    /// Whether `action` can be done right now (a menu entry is greyed out when not).
    pub(super) fn action_enabled(&self, action: Action) -> bool {
        let tab = self.tab();
        match action {
            Action::Send => !tab.is_loading(),
            Action::Cancel => tab.is_loading(),
            Action::RunWorkflow => self.workflow_view.is_some() && !self.workflow_is_running(),
            Action::CopyResponseBody | Action::FindInResponse => matches!(tab.outcome, Outcome::Response(_)),
            Action::CloseTab => true,
            Action::NextTab | Action::PreviousTab => self.tabs.len() > 1,
            _ => true,
        }
    }

    pub(super) fn run_action(&mut self, ctx: &egui::Context, action: Action) {
        if !self.action_enabled(action) {
            return;
        }
        match action {
            Action::NewTab => self.new_tab(),
            Action::SaveRequest => self.save_active(),
            Action::CloseTab => self.close_tab(self.active),
            Action::ImportCommand => self.open_curl_dialog(),
            Action::ImportHar => self.open_har_file(),
            Action::CopyAsMenu => {}
            Action::CopyAsLast => self.copy_request_as(ctx, self.export_dialect),
            Action::ExportRequest => self.open_export_dialog(),
            Action::ClearHistory => self.clear_history(),
            Action::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::CopyUrl => {
                ctx.copy_text(self.tab().state.url.clone());
                self.notify("Copied the URL");
            }
            Action::CopyResponseBody => {
                if let Outcome::Response(resp) = &self.tab().outcome {
                    let text = resp.raw_text.clone().unwrap_or_else(|| resp.body.clone());
                    ctx.copy_text(text);
                    self.notify("Copied the response body");
                }
            }
            Action::FindInResponse => {
                let tab = self.tab_mut();
                tab.response_tab = ResponseTab::Body;
                tab.response_search_open = true;
                tab.response_search_focus = true;
            }
            Action::GoToUrl => {
                let url_id = command_bar::url_field_id(self.tab().id);
                let url_length = self.tab().state.url.chars().count();
                ctx.memory_mut(|memory| memory.request_focus(url_id));
                let mut state = egui::text_edit::TextEditState::load(ctx, url_id).unwrap_or_default();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(url_length),
                )));
                state.store(ctx, url_id);
            }
            Action::ThemeMenu => {}
            Action::ZoomIn => self.set_zoom(ctx, self.settings.zoom + ZOOM_STEP),
            Action::ZoomOut => self.set_zoom(ctx, self.settings.zoom - ZOOM_STEP),
            Action::ZoomReset => self.set_zoom(ctx, 1.0),
            Action::ToggleSidebar => self.settings.show_sidebar = !self.settings.show_sidebar,
            Action::Send => self.trigger_send(ctx),
            Action::Cancel => {
                if self.renaming.is_none() {
                    self.tab_mut().cancel();
                }
            }
            Action::DuplicateTab => {
                let state = self.tab().state.clone();
                let id = self.next_id();
                self.push_tab(Tab::new(id, state));
            }
            Action::NextTab => {
                let next = (self.active + 1) % self.tabs.len();
                self.activate(next);
            }
            Action::PreviousTab => {
                let previous = (self.active + self.tabs.len() - 1) % self.tabs.len();
                self.activate(previous);
            }
            Action::ShowParams => self.show_request_tab(RequestTab::Params),
            Action::ShowAuth => self.show_request_tab(RequestTab::Auth),
            Action::ShowHeaders => self.show_request_tab(RequestTab::Headers),
            Action::ShowBody => self.show_request_tab(RequestTab::Body),
            Action::ShowVariables => self.show_request_tab(RequestTab::Variables),
            Action::ShowOptions => self.show_request_tab(RequestTab::Options),
            Action::ExpandRequest => self.tab_mut().pane = tab::Pane::RequestExpanded,
            Action::ExpandResponse => self.tab_mut().pane = tab::Pane::ResponseExpanded,
            Action::RestoreLayout => {
                let tab = self.tab_mut();
                tab.pane = tab::Pane::Both;
                tab.request_height = None;
            }
            Action::SetUpAgents => self.open_agents_dialog(),
            Action::ShowWorkflows => {
                // bring the list into view, and open the first workflow if none is open
                self.settings.show_sidebar = true;
                self.workflows_open = true;
                if self.workflow_view.is_none() {
                    if let Some(first) = self.workflows.first().map(|w| w.name.clone()) {
                        self.open_workflow(&first);
                    } else {
                        self.notify("No workflows yet: an agent can save one with save_workflow, or use plunger workflow save");
                    }
                }
            }
            Action::RunWorkflow => self.start_workflow_run(ctx),
            Action::KeyboardShortcuts => self.shortcuts_open = true,
            Action::Documentation => ctx.open_url(egui::OpenUrl::new_tab(DOCS_URL)),
            Action::ReportIssue => ctx.open_url(egui::OpenUrl::new_tab(issue_url())),
            Action::About => self.about_open = true,
        }
    }

    /// Opens a request tab, bringing the editor back if it was folded away.
    fn show_request_tab(&mut self, which: RequestTab) {
        let tab = self.tab_mut();
        tab.request_tab = which;
        if tab.pane == tab::Pane::ResponseExpanded {
            tab.pane = tab::Pane::Both;
        }
    }

    fn set_zoom(&mut self, ctx: &egui::Context, zoom: f32) {
        self.settings.zoom = (zoom * 10.0).round() / 10.0;
        self.settings.zoom = self.settings.zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        ctx.set_zoom_factor(self.settings.zoom);
        self.notify(format!("Zoom {}%", (self.settings.zoom * 100.0).round() as i32));
    }

    /// Every menu, built from the tables above.
    pub(super) fn render_menus(&mut self, ui: &mut egui::Ui) {
        for (name, items) in MENUS {
            ui.menu_button(*name, |ui| {
                for entry in items.iter() {
                    match entry {
                        None => {
                            ui.separator();
                        }
                        Some(Action::CopyAsMenu) => {
                            ui.menu_button("Copy as", |ui| {
                                for dialect in crate::commands::Dialect::ALL {
                                    if ui.button(dialect.label()).clicked() {
                                        self.copy_request_as(ui.ctx(), dialect);
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                        Some(Action::ThemeMenu) => {
                            ui.menu_button("Theme", |ui| {
                                let mut choice = self.settings.theme;
                                ui.radio_value(&mut choice, ThemeChoice::Dark, "Dark");
                                ui.radio_value(&mut choice, ThemeChoice::Light, "Light");
                                if choice != self.settings.theme {
                                    self.set_theme(ui.ctx(), choice);
                                    ui.close_menu();
                                }
                            });
                        }
                        Some(action) => {
                            let action = *action;
                            let shortcut = action.shortcut().map(|s| shortcut_text(ui.ctx(), &s)).unwrap_or_default();
                            let title = match action {
                                Action::CopyAsLast => format!("Copy as {}", self.export_dialect.label()),
                                Action::ToggleSidebar if self.settings.show_sidebar => "Hide the sidebar".to_string(),
                                Action::ToggleSidebar => "Show the sidebar".to_string(),
                                other => other.title().to_string(),
                            };
                            let enabled = self.action_enabled(action);
                            let clicked = ui.add_enabled(enabled, egui::Button::new(title).shortcut_text(shortcut)).clicked();
                            if clicked {
                                ui.close_menu();
                                self.run_action(ui.ctx(), action);
                            }
                        }
                    }
                }
            });
        }
    }

    /// The keyboard: every action with a shortcut, those needing more modifiers first so Ctrl+Shift+C is
    /// not taken for Ctrl+C.
    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let mut with_keys: Vec<(Action, egui::KeyboardShortcut)> =
            Action::all().into_iter().filter_map(|a| a.shortcut().map(|s| (a, s))).collect();
        with_keys.sort_by_key(|(_, s)| {
            let m = s.modifiers;
            std::cmp::Reverse(m.command as u8 + m.shift as u8 + m.alt as u8 + m.ctrl as u8)
        });
        for (action, shortcut) in with_keys {
            // Escape only means "cancel" while a request is in flight, so it stays free for the rest.
            if !self.action_enabled(action) {
                continue;
            }
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                self.run_action(ctx, action);
            }
        }
    }

    /// Help > Keyboard shortcuts and Help > About.
    pub(super) fn render_help_windows(&mut self, ctx: &egui::Context) {
        if self.shortcuts_open {
            let mut open = true;
            egui::Window::new("Keyboard shortcuts").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                egui::Grid::new("shortcuts").num_columns(2).spacing([28.0, 4.0]).show(ui, |ui| {
                    for (menu, items) in MENUS {
                        for action in items.iter().flatten() {
                            if let Some(shortcut) = action.shortcut() {
                                ui.label(format!("{}: {}", menu, action.title()));
                                ui.label(egui::RichText::new(shortcut_text(ctx, &shortcut)).monospace());
                                ui.end_row();
                            }
                        }
                    }
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Ctrl is Cmd on a Mac. Every action is also in the menus.").weak().small());
            });
            self.shortcuts_open = open;
        }
        if self.about_open {
            let mut open = true;
            egui::Window::new("About Plunger").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                ui.heading("Plunger");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("A quick, lightweight API client for you and your AI agent.");
                ui.label("MIT licence. No account, no cloud, no telemetry.");
                ui.add_space(6.0);
                ui.hyperlink_to("github.com/metamug/plunger", "https://github.com/metamug/plunger");
                ui.hyperlink_to("Using it with AI agents", AGENTS_DOCS_URL);
                ui.add_space(6.0);
                ui.label(egui::RichText::new(format!("Data folder: {}", crate::history::app_data_dir().display())).weak().small());
            });
            self.about_open = open;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_exactly_one_menu() {
        let mut seen = std::collections::HashMap::new();
        for (menu, items) in MENUS {
            for action in items.iter().flatten() {
                assert!(seen.insert(*action, *menu).is_none(), "{action:?} is in two menus");
            }
        }
        // the pseudo-entries that render submenus, plus every real action
        assert!(seen.contains_key(&Action::CopyAsMenu) && seen.contains_key(&Action::ThemeMenu));
        assert!(seen.len() >= 30, "{} actions", seen.len());
    }

    #[test]
    fn no_two_actions_share_a_shortcut() {
        let mut seen = std::collections::HashMap::new();
        for action in Action::all() {
            if let Some(s) = action.shortcut() {
                let key = format!("{:?}+{:?}", s.modifiers, s.logical_key);
                if let Some(other) = seen.insert(key, action) {
                    panic!("{action:?} and {other:?} share a shortcut");
                }
            }
        }
    }

    #[test]
    fn the_shortcuts_people_know_are_there() {
        let has = |a: Action| a.shortcut().is_some();
        for action in [Action::NewTab, Action::SaveRequest, Action::CloseTab, Action::Send, Action::Cancel, Action::GoToUrl, Action::FindInResponse, Action::NextTab, Action::PreviousTab, Action::CopyAsLast, Action::ZoomIn, Action::ZoomOut, Action::ZoomReset, Action::ToggleSidebar, Action::KeyboardShortcuts, Action::ShowParams, Action::ShowBody, Action::ShowVariables] {
            assert!(has(action), "{action:?} lost its shortcut");
        }
    }

    #[test]
    fn shortcuts_read_the_way_people_write_them() {
        // Formatting needs a frame (it asks which operating system this is), as in the window.
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let text = shortcut_text(ctx, &Action::ZoomIn.shortcut().unwrap());
            assert!(text.ends_with('=') && !text.contains("Equals"), "{text}");
            assert!(shortcut_text(ctx, &Action::ZoomOut.shortcut().unwrap()).ends_with('-'));
            assert_eq!(shortcut_text(ctx, &Action::Cancel.shortcut().unwrap()), "Esc");
        });
    }

    #[test]
    fn the_issue_link_carries_the_version() {
        let url = issue_url();
        assert!(url.starts_with("https://github.com/metamug/plunger/issues/new?body="));
        assert!(url.contains(&crate::query::encode_value(env!("CARGO_PKG_VERSION"))));
    }
}
