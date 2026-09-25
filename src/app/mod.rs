mod command_bar;
mod history_panel;
mod import_window;
mod request;
mod response_panel;

use crate::history::{History, HistoryEntry};
use crate::http::send_request;
use crate::model::{BodyMode, Outcome, ParsedRequest, PersistedState, RequestTab, ResponseTab, SendResult};
use crate::request::{build_request, normalize_url, parse_headers};
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
        Self::new(state, History::open().ok())
    }

    fn new(state: PersistedState, history: Option<History>) -> Self {
        let header_rows = parse_headers(&state.headers_text);
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
        self.render_ui(ctx, ctrl_enter);
    }
}

impl ApiTesterApp {
    /// Everything drawn each frame, separate from `update` so it can run headless in tests.
    fn render_ui(&mut self, ctx: &egui::Context, ctrl_enter: bool) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FieldKind, FormField, KeyValue, ResponseData, Variable};

    fn app(state: PersistedState) -> ApiTesterApp {
        ApiTesterApp::new(state, Some(History::in_memory()))
    }

    /// Runs a few real egui frames (layout and all) without a window.
    fn draw(app: &mut ApiTesterApp) {
        let ctx = egui::Context::default();
        theme::apply_theme(&ctx);
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.render_ui(ctx, false));
        }
    }

    fn busy_state() -> PersistedState {
        PersistedState {
            headers_text: "Accept: */*\nX-Trace: 1".into(),
            params: vec![KeyValue { key: "page".into(), value: "2".into(), enabled: true }],
            variables: vec![Variable { name: "host".into(), value: "localhost".into(), secret: false }],
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
            a.request_tab = tab;
            draw(&mut a);
        }
        assert!(a.state.params.last().unwrap().is_blank());
        assert_eq!(a.state.params.iter().filter(|p| p.is_blank()).count(), 1);
        assert!(a.state.variables.last().unwrap().is_blank());
        assert!(a.header_rows.last().unwrap().0.is_empty());
        assert_eq!(a.state.headers_text, "Accept: */*\nX-Trace: 1", "drawing must not alter the headers");
    }

    #[test]
    fn every_body_mode_draws_and_form_data_keeps_a_spare_row() {
        let mut a = app(busy_state());
        a.request_tab = RequestTab::Body;
        for mode in [BodyMode::None, BodyMode::Json, BodyMode::Multipart, BodyMode::UrlEncoded, BodyMode::Raw] {
            a.state.body_mode = mode;
            draw(&mut a);
        }
        // the last real row is a File, which counts as in use, so one blank follows it
        let fields = &a.state.multipart_fields;
        assert_eq!(fields.len(), 3);
        assert!(fields[2].is_blank() && fields[2].kind == FieldKind::Text);
        assert_eq!(fields[1].value, "C:/x/report.pdf");
    }

    #[test]
    fn invalid_and_variable_json_bodies_draw() {
        let mut a = app(busy_state());
        a.request_tab = RequestTab::Body;
        a.state.body_mode = BodyMode::Json;
        for body in ["{\"a\":", "{\"n\": {{count}}}", "", "   "] {
            a.state.json_body = body.into();
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
            a.outcome = outcome;
            for tab in [ResponseTab::Body, ResponseTab::Headers] {
                a.response_tab = tab;
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
        a.refresh_history();
        assert_eq!(a.history_entries.len(), 2);
        a.import.open = true;
        draw(&mut a);
    }

    #[test]
    fn an_undefined_variable_stops_the_send_and_names_it() {
        let mut a = app(PersistedState { url: "http://{{host}}/x/{{id}}".into(), ..Default::default() });
        a.trigger_send(&egui::Context::default());
        assert!(!a.is_loading(), "nothing should be in flight");
        let Outcome::Failed(msg) = &a.outcome else { panic!("expected a failure message") };
        assert!(msg.contains("{{host}}") && msg.contains("{{id}}"), "{msg}");
        assert_eq!(a.state.url, "http://{{host}}/x/{{id}}", "the template must be left untouched");
        assert!(a.history_entries.is_empty(), "a blocked send is not a request");
    }

    #[test]
    fn a_bare_host_gets_its_scheme_in_the_field_but_a_template_does_not() {
        let mut a = app(PersistedState { url: "localhost:3000/x".into(), ..Default::default() });
        a.trigger_send(&egui::Context::default());
        assert_eq!(a.state.url, "http://localhost:3000/x");
        a.cancel_send();
        assert!(!a.is_loading());

        a.state.url = "{{base}}/x".into();
        a.state.variables = vec![Variable { name: "base".into(), value: "localhost:1".into(), secret: false }];
        a.trigger_send(&egui::Context::default());
        assert_eq!(a.state.url, "{{base}}/x");
        a.cancel_send();
    }

    #[test]
    fn loading_history_keeps_session_variables_and_options() {
        let mut a = app(PersistedState {
            variables: vec![Variable { name: "keep".into(), value: "me".into(), secret: false }],
            insecure_tls: true,
            ..Default::default()
        });
        let entry = {
            let h = a.history.as_ref().unwrap();
            h.insert(&PersistedState { url: "http://old/x".into(), ..Default::default() }, Some(200), Some(1)).unwrap();
            h.list_recent(1).unwrap().remove(0)
        };
        a.load_history_entry(&entry);
        assert_eq!(a.state.url, "http://old/x");
        assert_eq!(a.state.variables[0].name, "keep");
        assert!(a.state.insecure_tls);
    }
}
