use crate::curl_import::{parse_curl, parse_har};
use crate::history::{History, HistoryEntry};
use crate::http::{format_bytes, parse_headers, send_request};
use crate::json_view::highlight_json;
use crate::model::{BodyMode, ParsedRequest, PersistedState, RequestTab, ResponseData, ResponseTab};
use crate::theme::{self, accented_card, card, copy_icon_button, status_badge, status_dot_color, ACCENT, AMBER};
use eframe::egui;
use egui_json_tree::{DefaultExpand, JsonTree};
use std::sync::mpsc::Receiver;
use std::time::Instant;

const COMMON_HEADERS: &[&str] = &[
    "Accept",
    "Accept-Encoding",
    "Accept-Language",
    "Authorization",
    "Cache-Control",
    "Content-Type",
    "Content-Length",
    "Cookie",
    "Host",
    "If-Match",
    "If-None-Match",
    "If-Modified-Since",
    "Origin",
    "Referer",
    "User-Agent",
    "X-Api-Key",
    "X-Correlation-Id",
    "X-Forwarded-For",
    "X-Request-Id",
    "X-Requested-With",
];

const COMMON_CONTENT_TYPES: &[&str] = &[
    "application/json",
    "application/xml",
    "application/x-www-form-urlencoded",
    "application/octet-stream",
    "multipart/form-data",
    "text/plain",
    "text/html",
    "text/csv",
];

/// Renders a row of small clickable suggestion buttons filtered by prefix
/// match against `target`'s current text. Returns true if a suggestion was
/// picked (i.e. `target` was just overwritten).
fn suggestion_chips(ui: &mut egui::Ui, target: &mut String, candidates: &[&str]) -> bool {
    let typed_lower = target.to_lowercase();
    let matches: Vec<&str> = candidates
        .iter()
        .filter(|c| c.to_lowercase().starts_with(&typed_lower) && c.to_lowercase() != typed_lower)
        .take(6)
        .copied()
        .collect();
    if matches.is_empty() {
        return false;
    }
    let mut picked = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("suggestions:").weak().small());
        for m in matches {
            if ui.small_button(m).clicked() {
                *target = m.to_string();
                picked = true;
            }
        }
    });
    picked
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
    is_loading: bool,
    response: Option<ResponseData>,
    error: Option<String>,
    rx: Option<Receiver<Result<ResponseData, String>>>,
    copied_flash: Option<Instant>,
    /// Snapshot of `state` at the moment Send was clicked, so the history
    /// row reflects what was actually sent even if the form is edited again
    /// before the response arrives.
    sent_state: Option<PersistedState>,

    history: Option<History>,
    history_entries: Vec<HistoryEntry>,

    show_import: bool,
    import_curl_text: String,
    import_error: Option<String>,
    har_candidates: Vec<(String, ParsedRequest)>,
}

impl ApiTesterApp {
    pub fn from_persisted(state: PersistedState) -> Self {
        let header_rows = parse_headers(&state.headers_text);
        let history = History::open().ok();
        let history_entries = history
            .as_ref()
            .and_then(|h| h.list_recent(50).ok())
            .unwrap_or_default();
        Self {
            state,
            bearer_token: String::new(),
            request_tab: RequestTab::Headers,
            headers_as_text: false,
            header_rows,
            response_tab: ResponseTab::Body,
            is_loading: false,
            response: None,
            error: None,
            rx: None,
            copied_flash: None,
            sent_state: None,
            history,
            history_entries,
            show_import: false,
            import_curl_text: String::new(),
            import_error: None,
            har_candidates: Vec::new(),
        }
    }

