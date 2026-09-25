mod command_bar;
mod history_panel;
mod import_window;
mod request_panel;
mod response_panel;

use crate::history::{History, HistoryEntry};
use crate::http::{build_request, normalize_url, parse_headers, send_request};
use crate::model::{BodyMode, Outcome, ParsedRequest, PersistedState, RequestTab, ResponseTab, SendResult};
use crate::theme::{self, card};
use eframe::egui;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

const HISTORY_LIMIT: i64 = 50;

/// The request lifecycle as one value, so "loading", "who to poll" and "what
/// was sent" can't drift out of sync with each other.
enum RequestStatus {
    Idle,
    InFlight {
        rx: Receiver<SendResult>,
        /// Snapshot taken when Send was clicked, so the history row reflects
        /// what was actually sent even if the form is edited before the
        /// response arrives.
        sent: Box<PersistedState>,
    },
}

#[derive(Default)]
struct ImportState {
    open: bool,
    curl_text: String,
    error: Option<String>,
    har_candidates: Vec<(String, ParsedRequest)>,
}

impl ImportState {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct ApiTesterApp {
    state: PersistedState,

    // Deliberately NOT part of PersistedState / not written to disk — it's a
    // credential, and silently persisting someone's auth token in a plaintext
    // local config file is the kind of thing that should be opt-in, not a
    // surprise. Cleared every time the app starts.
    bearer_token: String,

    request_tab: RequestTab,
    headers_as_text: bool,
    header_rows: Vec<(String, String)>,
    response_tab: ResponseTab,
    status: RequestStatus,
    outcome: Outcome,
    copied_flash: Option<Instant>,
    save_error: Option<String>,

    history: Option<History>,
    history_entries: Vec<HistoryEntry>,

    import: ImportState,
}

impl ApiTesterApp {
    pub fn from_persisted(state: PersistedState) -> Self {
        let header_rows = parse_headers(&state.headers_text);
        let history = History::open().ok();
        let history_entries = history
            .as_ref()
            .and_then(|h| h.list_recent(HISTORY_LIMIT).ok())
            .unwrap_or_default();
        Self {
            state,
            bearer_token: String::new(),
            request_tab: RequestTab::Params,
            headers_as_text: false,
            header_rows,
            response_tab: ResponseTab::Body,
            status: RequestStatus::Idle,
            outcome: Outcome::Empty,
            copied_flash: None,
            save_error: None,
            history,
            history_entries,
            import: ImportState::default(),
        }
    }

    fn is_loading(&self) -> bool {
        matches!(self.status, RequestStatus::InFlight { .. })
    }

    fn refresh_history(&mut self) {
        if let Some(h) = &self.history {
            if let Ok(entries) = h.list_recent(HISTORY_LIMIT) {
                self.history_entries = entries;
            }
        }
    }

    fn apply_parsed_request(&mut self, parsed: ParsedRequest) {
        self.state.method = parsed.method;
        self.state.url = parsed.url;
        self.state.headers_text = parsed
            .headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("\n");
        self.header_rows = parse_headers(&self.state.headers_text);

        match parsed.body {
            Some(body) if serde_json::from_str::<serde_json::Value>(&body).is_ok() => {
                self.state.body_mode = BodyMode::Json;
                self.state.json_body = body;
            }
            Some(body) => {
                self.state.body_mode = BodyMode::Raw;
                self.state.raw_body = body;
            }
            None if !parsed.form_fields.is_empty() => self.state.body_mode = BodyMode::Multipart,
            None => self.state.body_mode = BodyMode::None,
        }
        if !parsed.form_fields.is_empty() {
            self.state.multipart_fields = parsed.form_fields;
        }
        // Query params live in the URL for imports; the Params tab starts clean.
        self.state.params.clear();

        self.import.reset();
        self.outcome = Outcome::Empty;
    }

    fn load_history_entry(&mut self, entry: &HistoryEntry) {
        self.state = entry.to_persisted_state().with_session_from(&self.state);
        self.header_rows = parse_headers(&self.state.headers_text);
        self.outcome = Outcome::Empty;
    }

