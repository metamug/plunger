//! The application shell: open request tabs, the sidebar's saved requests and
//! history, and the app-wide pieces (menu bar, status bar, import dialogs,
//! credential store). Each panel lives in its own module.

mod chrome;
mod command_bar;
mod emboss;
mod agents_window;
mod export_window;
mod import_window;
mod lists;
mod request;
mod response_panel;
mod response_search;
mod shortcuts;
mod sidebar;
mod tab;
mod tabs;

pub use tab::SavedTab;

use crate::commands::Dialect;
use crate::history::{AgentVariable, History, HistoryEntry};
use crate::icons::{self, Icon};
use crate::model::{Outcome, ResponseTab, ParsedRequest, PersistedState};
use crate::secrets::{OsStore, SecretStore, SecretSync};
use crate::theme::{self, ThemeChoice};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tab::{CopiedFlash, Tab};

const HISTORY_LIMIT: i64 = 50;
const COPIED_FLASH: Duration = Duration::from_millis(1200);
const NOTICE_FOR: Duration = Duration::from_secs(4);
/// How often the window checks whether an agent (CLI or MCP) wrote to the
/// history, so its requests show up without clicking anything.
const DB_POLL: Duration = Duration::from_millis(1500);

/// eframe storage key for the open tabs (the legacy single-request state stays
/// under `eframe::APP_KEY`).
pub const TABS_KEY: &str = "tabs";
pub const SETTINGS_KEY: &str = "settings";

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OpenTabs {
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}

