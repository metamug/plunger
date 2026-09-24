#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use egui_json_tree::{DefaultExpand, JsonTree};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 680.0])
            .with_min_inner_size([560.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Metamug API Tester",
        options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);

            let state: PersistedState = cc
                .storage
                .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
                .unwrap_or_default();
            Ok(Box::new(ApiTesterApp::from_persisted(state)))
        }),
    )
}

/// The accent used for the active tab, focused-field borders, and the Send
/// button — one color, used consistently, rather than egui's default blue.
const ACCENT: egui::Color32 = egui::Color32::from_rgb(90, 125, 230);
const CARD_FILL: egui::Color32 = egui::Color32::from_rgb(32, 35, 42);
const INPUT_FILL: egui::Color32 = egui::Color32::from_rgb(21, 23, 28);
const BORDER: egui::Color32 = egui::Color32::from_rgb(60, 65, 76);
const AMBER: egui::Color32 = egui::Color32::from_rgb(210, 160, 60);

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    visuals.override_text_color = Some(egui::Color32::from_gray(225));
    visuals.window_fill = egui::Color32::from_rgb(24, 26, 31);
    visuals.panel_fill = egui::Color32::from_rgb(24, 26, 31);
    visuals.extreme_bg_color = INPUT_FILL; // TextEdit / ScrollArea background
    visuals.faint_bg_color = CARD_FILL;

    visuals.selection.bg_fill = ACCENT.linear_multiply(0.55);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT);

    for widget_visuals in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.noninteractive,
    ] {
        widget_visuals.bg_fill = INPUT_FILL;
        widget_visuals.weak_bg_fill = INPUT_FILL;
        widget_visuals.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
        widget_visuals.rounding = egui::Rounding::same(5.0);
    }
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.3_f32, ACCENT.linear_multiply(0.85));
    visuals.widgets.hovered.rounding = egui::Rounding::same(5.0);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5_f32, ACCENT);
    visuals.widgets.active.rounding = egui::Rounding::same(5.0);
    visuals.widgets.active.bg_fill = ACCENT.linear_multiply(0.25);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.window_margin = egui::Margin::same(14.0);
    for (text_style, font_id) in style.text_styles.iter_mut() {
        match text_style {
            egui::TextStyle::Body | egui::TextStyle::Button => font_id.size = 14.5,
            egui::TextStyle::Monospace => font_id.size = 13.5,
            egui::TextStyle::Small => font_id.size = 12.0,
            _ => {}
        }
    }
    ctx.set_style(style);
}

/// A visually distinct "card" — used to separate the command bar, the auth
/// row, and the headers/body panel from the flat window background instead
/// of leaving everything the same shade of dark gray.
fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(CARD_FILL)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add_contents(ui);
        });
}

/// Status code as a colored badge (green/blue/amber/red by class) instead of
/// plain colored text — reads at a glance rather than needing to parse a number.
fn status_badge(ui: &mut egui::Ui, status: u16, status_text: &str) {
    let (bg, fg) = match status {
        200..=299 => (
            egui::Color32::from_rgb(20, 60, 40),
            egui::Color32::from_rgb(140, 230, 180),
        ),
        300..=399 => (
            egui::Color32::from_rgb(25, 40, 80),
            egui::Color32::from_rgb(150, 180, 240),
        ),
        400..=499 => (
            egui::Color32::from_rgb(75, 50, 10),
            egui::Color32::from_rgb(240, 190, 110),
        ),
        _ => (
            egui::Color32::from_rgb(75, 20, 20),
            egui::Color32::from_rgb(240, 140, 130),
        ),
    };
    egui::Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(8.0, 3.0))
        .show(ui, |ui| {
            ui.colored_label(fg, format!("{status} {status_text}"));
        });
}

/// A small hand-painted copy icon (two overlapping page outlines) rather than
/// a text label or a font glyph — avoids repeating the missing-glyph issue
/// from the header remove button, since this doesn't depend on font coverage
/// at all.
fn copy_icon_button(ui: &mut egui::Ui) -> bool {
    let size = egui::vec2(24.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let color = visuals.fg_stroke.color;
        let painter = ui.painter();
        let back = egui::Rect::from_min_size(rect.min + egui::vec2(3.0, 4.0), egui::vec2(12.0, 14.0));
        painter.rect_stroke(back, egui::Rounding::same(2.0), egui::Stroke::new(1.3_f32, color));
        let front = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 1.0), egui::vec2(12.0, 14.0));
        painter.rect_filled(front, egui::Rounding::same(2.0), CARD_FILL);
        painter.rect_stroke(front, egui::Rounding::same(2.0), egui::Stroke::new(1.3_f32, color));
    }
    response.on_hover_text("Copy response body").clicked()
}