    fn refresh_history(&mut self) {
        if let Some(h) = &self.history {
            if let Ok(entries) = h.list_recent(50) {
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
            None => {
                self.state.body_mode = BodyMode::None;
            }
        }

        self.show_import = false;
        self.import_curl_text.clear();
        self.import_error = None;
        self.har_candidates.clear();
        self.response = None;
        self.error = None;
    }

    fn load_history_entry(&mut self, entry: &HistoryEntry) {
        self.state = entry.to_persisted_state();
        self.header_rows = parse_headers(&self.state.headers_text);
        self.response = None;
        self.error = None;
    }

    fn trigger_send(&mut self, ctx: &egui::Context) {
        let mut headers = parse_headers(&self.state.headers_text);
        let bearer = self.bearer_token.trim();
        if !bearer.is_empty() && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization")) {
            headers.push(("Authorization".to_string(), format!("Bearer {bearer}")));
        }
        let body = match self.state.body_mode {
            BodyMode::None => None,
            BodyMode::Json => {
                if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
                    headers.push(("Content-Type".to_string(), "application/json".to_string()));
                }
                Some(self.state.json_body.clone())
            }
            BodyMode::UrlEncoded => {
                if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
                    headers.push((
                        "Content-Type".to_string(),
                        "application/x-www-form-urlencoded".to_string(),
                    ));
                }
                Some(
                    self.state
                        .urlencoded_body
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join("&"),
                )
            }
            BodyMode::Raw => Some(self.state.raw_body.clone()),
        };

        self.sent_state = Some(self.state.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.is_loading = true;
        self.error = None;
        self.response = None;
        send_request(self.state.method.clone(), self.state.url.clone(), headers, body, tx);
        ctx.request_repaint();
    }

    fn render_history_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("history_panel")
            .resizable(true)
            .default_width(230.0)
            .width_range(160.0..=400.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("HISTORY").weak().small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Clear").clicked() {
                            if let Some(h) = &self.history {
                                let _ = h.clear();
                            }
                            self.refresh_history();
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();

                if self.history.is_none() {
                    ui.label(
                        egui::RichText::new("History unavailable (couldn't open local database).")
                            .weak()
                            .small(),
                    );
                    return;
                }

                let mut pick: Option<usize> = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, entry) in self.history_entries.iter().enumerate() {
                        let dot = status_dot_color(entry.status.map(|s| s as u16));
                        let response = ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 4.0, dot);
                            ui.add_space(2.0);
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&entry.method).small().strong());
                                let mut url_label = entry.url.clone();
                                if url_label.len() > 34 {
                                    url_label.truncate(31);
                                    url_label.push_str("...");
                                }
                                ui.label(egui::RichText::new(url_label).small().weak());
                            });
                        });
                        let row_response = ui
                            .interact(
                                response.response.rect,
                                ui.id().with(("history-row", entry.id)),
                                egui::Sense::click(),
                            )
                            .on_hover_text(format!(
                                "{}\n{}\n{}",
                                entry.url,
                                entry.created_at,
                                entry
                                    .elapsed_ms
                                    .map(|ms| format!("{ms} ms"))
                                    .unwrap_or_else(|| "no response".to_string()),
                            ));
                        if row_response.clicked() {
                            pick = Some(i);
                        }
                        if row_response.hovered() {
                            ui.painter().rect_filled(
                                response.response.rect,
                                egui::Rounding::same(4.0),
                                theme::CARD_FILL,
                            );
                        }
                        ui.add_space(3.0);
                    }
                    if self.history_entries.is_empty() {
                        ui.label(egui::RichText::new("No requests yet.").weak().small());
                    }
                });

                if let Some(i) = pick {
                    let entry = self.history_entries[i].clone();
                    self.load_history_entry(&entry);
                }
            });
    }

    fn render_import_window(&mut self, ctx: &egui::Context) {
        if !self.show_import {
            return;
        }
        let mut still_open = true;
        egui::Window::new("Import request")
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .open(&mut still_open)
            .show(ctx, |ui| {
                ui.label("Paste a curl command:");
                ui.add(
                    egui::TextEdit::multiline(&mut self.import_curl_text)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("curl 'https://api.example.com/resource' -H 'Authorization: Bearer ...' -d '{...}'"),
                );
                ui.horizontal(|ui| {
                    if ui.button("Parse curl").clicked() {
                        match parse_curl(&self.import_curl_text) {
                            Ok(parsed) => self.apply_parsed_request(parsed),
                            Err(e) => self.import_error = Some(e),
                        }
                    }
                    if ui.button("Import HAR file...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("HAR", &["har"]).pick_file() {
                            match parse_har(&path) {
                                Ok(entries) => {
                                    self.har_candidates = entries
                                        .into_iter()
                                        .map(|p| (format!("{} {}", p.method, p.url), p))
                                        .collect();
                                    self.import_error = if self.har_candidates.is_empty() {
                                        Some("No requests found in that HAR file.".to_string())
                                    } else {
                                        None
                                    };
                                }
                                Err(e) => self.import_error = Some(e),
                            }
                        }
                    }
                });

                if let Some(err) = &self.import_error {
                    ui.colored_label(egui::Color32::from_rgb(230, 100, 90), err);
                }

                if !self.har_candidates.is_empty() {
                    ui.separator();
                    ui.label(format!("{} request(s) found — pick one to import:", self.har_candidates.len()));
                    let mut pick: Option<usize> = None;
                    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                        for (i, (label, _)) in self.har_candidates.iter().enumerate() {
                            if ui.selectable_label(false, label).clicked() {
                                pick = Some(i);
                            }
                        }
                    });
                    if let Some(i) = pick {
                        let (_, parsed) = self.har_candidates.remove(i);
                        self.apply_parsed_request(parsed);
                    }
                }
            });
        if !still_open {
            self.show_import = false;
            self.import_curl_text.clear();
            self.import_error = None;
            self.har_candidates.clear();
        }
    }

    /// The method dropdown and URL field rendered as one joined control
    /// (shared border/background, zero gap) instead of two separate boxes.
    fn render_command_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, ctrl_enter: bool) {
        ui.horizontal(|ui| {
            let button_w = if self.is_loading { 150.0 } else { 150.0 };
            let bar_width = ui.available_width() - button_w;

            egui::Frame::none()
                .fill(theme::INPUT_FILL)
                .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
                .rounding(egui::Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(2.0, 2.0))
                .show(ui, |ui| {
                    ui.set_width(bar_width);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.horizontal(|ui| {
                        ui.scope(|ui| {
                            let widgets = &mut ui.visuals_mut().widgets;
                            widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                            widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            widgets.hovered.bg_fill = theme::CARD_FILL;
                            widgets.hovered.rounding = egui::Rounding::same(4.0);
                            egui::ComboBox::from_id_salt("method")
                                .selected_text(egui::RichText::new(&self.state.method).strong())
                                .width(72.0)
                                .show_ui(ui, |ui| {
                                    for m in ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"] {
                                        ui.selectable_value(&mut self.state.method, m.to_string(), m);
                                    }
                                });
                        });
                        ui.add(egui::Separator::default().vertical().spacing(2.0));
                        ui.scope(|ui| {
                            let widgets = &mut ui.visuals_mut().widgets;
                            widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            ui.add(
                                egui::TextEdit::singleline(&mut self.state.url)
                                    .desired_width(ui.available_width())
                                    .hint_text("https://api.example.com/resource")
                                    .frame(false),
                            );
                        });
                    });
                });

            let send_clicked = ui
                .add_enabled(
                    !self.is_loading,
                    egui::Button::new(if self.is_loading { "Sending…" } else { "Send" })
                        .fill(ACCENT.linear_multiply(0.35)),
                )
                .on_hover_text("Ctrl+Enter")
                .clicked();

            if self.is_loading {
                if ui.button("Cancel").clicked() {
                    // Dropping the receiver means the background thread's eventual
                    // result is silently discarded — this isn't a true network abort
                    // (reqwest::blocking can't be interrupted mid-flight), it just
                    // stops the UI from waiting on it.
                    self.rx = None;
                    self.is_loading = false;
                }
            } else if ui.button("Import").clicked() {
                self.show_import = true;
            }

            if send_clicked || (ctrl_enter && !self.is_loading) {
                self.trigger_send(ctx);
            }
        });
    }

    fn render_headers_tab(&mut self, ui: &mut egui::Ui) {
        accented_card(ui, AMBER, |ui| {
            ui.horizontal(|ui| {
                ui.label("Authorization: Bearer");
                ui.add(
                    egui::TextEdit::singleline(&mut self.bearer_token)
                        .desired_width(ui.available_width())
                        .hint_text("token — not saved between runs")
                        .password(true),
                );
            });
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.headers_as_text, "Edit as raw text");
            ui.label(egui::RichText::new("The Bearer token above is added automatically — no need to repeat it here.").weak().small());
        });
        ui.add_space(6.0);

        if self.headers_as_text {
            ui.add(
                egui::TextEdit::multiline(&mut self.state.headers_text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("Content-Type: application/json"),
            );
            return;
        }

        if self.header_rows.is_empty() {
            self.header_rows.push((String::new(), String::new()));
        }
        let mut changed = false;
        let mut remove_idx: Option<usize> = None;

        for i in 0..self.header_rows.len() {
            let (key_resp, val_resp) = ui
                .horizontal(|ui| {
                    let key_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.header_rows[i].0)
                            .desired_width(200.0)
                            .hint_text("Header name"),
                    );
                    let val_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.header_rows[i].1)
                            .desired_width(ui.available_width() - 34.0)
                            .hint_text("Value"),
                    );
                    if ui.small_button("x").on_hover_text("Remove").clicked() {
                        remove_idx = Some(i);
                    }
                    (key_resp, val_resp)
                })
                .inner;

            if key_resp.changed() || val_resp.changed() {
                changed = true;
            }
            if key_resp.has_focus() {
                if suggestion_chips(ui, &mut self.header_rows[i].0, COMMON_HEADERS) {
                    changed = true;
                }
            } else if val_resp.has_focus() && self.header_rows[i].0.eq_ignore_ascii_case("content-type") {
                if suggestion_chips(ui, &mut self.header_rows[i].1, COMMON_CONTENT_TYPES) {
                    changed = true;
                }
            }
        }

        if let Some(idx) = remove_idx {
            self.header_rows.remove(idx);
            changed = true;
        }
        let needs_blank_row = self
            .header_rows
            .last()
            .is_none_or(|(k, v)| !k.is_empty() || !v.is_empty());
        if needs_blank_row {
            self.header_rows.push((String::new(), String::new()));
        }

        if changed {
            self.state.headers_text = self
                .header_rows
                .iter()
                .filter(|(k, v)| !k.trim().is_empty() || !v.trim().is_empty())
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n");
        }
    }

    fn render_body_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.state.body_mode, BodyMode::None, "None");
            ui.selectable_value(&mut self.state.body_mode, BodyMode::Json, "JSON");
            ui.selectable_value(&mut self.state.body_mode, BodyMode::UrlEncoded, "x-www-form-urlencoded");
            ui.selectable_value(&mut self.state.body_mode, BodyMode::Raw, "Raw");
        });
        ui.add_space(4.0);
        match self.state.body_mode {
            BodyMode::None => {
                ui.label(egui::RichText::new("This request has no body.").weak());
            }
            BodyMode::Json => {
                let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| -> std::sync::Arc<egui::Galley> {
                    let mut job = highlight_json(text);
                    job.wrap.max_width = wrap_width;
                    ui.fonts(|f| f.layout_job(job))
                };
                ui.add(
                    egui::TextEdit::multiline(&mut self.state.json_body)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY)
                        .layouter(&mut layouter),
                );
                let trimmed = self.state.json_body.trim();
                if trimmed.is_empty() {
                    // nothing to validate
                } else if let Err(e) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    ui.colored_label(egui::Color32::from_rgb(230, 100, 90), format!("Invalid JSON: {e}"));
                } else {
                    ui.colored_label(egui::Color32::from_rgb(90, 200, 140), "Valid JSON");
                }
            }
            BodyMode::UrlEncoded => {
                ui.label(egui::RichText::new("One key=value per line").weak());
                ui.add(
                    egui::TextEdit::multiline(&mut self.state.urlencoded_body)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY),
                );
            }
            BodyMode::Raw => {
                ui.add(
                    egui::TextEdit::multiline(&mut self.state.raw_body)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            }
        }
    }

    fn render_response_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(230, 100, 90), format!("Request failed: {err}"));
        }

        let Some(resp) = &self.response else { return };

        ui.add_space(6.0);
        ui.label(egui::RichText::new("RESPONSE").weak().small());
        ui.add_space(2.0);

        ui.horizontal(|ui| {
            status_badge(ui, resp.status, &resp.status_text);
            ui.label(format!("{} ms", resp.elapsed_ms));
            ui.label(format_bytes(resp.size_bytes));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if copy_icon_button(ui) {
                    ui.output_mut(|o| o.copied_text = resp.body.clone());
                    self.copied_flash = Some(Instant::now());
                }
                if self.copied_flash.is_some_and(|t| t.elapsed().as_secs_f32() < 1.2) {
                    ui.label(egui::RichText::new("Copied!").small().weak());
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                }
            });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.response_tab, ResponseTab::Body, "Body");
            ui.selectable_value(&mut self.response_tab, ResponseTab::Headers, "Headers");
        });
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match self.response_tab {
                ResponseTab::Body => {
                    if let Some(value) = &resp.json_value {
                        JsonTree::new("response-json-tree", value)
                            .default_expand(DefaultExpand::All)
                            .show(ui);
                    } else {
                        let mut body_copy = resp.body.clone();
                        ui.add(
                            egui::TextEdit::multiline(&mut body_copy)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    }
                }
                ResponseTab::Headers => {
                    for (k, v) in &resp.headers {
                        ui.monospace(format!("{k}: {v}"));
                    }
                }
            });
    }
}