/// An icon button that copies `text` to the clipboard and briefly turns into
/// a check mark. A free function (not a method) so callers can pass text
/// borrowed from other fields.
fn copy_button(ui: &mut egui::Ui, flash: &mut CopiedFlash, key: &'static str, tooltip: &str, text: &str) {
    let fresh = flash.is_some_and(|(t, k)| k == key && t.elapsed() < COPIED_FLASH);
    let (icon, tip) = if fresh { (Icon::Check, "Copied") } else { (Icon::Copy, tooltip) };
    if icons::button(ui, icon, tip).clicked() {
        ui.output_mut(|o| o.copied_text = text.to_string());
        *flash = Some((Instant::now(), key));
    }
    if fresh {
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
}

/// The curl and HAR import dialogs. Only one is open at a time.
#[derive(Default)]
enum ImportDialog {
    #[default]
    Closed,
    Curl {
        text: String,
        error: Option<String>,
        /// Focus the text box on the first frame, so a paste goes straight in.
        focus: bool,
    },
    Har {
        candidates: Vec<(String, ParsedRequest)>,
    },
}

/// A sidebar row being renamed in place.
struct Rename {
    id: i64,
    /// Which sidebar list ("saved" / "history") shows the field.
    list: &'static str,
    text: String,
    /// Focus the field on the first frame only.
    focus: bool,
}

pub struct ApiTesterApp {
    tabs: Vec<Tab>,
    active: usize,
    next_tab_id: u64,

    // Deliberately NOT part of PersistedState / not written to disk — it's a
    // credential. Shared by every tab; kept in the OS credential store only
    // when "remember" is on.
    bearer_token: String,

    settings: Settings,

    history: Option<History>,
    /// The database's change counter when the lists were last read.
    db_version: Option<i64>,
    last_db_poll: Instant,
    history_entries: Vec<HistoryEntry>,
    saved_entries: Vec<HistoryEntry>,
    /// The sidebar's filter box; empty shows everything.
    sidebar_filter: String,
    /// What was last scanned for scripts the bundled fonts can't draw (see `fallback_fonts`).
    font_sig: (u64, [usize; 7]),
    renaming: Option<Rename>,
    saved_open: bool,
    history_open: bool,

    /// Variables an agent set (read from the shared database).
    agent_variables: Vec<AgentVariable>,
    import: ImportDialog,
    export: Option<export_window::ExportDialog>,
    agents_dialog: Option<agents_window::AgentsDialog>,
    /// The syntax Ctrl+Shift+C copies in: the one last chosen.
    export_dialect: Dialect,
    /// A short message for the status bar ("Saved …"), and when it was set.
    notice: Option<(String, Instant)>,

    secrets: Box<dyn SecretStore>,
    secret_sync: SecretSync,
    secrets_error: Option<String>,
}

impl ApiTesterApp {
    pub fn from_persisted(state: PersistedState, open_tabs: OpenTabs, settings: Settings) -> Self {
        let mut app = Self::new(state, History::open().ok(), Box::new(OsStore::new()));
        app.settings = settings;
        app.restore_tabs(open_tabs);
        app.restore_secrets();
        app
    }

    fn new(state: PersistedState, history: Option<History>, secrets: Box<dyn SecretStore>) -> Self {
        let mut app = Self {
            tabs: vec![Tab::new(0, state)],
            active: 0,
            next_tab_id: 0,
            bearer_token: String::new(),
            settings: Settings::default(),
            history,
            history_entries: Vec::new(),
            saved_entries: Vec::new(),
            sidebar_filter: String::new(),
            font_sig: (u64::MAX, [0; 7]),
            renaming: None,
            saved_open: true,
            history_open: true,
            db_version: None,
            last_db_poll: Instant::now(),
            agent_variables: Vec::new(),
            import: ImportDialog::default(),
            export: None,
            agents_dialog: None,
            export_dialect: Dialect::CurlBash,
            notice: None,
            secrets,
            secret_sync: SecretSync::default(),
            secrets_error: None,
        };
        app.refresh_lists();
        app
    }

    /// Reopens the tabs from last time. The legacy single state (already in
    /// `tabs[0]`) carries the latest session settings — variables, options —
    /// which every restored tab shares.
    fn restore_tabs(&mut self, open_tabs: OpenTabs) {
        if open_tabs.tabs.is_empty() {
            return;
        }
        let session = self.tabs[0].state.clone();
        self.tabs = open_tabs
            .tabs
            .into_iter()
            .map(|mut saved| {
                saved.state = saved.state.with_session_from(&session);
                self.next_tab_id += 1;
                Tab::from_saved(self.next_tab_id, saved)
            })
            .collect();
        self.active = open_tabs.active.min(self.tabs.len() - 1);
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
    }

    // ---- credential store -------------------------------------------------

    fn restore_secrets(&mut self) {
        let tab = &mut self.tabs[self.active];
        let problems = self.secret_sync.restore(&*self.secrets, &mut tab.state, &mut self.bearer_token);
        self.secrets_error = problems.into_iter().next();
    }

    /// Called from `save`: writes ticked secrets to the OS store, deletes unticked ones.
    fn sync_secrets(&mut self) {
        let tab = &self.tabs[self.active];
        let problems = self.secret_sync.persist(&*self.secrets, &tab.state, &self.bearer_token);
        self.secrets_error = problems.into_iter().next();
    }

    fn forget_secrets(&mut self) {
        let tab = &mut self.tabs[self.active];
        let problems = self.secret_sync.forget_all(&*self.secrets, &mut tab.state);
        self.secrets_error = problems.into_iter().next();
    }

    // ---- sending ----------------------------------------------------------

    fn trigger_send(&mut self, ctx: &egui::Context) {
        let bearer = self.bearer_token.clone();
        let agent_variables = self.agent_variable_values();
        self.tab_mut().send(&bearer, &agent_variables);
        ctx.request_repaint();
    }

    fn poll_responses(&mut self, ctx: &egui::Context) {
        let mut recorded = false;
        for tab in &mut self.tabs {
            if !tab.is_loading() {
                continue;
            }
            ctx.request_repaint();
            let Some(done) = tab.poll() else { continue };
            if let Some(h) = &self.history {
                if let Ok(id) = h.insert(&done.sent, done.status, done.elapsed_ms) {
                    // The request just sent is now the newest row; highlight it.
                    tab.history_id = Some(id);
                }
            }
            recorded = true;
        }
        if recorded {
            self.refresh_lists();
        }
    }

    fn set_theme(&mut self, ctx: &egui::Context, choice: ThemeChoice) {
        self.settings.theme = choice;
        theme::apply_theme(ctx, choice);
    }

}

impl eframe::App for ApiTesterApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Credentials must not land in the plaintext config file (the Bearer
        // field is never part of `state` at all).
        eframe::set_value(storage, eframe::APP_KEY, &self.tab().state.redacted());
        let open = OpenTabs {
            tabs: self.tabs.iter().map(Tab::to_saved).collect(),
            active: self.active,
        };
        eframe::set_value(storage, TABS_KEY, &open);
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        self.sync_secrets();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // egui_winit reacts to OS ThemeChanged events by resetting visuals to
        // the system theme; re-assert the chosen one.
        if !theme::is_applied(ctx, self.settings.theme) {
            theme::apply_theme(ctx, self.settings.theme);
        }
        self.poll_responses(ctx);
        self.poll_database(ctx);
        self.handle_shortcuts(ctx);
        self.scan_fonts(ctx);
        self.render_ui(ctx);
    }
}