#[derive(PartialEq, Clone, Copy, Serialize, Deserialize)]
enum BodyMode {
    None,
    Json,
    UrlEncoded,
    Raw,
}

impl Default for BodyMode {
    fn default() -> Self {
        BodyMode::None
    }
}

#[derive(PartialEq, Clone, Copy)]
enum RequestTab {
    Headers,
    Body,
}

#[derive(PartialEq, Clone, Copy)]
enum HeaderInputMode {
    Table,
    Text,
}

#[derive(PartialEq, Clone, Copy)]
enum ResponseTab {
    Body,
    Headers,
}

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

/// The subset of app state worth remembering between runs — request config,
/// not transient things like "is a request in flight right now."
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct PersistedState {
    method: String,
    url: String,
    headers_text: String,
    body_mode: BodyMode,
    json_body: String,
    urlencoded_body: String,
    raw_body: String,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            method: "GET".to_string(),
            url: "https://jsonplaceholder.typicode.com/todos/1".to_string(),
            headers_text: String::new(),
            body_mode: BodyMode::None,
            json_body: String::from("{\n  \"key\": \"value\"\n}"),
            urlencoded_body: String::new(),
            raw_body: String::new(),
        }
    }
}

struct ResponseData {
    status: u16,
    status_text: String,
    elapsed_ms: u128,
    size_bytes: usize,
    headers: Vec<(String, String)>,
    body: String,
    json_value: Option<serde_json::Value>,
}

struct ApiTesterApp {
    state: PersistedState,

    // Deliberately NOT part of PersistedState / not written to disk — it's a
    // credential, and silently persisting someone's auth token in a plaintext
    // local config file is the kind of thing that should be opt-in, not a
    // surprise. Cleared every time the app starts.
    bearer_token: String,

    request_tab: RequestTab,
    header_input_mode: HeaderInputMode,
    header_rows: Vec<(String, String)>,
    response_tab: ResponseTab,
    is_loading: bool,
    response: Option<ResponseData>,
    error: Option<String>,
    rx: Option<Receiver<Result<ResponseData, String>>>,
    copied_flash: Option<Instant>,
}

impl ApiTesterApp {
    fn from_persisted(state: PersistedState) -> Self {
        let header_rows = parse_headers(&state.headers_text);
        Self {
            state,
            bearer_token: String::new(),
            request_tab: RequestTab::Headers,
            header_input_mode: HeaderInputMode::Table,
            header_rows,
            response_tab: ResponseTab::Body,
            is_loading: false,
            response: None,
            error: None,
            rx: None,
            copied_flash: None,
        }
    }
}

fn parse_headers(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.trim().to_string()))
        })
        .collect()
}

fn format_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

fn pretty_json_if_possible(text: &str) -> (String, Option<serde_json::Value>) {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => {
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.to_string());
            (pretty, Some(value))
        }
        Err(_) => (text.to_string(), None),
    }
}