    fn trigger_send(&mut self, ctx: &egui::Context) {
        // Fill in a missing scheme in the field itself. A URL that uses
        // variables is left alone: the variable may supply the scheme, and the
        // field must keep the `{{template}}` rather than a resolved secret.
        if !self.state.url.contains("{{") {
            self.state.url = normalize_url(&self.state.url);
        }
        let req = match build_request(&self.state, &self.bearer_token) {
            Ok(req) => req,
            Err(message) => {
                self.outcome = Outcome::Failed(message);
                return;
            }
        };

        let (tx, rx) = std::sync::mpsc::channel();
        self.status = RequestStatus::InFlight {
            rx,
            sent: Box::new(self.state.clone()),
        };
        self.outcome = Outcome::Empty;
        self.save_error = None;
        send_request(req, tx);
        ctx.request_repaint();
    }

    /// Cancelling only stops the UI waiting: `reqwest::blocking` can't be
    /// interrupted mid-flight, so the background thread's result is dropped.
    fn cancel_send(&mut self) {
        self.status = RequestStatus::Idle;
    }

    fn poll_response(&mut self, ctx: &egui::Context) {
        let RequestStatus::InFlight { rx, .. } = &self.status else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint();
                return;
            }
            Err(TryRecvError::Disconnected) => Err("The request thread ended unexpectedly.".to_string()),
        };
        let RequestStatus::InFlight { sent, .. } = std::mem::replace(&mut self.status, RequestStatus::Idle) else {
            return;
        };

        if let Some(h) = &self.history {
            let _ = match &result {
                Ok(data) => h.insert(&sent, Some(data.status), Some(data.elapsed_ms)),
                Err(_) => h.insert(&sent, None, None),
            };
        }
        self.refresh_history();

        self.outcome = match result {
            Ok(data) => {
                self.response_tab = ResponseTab::Body;
                Outcome::Response(data)
            }
            Err(err) => Outcome::Failed(err),
        };
    }
}

impl eframe::App for ApiTesterApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Credentials must not land in the plaintext config file (the Bearer
        // field is never part of `state` at all).
        eframe::set_value(storage, eframe::APP_KEY, &self.state.redacted());
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // egui_winit reacts to OS ThemeChanged events by resetting visuals to
        // the system theme, which silently undid the one-time dark setup on
        // some Windows configurations. Re-asserting is cheap and makes dark
        // mode immune to that.
        if !ctx.style().visuals.dark_mode {
            theme::apply_theme(ctx);
        }

        self.poll_response(ctx);

        let ctrl_enter =
            ctx.input(|i| i.key_pressed(egui::Key::Enter) && (i.modifiers.ctrl || i.modifiers.command));

        self.render_history_panel(ctx);
        self.render_import_window(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(4.0);
            self.render_command_bar(ui, ctx, ctrl_enter);

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let params = self.state.params.iter().filter(|p| p.enabled && !p.key.is_empty()).count();
                let vars = self.state.variables.iter().filter(|v| !v.name.is_empty()).count();
                let count = |label: &str, n: usize| if n > 0 { format!("{label} ({n})") } else { label.to_string() };
                ui.selectable_value(&mut self.request_tab, RequestTab::Params, count("Params", params));
                ui.selectable_value(&mut self.request_tab, RequestTab::Headers, "Headers");
                ui.selectable_value(&mut self.request_tab, RequestTab::Body, "Body");
                ui.selectable_value(&mut self.request_tab, RequestTab::Variables, count("Variables", vars));
                let options_label = if self.state.insecure_tls { "Options (TLS check off)" } else { "Options" };
                ui.selectable_value(&mut self.request_tab, RequestTab::Options, options_label);
            });
            ui.add_space(4.0);
            card(ui, |ui| match self.request_tab {
                RequestTab::Params => self.render_params_tab(ui),
                RequestTab::Variables => self.render_variables_tab(ui),
                RequestTab::Headers => self.render_headers_tab(ui),
                RequestTab::Body => self.render_body_tab(ui),
                RequestTab::Options => self.render_options_tab(ui),
            });

            ui.add_space(10.0);
            ui.separator();
            self.render_response_section(ui, ctx);
        });
    }
}
