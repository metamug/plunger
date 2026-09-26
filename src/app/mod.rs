//! The application shell: open request tabs, the sidebar's saved requests and
//! history, and the app-wide pieces (menu bar, status bar, import dialogs,
//! credential store). Each panel lives in its own module.

mod chrome;
mod command_bar;
mod emboss;
mod import_window;
mod request;
mod response_panel;
mod sidebar;
mod tab;

pub use tab::SavedTab;

use crate::history::{History, HistoryEntry};
use crate::icons::{self, Icon};
use crate::model::{ParsedRequest, PersistedState};
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
    renaming: Option<Rename>,
    saved_open: bool,
    history_open: bool,

    import: ImportDialog,
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
            renaming: None,
            saved_open: true,
            history_open: true,
            db_version: None,
            last_db_poll: Instant::now(),
            import: ImportDialog::default(),
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

    // ---- tabs -------------------------------------------------------------

    fn next_id(&mut self) -> u64 {
        self.next_tab_id += 1;
        self.next_tab_id
    }

    /// Switches tabs. Variables and options are session-wide, so the tab being
    /// shown takes them over from the one being left.
    fn activate(&mut self, index: usize) {
        if index == self.active || index >= self.tabs.len() {
            return;
        }
        let session = self.tabs[self.active].state.clone();
        self.active = index;
        let tab = &mut self.tabs[index];
        tab.state = std::mem::take(&mut tab.state).with_session_from(&session);
    }

    fn push_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
        self.activate(self.tabs.len() - 1);
    }

    fn new_tab(&mut self) {
        let id = self.next_id();
        let tab = Tab::blank(id, &self.tab().state);
        self.push_tab(tab);
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            // Never leave zero tabs: closing the last one leaves a blank one.
            let id = self.next_id();
            self.tabs[0] = Tab::blank(id, &self.tabs[0].state);
            return;
        }
        let session = self.tabs[self.active].state.clone();
        self.tabs.remove(index);
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        let tab = &mut self.tabs[self.active];
        tab.state = std::mem::take(&mut tab.state).with_session_from(&session);
    }

    /// Replaces the current tab if it is an untouched blank one, otherwise opens a new tab.
    fn open_in_tab(&mut self, mut tab: Tab) {
        if self.tab().is_pristine() {
            let session = self.tab().state.clone();
            tab.state = std::mem::take(&mut tab.state).with_session_from(&session);
            self.tabs[self.active] = tab;
        } else {
            self.push_tab(tab);
        }
    }

    /// Opens a sidebar entry, or switches to the tab already showing it.
    fn open_entry(&mut self, entry: &HistoryEntry) {
        let existing = self.tabs.iter().position(|t| {
            t.history_id == Some(entry.id) || (entry.name.is_some() && t.saved_id == Some(entry.id))
        });
        match existing {
            Some(i) => self.activate(i),
            None => {
                let id = self.next_id();
                let tab = Tab::from_entry(id, entry, &self.tab().state);
                self.open_in_tab(tab);
            }
        }
    }

    fn open_parsed(&mut self, parsed: ParsedRequest) {
        let id = self.next_id();
        let mut tab = Tab::blank(id, &self.tab().state);
        tab.apply_parsed_request(parsed);
        self.open_in_tab(tab);
        self.import = ImportDialog::Closed;
    }

    // ---- sending ----------------------------------------------------------

    fn trigger_send(&mut self, ctx: &egui::Context) {
        let bearer = self.bearer_token.clone();
        self.tab_mut().send(&bearer);
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

    // ---- saved requests and history ---------------------------------------

    fn refresh_lists(&mut self) {
        if let Some(h) = &self.history {
            self.db_version = h.data_version();
            if let Ok(entries) = h.list_recent(HISTORY_LIMIT) {
                self.history_entries = entries;
            }
            if let Ok(entries) = h.list_saved() {
                self.saved_entries = entries;
            }
        }
    }

    /// Ctrl+S: writes the tab back to its saved request, or saves it as a new
    /// one and lets the user name it right away in the sidebar.
    fn save_active(&mut self) {
        let Some(h) = &self.history else {
            self.notify("Can't save: the local database couldn't be opened.");
            return;
        };
        let tab = &self.tabs[self.active];
        let name = tab.title();
        if let Some(id) = tab.saved_id {
            if h.update_request(id, &tab.state).unwrap_or(false) {
                self.refresh_lists();
                self.notify(format!("Saved \u{201c}{name}\u{201d}"));
                return;
            }
        }
        match h.save_new(&tab.state, &name) {
            Ok(id) => {
                let tab = &mut self.tabs[self.active];
                tab.saved_id = Some(id);
                tab.name = Some(name.clone());
                self.refresh_lists();
                self.saved_open = true;
                self.renaming = Some(Rename { id, list: "saved", text: name, focus: true });
                self.notify("Saved. Type a name for it and press Enter.");
            }
            Err(e) => self.notify(format!("Could not save: {e}")),
        }
    }

    fn start_rename(&mut self, entry: &HistoryEntry, list: &'static str) {
        let text = entry.name.clone().unwrap_or_default();
        self.renaming = Some(Rename { id: entry.id, list, text, focus: true });
    }

    /// Naming a row saves it; the tabs showing it take the new name.
    fn commit_rename(&mut self, id: i64, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(h) = &self.history {
            if h.set_name(id, Some(name)).is_err() {
                self.notify("Could not save the name.");
                return;
            }
        }
        for tab in &mut self.tabs {
            if tab.saved_id == Some(id) || tab.history_id == Some(id) {
                tab.saved_id = Some(id);
                tab.name = Some(name.to_string());
            }
        }
        self.refresh_lists();
        self.notify(format!("Saved \u{201c}{name}\u{201d}"));
    }

    fn unsave(&mut self, id: i64) {
        if let Some(h) = &self.history {
            let _ = h.set_name(id, None);
        }
        for tab in &mut self.tabs {
            if tab.saved_id == Some(id) {
                tab.saved_id = None;
                tab.name = None;
            }
        }
        self.refresh_lists();
    }

    fn clear_history(&mut self) {
        if let Some(h) = &self.history {
            let _ = h.clear();
        }
        self.refresh_lists();
    }

    /// Picks up requests an agent sent through the CLI or MCP server. One
    /// cheap PRAGMA every couple of seconds; the lists are only re-read when
    /// another process actually changed the database.
    fn poll_database(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(DB_POLL);
        if self.last_db_poll.elapsed() < DB_POLL {
            return;
        }
        self.last_db_poll = Instant::now();
        let current = self.history.as_ref().and_then(History::data_version);
        if current.is_some() && current != self.db_version {
            self.refresh_lists();
        }
    }

    fn set_theme(&mut self, ctx: &egui::Context, choice: ThemeChoice) {
        self.settings.theme = choice;
        theme::apply_theme(ctx, choice);
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let pressed = |key| {
            ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key)))
        };
        if pressed(egui::Key::Enter) && !self.tab().is_loading() {
            self.trigger_send(ctx);
        }
        if pressed(egui::Key::T) {
            self.new_tab();
        }
        if pressed(egui::Key::W) {
            self.close_tab(self.active);
        }
        if pressed(egui::Key::S) {
            self.save_active();
        }
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
        self.render_ui(ctx);
    }
}