/// A small hand-written JSON highlighter (not a general syntax-highlighting
/// engine like syntect) — keeps the binary lightweight, and JSON's grammar
/// is simple enough that a general engine would be overkill.
fn highlight_json(text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let font_id = egui::FontId::monospace(13.0);

    let color_punct = egui::Color32::from_rgb(150, 150, 150);
    let color_key = egui::Color32::from_rgb(220, 120, 160);
    let color_string = egui::Color32::from_rgb(120, 200, 140);
    let color_number = egui::Color32::from_rgb(110, 170, 230);
    let color_literal = egui::Color32::from_rgb(220, 160, 90);
    let color_default = egui::Color32::from_rgb(210, 210, 210);

    let append = |job: &mut egui::text::LayoutJob, s: &str, color: egui::Color32| {
        job.append(
            s,
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color,
                ..Default::default()
            },
        );
    };

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    let mut idx = 0;

    while idx < n {
        let (byte_pos, ch) = chars[idx];

        if ch.is_whitespace() {
            let start = byte_pos;
            let mut end = byte_pos + ch.len_utf8();
            idx += 1;
            while idx < n && chars[idx].1.is_whitespace() {
                end = chars[idx].0 + chars[idx].1.len_utf8();
                idx += 1;
            }
            append(&mut job, &text[start..end], color_default);
            continue;
        }

        if ch == '"' {
            let start = byte_pos;
            let mut end = byte_pos + 1;
            idx += 1;
            let mut escaped = false;
            while idx < n {
                let (bp, c) = chars[idx];
                end = bp + c.len_utf8();
                idx += 1;
                if escaped {
                    escaped = false;
                    continue;
                }
                if c == '\\' {
                    escaped = true;
                    continue;
                }
                if c == '"' {
                    break;
                }
            }
            let mut lookahead = idx;
            while lookahead < n && chars[lookahead].1.is_whitespace() {
                lookahead += 1;
            }
            let is_key = lookahead < n && chars[lookahead].1 == ':';
            append(
                &mut job,
                &text[start..end],
                if is_key { color_key } else { color_string },
            );
            continue;
        }

        if matches!(ch, '{' | '}' | '[' | ']' | ',' | ':') {
            let start = byte_pos;
            let end = byte_pos + ch.len_utf8();
            idx += 1;
            append(&mut job, &text[start..end], color_punct);
            continue;
        }

        if ch == '-' || ch.is_ascii_digit() {
            let start = byte_pos;
            let mut end = byte_pos + ch.len_utf8();
            idx += 1;
            while idx < n {
                let (bp, c) = chars[idx];
                if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
                    end = bp + c.len_utf8();
                    idx += 1;
                } else {
                    break;
                }
            }
            append(&mut job, &text[start..end], color_number);
            continue;
        }

        if ch.is_alphabetic() {
            let start = byte_pos;
            let mut end = byte_pos + ch.len_utf8();
            idx += 1;
            while idx < n && chars[idx].1.is_alphanumeric() {
                end = chars[idx].0 + chars[idx].1.len_utf8();
                idx += 1;
            }
            let word = &text[start..end];
            let color = if word == "true" || word == "false" || word == "null" {
                color_literal
            } else {
                color_default
            };
            append(&mut job, word, color);
            continue;
        }

        let start = byte_pos;
        let end = byte_pos + ch.len_utf8();
        idx += 1;
        append(&mut job, &text[start..end], color_default);
    }

    job
}