impl eframe::App for ApiTesterApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.state);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // egui_winit reacts to OS ThemeChanged events by resetting visuals to
        // the system theme, which silently undid the one-time dark setup on
        // some Windows configurations. Re-asserting every frame is cheap and
        // makes dark mode immune to that.
        if !ctx.style().visuals.dark_mode {
            theme::apply_theme(ctx);
        }

        if let Some(rx) = &self.rx {
            if let Ok(result) = rx.try_recv() {
                self.is_loading = false;
                self.rx = None;
                let sent_state = self.sent_state.take();

                match &result {
                    Ok(data) => {
                        self.response_tab = ResponseTab::Body;
                        if let (Some(h), Some(s)) = (&self.history, &sent_state) {
                            let _ = h.insert(s, Some(data.status), Some(data.elapsed_ms));
                        }
                    }
                    Err(_) => {
                        if let (Some(h), Some(s)) = (&self.history, &sent_state) {
                            let _ = h.insert(s, None, None);
                        }
                    }
                }
                self.refresh_history();

                match result {
                    Ok(data) => {
                        self.response = Some(data);
                        self.error = None;
                    }
                    Err(err) => {
                        self.error = Some(err);
                        self.response = None;
                    }
                }
            } else if self.is_loading {
                ctx.request_repaint();
            }
        }

        let ctrl_enter =
            ctx.input(|i| i.key_pressed(egui::Key::Enter) && (i.modifiers.ctrl || i.modifiers.command));

        self.render_history_panel(ctx);
        self.render_import_window(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(4.0);
            self.render_command_bar(ui, ctx, ctrl_enter);

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.request_tab, RequestTab::Headers, "Headers");
                ui.selectable_value(&mut self.request_tab, RequestTab::Body, "Body");
            });
            ui.add_space(4.0);
            card(ui, |ui| match self.request_tab {
                RequestTab::Headers => self.render_headers_tab(ui),
                RequestTab::Body => self.render_body_tab(ui),
            });

            ui.add_space(10.0);
            ui.separator();
            self.render_response_section(ui, ctx);
        });
    }
}