impl ApiTesterApp {
    /// Everything drawn each frame, separate from `update` so it can run headless in tests.
    fn render_ui(&mut self, ctx: &egui::Context) {
        self.render_menu_bar(ctx);
        self.render_status_bar(ctx);
        self.render_sidebar(ctx);
        self.render_import_windows(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_tab_bar(ui);
            ui.add_space(6.0);
            self.render_command_bar(ui);
            self.tab_mut().sync_params_from_url();
            ui.add_space(12.0);
            self.render_request_section(ui);
            ui.add_space(10.0);
            ui.separator();
            self.tab_mut().render_response_section(ui);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BodyMode, FieldKind, FormField, KeyValue, Outcome, RequestTab, ResponseData, ResponseTab, Variable};
    use crate::request::build_request;
    use crate::secrets::test_support::MemoryStore;

    fn app(state: PersistedState) -> ApiTesterApp {
        ApiTesterApp::new(state, Some(History::in_memory()), Box::new(MemoryStore::default()))
    }

    /// Runs a few real egui frames (layout and all) without a window.
    fn draw(app: &mut ApiTesterApp) {
        let ctx = egui::Context::default();
        theme::apply_theme(&ctx, ThemeChoice::Dark);
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.render_ui(ctx));
        }
    }

    #[test]
    fn typing_a_query_into_the_url_fills_the_params_table() {
        let mut a = app(PersistedState::default());
        a.tab_mut().state.url = "https://h/x?include=events.player&q=a%20b".into();
        draw(&mut a);
        let named: Vec<(&str, &str)> =
            a.tab_mut().state.params.iter().filter(|p| !p.key.is_empty()).map(|p| (p.key.as_str(), p.value.as_str())).collect();
        assert_eq!(named, vec![("include", "events.player"), ("q", "a b")]);
    }

    #[test]
    fn editing_a_param_row_rewrites_the_url_query() {
        let mut a = app(PersistedState { url: "https://h/x?a=1#top".into(), ..Default::default() });
        a.tab_mut().state.params[0].value = "two words".into();
        a.tab_mut().state.params.push(KeyValue { key: "b".into(), value: "&".into(), enabled: true });
        a.tab_mut().render_params_rows_changed_for_test();
        assert_eq!(a.tab_mut().state.url, "https://h/x?a=two%20words&b=%26#top");
        // The URL change came from the table, so the next frame keeps the rows as they are.
        draw(&mut a);
        assert_eq!(a.tab_mut().state.params[1].value, "&");
    }

    #[test]
    fn unticking_a_row_drops_it_from_the_url_but_keeps_it_in_the_table() {
        let mut a = app(PersistedState { url: "https://h/x?a=1&b=2".into(), ..Default::default() });
        a.tab_mut().state.params[0].enabled = false;
        a.tab_mut().render_params_rows_changed_for_test();
        assert_eq!(a.tab_mut().state.url, "https://h/x?b=2");
        draw(&mut a);
        assert_eq!((a.tab().state.params[0].key.as_str(), a.tab().state.params[0].enabled), ("a", false));
    }

    #[test]
    fn old_saved_state_moves_its_params_into_the_url_once() {
        let mut a = app(PersistedState {
            url: "https://h/x".into(),
            params: vec![KeyValue { key: "page".into(), value: "2".into(), enabled: true }],
            ..Default::default()
        });
        draw(&mut a);
        assert_eq!(a.tab_mut().state.url, "https://h/x?page=2");
        let sent = build_request(&a.tab_mut().state, "").unwrap();
        assert_eq!(sent.url, "https://h/x?page=2");
    }

    fn busy_state() -> PersistedState {
        PersistedState {
            headers_text: "Accept: */*\nX-Trace: 1".into(),
            params: vec![KeyValue { key: "page".into(), value: "2".into(), enabled: true }],
            variables: vec![Variable { name: "host".into(), value: "localhost".into(), secret: false, remember: false }],
            multipart_fields: vec![
                FormField { key: "title".into(), kind: FieldKind::Text, value: "hi".into(), enabled: true },
                FormField { key: "doc".into(), kind: FieldKind::File, value: "C:/x/report.pdf".into(), enabled: true },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn every_request_tab_draws_and_keeps_one_spare_row() {
        let mut a = app(busy_state());
        for tab in [
            RequestTab::Params,
            RequestTab::Headers,
            RequestTab::Body,
            RequestTab::Variables,
            RequestTab::Options,
        ] {
            a.tab_mut().request_tab = tab;
            draw(&mut a);
        }
        assert!(a.tab_mut().state.params.last().unwrap().is_blank());
        assert_eq!(a.tab_mut().state.params.iter().filter(|p| p.is_blank()).count(), 1);
        assert!(a.tab_mut().state.variables.last().unwrap().is_blank());
        assert!(a.tab_mut().header_rows.last().unwrap().0.is_empty());
        assert_eq!(a.tab_mut().state.headers_text, "Accept: */*\nX-Trace: 1", "drawing must not alter the headers");
    }

    #[test]
    fn every_body_mode_draws_and_form_data_keeps_a_spare_row() {
        let mut a = app(busy_state());
        a.tab_mut().request_tab = RequestTab::Body;
        for mode in [BodyMode::None, BodyMode::Json, BodyMode::Multipart, BodyMode::UrlEncoded, BodyMode::Raw] {
            a.tab_mut().state.body_mode = mode;
            draw(&mut a);
        }
        // the last real row is a File, which counts as in use, so one blank follows it
        let fields = &a.tab_mut().state.multipart_fields;
        assert_eq!(fields.len(), 3);
        assert!(fields[2].is_blank() && fields[2].kind == FieldKind::Text);
        assert_eq!(fields[1].value, "C:/x/report.pdf");
    }

    #[test]
    fn invalid_and_variable_json_bodies_draw() {
        let mut a = app(busy_state());
        a.tab_mut().request_tab = RequestTab::Body;
        a.tab_mut().state.body_mode = BodyMode::Json;
        for body in ["{\"a\":", "{\"n\": {{count}}}", "", "   "] {
            a.tab_mut().state.json_body = body.into();
            draw(&mut a);
        }
    }

    fn response(body: &str, json: bool, truncated: bool) -> ResponseData {
        ResponseData {
            status: 200,
            status_text: "OK".into(),
            elapsed_ms: 12,
            size_bytes: body.len(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.into(),
            json_value: json.then(|| serde_json::from_str(body).unwrap()),
            truncated,
            total_size: truncated.then_some(99_999_999),
        }
    }

    #[test]
    fn every_response_state_draws() {
        let mut a = app(PersistedState::default());
        for outcome in [
            Outcome::Empty,
            Outcome::Failed("boom".into()),
            Outcome::Response(response("{\"a\":[1,2,{\"b\":null}]}", true, false)),
            Outcome::Response(response("plain text", false, false)),
            Outcome::Response(response("{\"a\":1}", true, true)),
        ] {
            a.tab_mut().outcome = outcome;
            for tab in [ResponseTab::Body, ResponseTab::Headers] {
                a.tab_mut().response_tab = tab;
                draw(&mut a);
            }
        }
    }

    #[test]
    fn history_and_import_panels_draw() {
        let mut a = app(PersistedState::default());
        if let Some(h) = &a.history {
            h.insert(&busy_state(), Some(200), Some(5)).unwrap();
            h.insert(&PersistedState { url: "https://exämple.com/ü/🚀/long/long/long/long/long/long".into(), ..Default::default() }, None, None)
                .unwrap();
        }
        a.refresh_lists();
        assert_eq!(a.history_entries.len(), 2);
        a.open_curl_dialog();
        draw(&mut a);
    }

    #[test]
    fn an_undefined_variable_stops_the_send_and_names_it() {
        let mut a = app(PersistedState { url: "http://{{host}}/x/{{id}}".into(), ..Default::default() });
        a.trigger_send(&egui::Context::default());
        assert!(!a.tab().is_loading(), "nothing should be in flight");
        let Outcome::Failed(msg) = &a.tab_mut().outcome else { panic!("expected a failure message") };
        assert!(msg.contains("{{host}}") && msg.contains("{{id}}"), "{msg}");
        assert_eq!(a.tab_mut().state.url, "http://{{host}}/x/{{id}}", "the template must be left untouched");
        assert!(a.history_entries.is_empty(), "a blocked send is not a request");
    }

    #[test]
    fn a_bare_host_gets_its_scheme_in_the_field_but_a_template_does_not() {
        let mut a = app(PersistedState { url: "localhost:3000/x".into(), ..Default::default() });
        a.trigger_send(&egui::Context::default());
        assert_eq!(a.tab_mut().state.url, "http://localhost:3000/x");
        a.tab_mut().cancel();
        assert!(!a.tab().is_loading());

        a.tab_mut().state.url = "{{base}}/x".into();
        a.tab_mut().state.variables = vec![Variable { name: "base".into(), value: "localhost:1".into(), secret: false, remember: false }];
        a.trigger_send(&egui::Context::default());
        assert_eq!(a.tab_mut().state.url, "{{base}}/x");
        a.tab_mut().cancel();
    }

    #[test]
    fn loading_history_keeps_session_variables_and_options() {
        let mut a = app(PersistedState {
            variables: vec![Variable { name: "keep".into(), value: "me".into(), secret: false, remember: false }],
            insecure_tls: true,
            ..Default::default()
        });
        let entry = {
            let h = a.history.as_ref().unwrap();
            h.insert(&PersistedState { url: "http://old/x".into(), ..Default::default() }, Some(200), Some(1)).unwrap();
            h.list_recent(1).unwrap().remove(0)
        };
        a.new_tab(); // not pristine any more once it has a URL
        a.tab_mut().state.url = "http://current".into();
        a.open_entry(&entry);
        assert_eq!(a.tab_mut().state.url, "http://old/x");
        assert_eq!(a.tab_mut().state.variables[0].name, "keep");
        assert!(a.tab_mut().state.insecure_tls);
    }

    fn remembering_app(store: &MemoryStore, state: PersistedState) -> ApiTesterApp {
        ApiTesterApp::new(state, Some(History::in_memory()), Box::new(store.clone()))
    }

    #[test]
    fn remembered_secrets_survive_a_restart_but_never_reach_the_state_file() {
        let store = MemoryStore::default();
        let mut first = remembering_app(
            &store,
            PersistedState {
                variables: vec![
                    Variable { name: "apiKey".into(), value: "SEKRET-VALUE".into(), secret: true, remember: true },
                    Variable { name: "forgetful".into(), value: "GONE-AFTER-RESTART".into(), secret: true, remember: false },
                    Variable { name: "host".into(), value: "localhost".into(), secret: false, remember: false },
                ],
                remember_bearer: true,
                ..Default::default()
            },
        );
        first.bearer_token = "BEARER-VALUE".into();
        first.sync_secrets();
        assert!(first.secrets_error.is_none());

        // what eframe would write to disk
        let on_disk = serde_json::to_string(&first.tab_mut().state.redacted()).unwrap();
        for secret in ["SEKRET-VALUE", "BEARER-VALUE", "GONE-AFTER-RESTART"] {
            assert!(!on_disk.contains(secret), "{secret} leaked into the state file: {on_disk}");
        }

        let mut second = remembering_app(&store, serde_json::from_str(&on_disk).unwrap());
        second.restore_secrets();
        assert_eq!(second.bearer_token, "BEARER-VALUE");
        assert_eq!(second.tab_mut().state.variables[0].value, "SEKRET-VALUE");
        assert_eq!(second.tab_mut().state.variables[1].value, "", "an un-remembered secret is blank after a restart");
        assert_eq!(second.tab_mut().state.variables[2].value, "localhost");
    }

    #[test]
    fn forgetting_secrets_empties_the_store_and_unticks_everything() {
        let store = MemoryStore::default();
        let mut a = remembering_app(
            &store,
            PersistedState {
                variables: vec![Variable { name: "tok".into(), value: "v".into(), secret: true, remember: true }],
                remember_bearer: true,
                ..Default::default()
            },
        );
        a.bearer_token = "b".into();
        a.sync_secrets();
        assert_eq!(store.data.borrow().len(), 2);

        a.forget_secrets();
        assert!(store.data.borrow().is_empty());
        assert!(!a.tab_mut().state.remember_bearer && !a.tab_mut().state.variables[0].remember);
        assert!(a.secrets_error.is_none());
    }

    #[test]
    fn an_unreadable_store_shows_an_error_keeps_the_data_and_the_ui_still_draws() {
        let store = MemoryStore::default();
        store.data.borrow_mut().insert("var:apiKey".into(), "precious".into());
        *store.fail_reads.borrow_mut() = true;
        let mut a = remembering_app(
            &store,
            PersistedState {
                variables: vec![Variable { name: "apiKey".into(), value: String::new(), secret: true, remember: true }],
                remember_bearer: true,
                ..Default::default()
            },
        );
        a.restore_secrets();
        assert!(a.secrets_error.as_deref().is_some_and(|e| e.contains("locked")));

        for tab in [RequestTab::Headers, RequestTab::Variables] {
            a.tab_mut().request_tab = tab;
            draw(&mut a);
        }
        a.sync_secrets(); // an autosave right now must not wipe the stored value
        assert_eq!(store.data.borrow()["var:apiKey"], "precious");
    }

    fn entry(a: &ApiTesterApp, url: &str) -> HistoryEntry {
        let h = a.history.as_ref().unwrap();
        h.insert(&PersistedState { url: url.into(), ..Default::default() }, Some(200), Some(1)).unwrap();
        h.list_recent(1).unwrap().remove(0)
    }

    #[test]
    fn opening_from_the_sidebar_reuses_a_blank_tab_then_adds_tabs_and_never_duplicates() {
        let mut a = app(PersistedState { url: String::new(), ..Default::default() });
        let one = entry(&a, "http://one");
        let two = entry(&a, "http://two");
        a.open_entry(&one);
        assert_eq!(a.tabs.len(), 1, "the blank tab is reused");
        a.open_entry(&two);
        assert_eq!((a.tabs.len(), a.active), (2, 1));
        a.open_entry(&one);
        assert_eq!((a.tabs.len(), a.active), (2, 0), "an already open request is switched to, not reopened");
    }

    #[test]
    fn new_and_closed_tabs_keep_session_settings_and_never_leave_zero_tabs() {
        let mut a = app(PersistedState {
            variables: vec![Variable { name: "host".into(), value: "h".into(), secret: false, remember: false }],
            ..Default::default()
        });
        a.new_tab();
        assert_eq!(a.tabs.len(), 2);
        assert_eq!(a.tab().state.url, "");
        assert_eq!(a.tab().state.variables[0].name, "host");
        a.tab_mut().state.variables[0].value = "changed".into();
        a.activate(0);
        assert_eq!(a.tab().state.variables[0].value, "changed", "variables follow you across tabs");
        a.close_tab(0);
        a.close_tab(0);
        assert_eq!(a.tabs.len(), 1);
        assert!(a.tab().is_pristine());
        draw(&mut a);
    }

    #[test]
    fn naming_a_history_row_saves_it_and_titles_its_tab() {
        let mut a = app(PersistedState { url: String::new(), ..Default::default() });
        let e = entry(&a, "http://h/fixtures/1");
        a.open_entry(&e);
        assert_eq!(a.tab().title(), "1");
        a.start_rename(&e, "history");
        draw(&mut a);
        a.commit_rename(e.id, "  Fixture one ");
        assert_eq!(a.saved_entries.len(), 1);
        assert_eq!(a.tab().title(), "Fixture one");
        assert_eq!(a.tab().saved_id, Some(e.id));
        a.unsave(e.id);
        assert!(a.saved_entries.is_empty());
        assert_eq!(a.tab().title(), "1");
    }

    #[test]
    fn saving_a_new_tab_creates_a_saved_request_and_saving_again_updates_it() {
        let mut a = app(PersistedState { url: "http://h/users".into(), ..Default::default() });
        a.save_active();
        assert_eq!(a.saved_entries.len(), 1);
        assert!(a.renaming.is_some(), "a new save asks for a name right away");
        let id = a.tab().saved_id.unwrap();
        a.tab_mut().state.url = "http://h/users?page=2".into();
        a.save_active();
        assert_eq!(a.saved_entries.len(), 1);
        assert_eq!(a.saved_entries[0].url, "http://h/users?page=2");
        assert_eq!(a.saved_entries[0].id, id);
        assert!(a.history_entries.is_empty(), "saving is not sending");
    }

    #[test]
    fn imported_requests_open_in_a_tab_and_the_dialogs_draw() {
        let mut a = app(PersistedState { url: "http://busy".into(), ..Default::default() });
        a.open_curl_dialog();
        if let ImportDialog::Curl { text, .. } = &mut a.import {
            *text = format!("curl 'https://h/x?q=1' {}", "-H 'X-Long: aaaaaaaaaaaaaaaaaaaa' ".repeat(80));
        }
        draw(&mut a);
        a.open_parsed(crate::curl_import::parse_curl("curl https://h/x?q=1").unwrap());
        assert_eq!(a.tabs.len(), 2);
        assert_eq!(a.tab().state.params[0].key, "q");
        assert!(matches!(a.import, ImportDialog::Closed));
    }

    #[test]
    fn open_tabs_survive_a_restart_without_their_secrets() {
        let mut a = app(PersistedState { url: "http://one".into(), ..Default::default() });
        a.new_tab();
        a.tab_mut().state.url = "http://two?api_key=SECRET".into();
        a.tab_mut().name = Some("Two".into());
        let open = OpenTabs { tabs: a.tabs.iter().map(Tab::to_saved).collect(), active: a.active };
        let json = serde_json::to_string(&open).unwrap();
        assert!(!json.contains("SECRET"));

        let mut b = app(PersistedState::default());
        b.restore_tabs(serde_json::from_str(&json).unwrap());
        assert_eq!((b.tabs.len(), b.active), (2, 1));
        assert_eq!(b.tab().title(), "Two");
        assert_eq!(b.tabs[0].state.url, "http://one");
    }

    #[test]
    fn requests_an_agent_sends_show_up_in_the_window_on_their_own() {
        let path = std::env::temp_dir().join(format!("plunger-live-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut a = ApiTesterApp::new(PersistedState::default(), Some(History::open_at(&path)), Box::new(MemoryStore::default()));
        assert!(a.history_entries.is_empty());

        // A separate connection, as `plunger mcp` would have.
        let agent = History::open_at(&path);
        agent
            .insert_from(&PersistedState { url: "http://agent/x".into(), ..Default::default() }, Some(200), Some(3), crate::history::Source::Mcp)
            .unwrap();

        a.last_db_poll = Instant::now() - DB_POLL * 2;
        a.poll_database(&egui::Context::default());
        assert_eq!(a.history_entries.len(), 1);
        assert_eq!(a.history_entries[0].source, crate::history::Source::Mcp);
        draw(&mut a); // the agent tag renders
    }

    #[test]
    fn both_themes_draw() {
        let mut a = app(busy_state());
        for choice in [ThemeChoice::Light, ThemeChoice::Dark] {
            let ctx = egui::Context::default();
            theme::apply_theme(&ctx, choice);
            let _ = ctx.run(egui::RawInput::default(), |ctx| a.render_ui(ctx));
        }
    }
}