fn send_request(
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    tx: Sender<Result<ResponseData, String>>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<ResponseData, String> {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .map_err(|e| e.to_string())?;

            let method = reqwest::Method::from_bytes(method.as_bytes())
                .map_err(|_| "Invalid HTTP method".to_string())?;

            let mut builder = client.request(method, &url);
            for (k, v) in &headers {
                builder = builder.header(k, v);
            }
            if let Some(b) = body {
                builder = builder.body(b);
            }

            let start = Instant::now();
            let res = builder.send().map_err(|e| e.to_string())?;
            let elapsed_ms = start.elapsed().as_millis();

            let status = res.status().as_u16();
            let status_text = res.status().canonical_reason().unwrap_or("").to_string();
            let resp_headers: Vec<(String, String)> = res
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
                .collect();
            let content_type = res
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let text = res.text().map_err(|e| e.to_string())?;
            let size_bytes = text.len();

            let looks_json = content_type.contains("json")
                || text.trim_start().starts_with('{')
                || text.trim_start().starts_with('[');
            let (display_body, json_value) = if looks_json {
                pretty_json_if_possible(&text)
            } else {
                (text, None)
            };

            Ok(ResponseData {
                status,
                status_text,
                elapsed_ms,
                size_bytes,
                headers: resp_headers,
                body: display_body,
                json_value,
            })
        })();

        let _ = tx.send(result);
    });
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
            apply_theme(ctx);
        }

        if let Some(rx) = &self.rx {
            if let Ok(result) = rx.try_recv() {
                self.is_loading = false;
                self.rx = None;
                match result {
                    Ok(data) => {
                        self.response = Some(data);
                        self.error = None;
                        self.response_tab = ResponseTab::Body;
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

        let ctrl_enter = ctx.input(|i| {
            i.key_pressed(egui::Key::Enter) && (i.modifiers.ctrl || i.modifiers.command)
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(4.0);

            // Command bar: method + URL + send/cancel, all on one row — this
            // is the thing you use most, so it shouldn't be split across rows.
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("method")
                        .selected_text(self.state.method.clone())
                        .width(85.0)
                        .show_ui(ui, |ui| {
                            for m in ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"] {
                                ui.selectable_value(&mut self.state.method, m.to_string(), m);
                            }
                        });

                    let button_w = if self.is_loading { 140.0 } else { 70.0 };
                    ui.add(
                        egui::TextEdit::singleline(&mut self.state.url)
                            .desired_width(ui.available_width() - button_w)
                            .hint_text("https://api.example.com/resource"),
                    );

                    let send_clicked = ui
                        .add_enabled(
                            !self.is_loading,
                            egui::Button::new(if self.is_loading { "Sending…" } else { "Send" })
                                .fill(ACCENT.linear_multiply(0.35)),
                        )
                        .on_hover_text("Ctrl+Enter")
                        .clicked();

                    if self.is_loading && ui.button("Cancel").clicked() {
                        // Dropping the receiver means the background thread's eventual
                        // result is silently discarded — this isn't a true network abort
                        // (reqwest::blocking can't be interrupted mid-flight), it just
                        // stops the UI from waiting on it.
                        self.rx = None;
                        self.is_loading = false;
                    }

                    if send_clicked || (ctrl_enter && !self.is_loading) {
                        self.trigger_send(ctx);
                    }
                });

            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.request_tab, RequestTab::Headers, "Headers");
                ui.selectable_value(&mut self.request_tab, RequestTab::Body, "Body");
            });
            ui.add_space(4.0);
            card(ui, |ui| {
                match self.request_tab {
                    RequestTab::Headers => {
                        egui::Frame::none()
                            .stroke(egui::Stroke::new(1.3_f32, AMBER))
                            .rounding(egui::Rounding::same(5.0))
                            .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
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

                        let prev_mode = self.header_input_mode;
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.header_input_mode, HeaderInputMode::Table, "Table");
                            ui.selectable_value(&mut self.header_input_mode, HeaderInputMode::Text, "Text");
                            ui.label(egui::RichText::new("The Bearer token above is added automatically — no need to repeat it here.").weak().small());
                        });
                        if prev_mode != self.header_input_mode && self.header_input_mode == HeaderInputMode::Table {
                            self.header_rows = parse_headers(&self.state.headers_text);
                        }
                        ui.add_space(6.0);

                        match self.header_input_mode {
                            HeaderInputMode::Text => {
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.state.headers_text)
                                        .desired_rows(3)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("Content-Type: application/json"),
                                );
                            }
                            HeaderInputMode::Table => {
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
                                    } else if val_resp.has_focus()
                                        && self.header_rows[i].0.eq_ignore_ascii_case("content-type")
                                    {
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
                        }
                    }
                    RequestTab::Body => {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.state.body_mode, BodyMode::None, "None");
                            ui.selectable_value(&mut self.state.body_mode, BodyMode::Json, "JSON");
                            ui.selectable_value(
                                &mut self.state.body_mode,
                                BodyMode::UrlEncoded,
                                "x-www-form-urlencoded",
                            );
                            ui.selectable_value(&mut self.state.body_mode, BodyMode::Raw, "Raw");
                        });
                        ui.add_space(4.0);
                        match self.state.body_mode {
                            BodyMode::None => {
                                ui.label(egui::RichText::new("This request has no body.").weak());
                            }
                            BodyMode::Json => {
                                let mut layouter =
                                    |ui: &egui::Ui, text: &str, wrap_width: f32| -> std::sync::Arc<egui::Galley> {
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
                                    ui.colored_label(
                                        egui::Color32::from_rgb(230, 100, 90),
                                        format!("Invalid JSON: {e}"),
                                    );
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
                }
            });

            ui.add_space(10.0);
            ui.separator();

            if let Some(err) = &self.error {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 100, 90),
                    format!("Request failed: {err}"),
                );
            }

            if let Some(resp) = &self.response {
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
                        if self
                            .copied_flash
                            .is_some_and(|t| t.elapsed().as_secs_f32() < 1.2)
                        {
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

                // Fills whatever space remains in the window instead of a
                // small fixed-height box — nothing collapsed, nothing hidden
                // behind an extra click.
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
        });
    }
}

impl ApiTesterApp {
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

        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.is_loading = true;
        self.error = None;
        self.response = None;
        send_request(self.state.method.clone(), self.state.url.clone(), headers, body, tx);
        ctx.request_repaint();
    }
}
