use super::ApiTesterApp;
use crate::json_view::highlight_json;
use crate::model::BodyMode;
use crate::theme::{accented_card, AMBER};
use eframe::egui;

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

const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 100, 90);
const OK_COLOR: egui::Color32 = egui::Color32::from_rgb(90, 200, 140);

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

impl ApiTesterApp {
    pub(super) fn render_headers_tab(&mut self, ui: &mut egui::Ui) {
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
            ui.label(
                egui::RichText::new("The Bearer token above is added automatically — no need to repeat it here.")
                    .weak()
                    .small(),
            );
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
            } else if val_resp.has_focus()
                && self.header_rows[i].0.eq_ignore_ascii_case("content-type")
                && suggestion_chips(ui, &mut self.header_rows[i].1, COMMON_CONTENT_TYPES)
            {
                changed = true;
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

    pub(super) fn render_options_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Timeout");
            ui.add(egui::DragValue::new(&mut self.state.timeout_secs).range(1..=600).suffix(" s"));
        });
        ui.add_space(4.0);
        ui.checkbox(&mut self.state.follow_redirects, "Follow redirects");
        ui.add_space(4.0);
        ui.checkbox(&mut self.state.insecure_tls, "Skip TLS certificate verification");
        if self.state.insecure_tls {
            ui.colored_label(
                AMBER,
                "Certificates are not checked. Use this only for servers you trust, e.g. a local API with a self-signed certificate.",
            );
        }
    }

    pub(super) fn render_body_tab(&mut self, ui: &mut egui::Ui) {
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
                if !trimmed.is_empty() {
                    match serde_json::from_str::<serde_json::Value>(trimmed) {
                        Ok(_) => ui.colored_label(OK_COLOR, "Valid JSON"),
                        Err(e) => ui.colored_label(ERROR_COLOR, format!("Invalid JSON: {e}")),
                    };
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