impl ApiTesterApp {
    /// Loads a system font when the active request or the sidebar holds text the bundled fonts
    /// can't draw. Comparing a few lengths each frame tells whether anything changed; the
    /// text itself is only looked at then.
    fn scan_fonts(&mut self, ctx: &egui::Context) {
        let t = self.tab();
        let s = &t.state;
        let sig = (
            t.id,
            [
                s.url.len(),
                s.headers_text.len(),
                s.json_body.len(),
                s.raw_body.len(),
                s.urlencoded_body.len(),
                self.history_entries.len(),
                self.saved_entries.len(),
            ],
        );
        if sig == self.font_sig {
            return;
        }
        let mut text = format!("{}\n{}\n{}\n{}\n{}", s.url, s.headers_text, s.json_body, s.raw_body, s.urlencoded_body);
        for e in self.history_entries.iter().chain(&self.saved_entries).take(120) {
            text.push('\n');
            text.push_str(&e.url);
            text.push_str(e.name.as_deref().unwrap_or(""));
        }
        self.font_sig = sig;
        crate::fallback_fonts::ensure(ctx, &text);
    }

    /// The line between the request editor and the response. Drag it to give either more room;
    /// double-click to let the window decide again.
    fn render_divider(&mut self, ui: &mut egui::Ui) {
        let tab = &mut self.tabs[self.active];
        if tab.pane != tab::Pane::Both {
            ui.add_space(2.0);
            ui.separator();
            return;
        }
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 14.0), egui::Sense::click_and_drag());
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text("Drag to resize. Double-click to reset");
        if response.dragged() {
            let most = tab.request_shown_height + ui.available_height() - request::MIN_RESPONSE_HEIGHT;
            let current = tab.request_height.unwrap_or(tab.request_shown_height);
            tab.request_height = Some((current + response.drag_delta().y).clamp(request::MIN_REQUEST_HEIGHT, most.max(request::MIN_REQUEST_HEIGHT)));
        }
        if response.double_clicked() {
            tab.request_height = None;
        }
        let active = response.hovered() || response.dragged();
        let stroke = egui::Stroke::new(if active { 2.0_f32 } else { 1.0_f32 }, if active { theme::palette().accent_text } else { theme::palette().border });
        ui.painter().hline(rect.x_range(), rect.center().y, stroke);
        // A grip in the middle, so it is clear the line can be pulled; it lights up under the pointer.
        let grip = egui::Rect::from_center_size(rect.center(), egui::vec2(46.0, 6.0));
        let grip_color = if active { theme::palette().accent_text } else { theme::palette().text_widget.linear_multiply(0.5) };
        ui.painter().rect_filled(grip, egui::Rounding::same(3.0), theme::palette().panel);
        ui.painter().rect_stroke(grip, egui::Rounding::same(3.0), egui::Stroke::new(1.0_f32, grip_color));
        for dx in [-10.0_f32, 0.0, 10.0] {
            ui.painter().circle_filled(grip.center() + egui::vec2(dx, 0.0), 1.2, grip_color);
        }
    }

    /// Everything drawn each frame, separate from `update` so it can run headless in tests.
    fn render_ui(&mut self, ctx: &egui::Context) {
        self.render_menu_bar(ctx);
        self.render_status_bar(ctx);
        self.render_sidebar(ctx);
        self.render_import_windows(ctx);
        self.render_export_window(ctx);
        self.render_agents_window(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_tab_bar(ui);
            ui.add_space(4.0);
            self.render_command_bar(ui);
            self.tab_mut().sync_params_from_url();
            ui.add_space(6.0);
            self.render_request_section(ui);
            self.render_divider(ui);
            if self.tab().pane == tab::Pane::RequestExpanded {
                if ui.small_button("Show the response").on_hover_text("Give the response its space back").clicked() {
                    self.tab_mut().pane = tab::Pane::Both;
                }
            } else {
                self.tab_mut().render_response_section(ui);
            }
        });
    }
}

#[cfg(test)]
mod tests;
