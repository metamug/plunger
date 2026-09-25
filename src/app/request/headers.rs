use super::rows::{edit_rows, remove_button};
use crate::app::ApiTesterApp;
use crate::request::parse_headers;
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

/// Renders a row of small clickable suggestion buttons filtered by prefix
/// match against `target`'s current text; picking one overwrites `target`.
fn suggestion_chips(ui: &mut egui::Ui, target: &mut String, candidates: &[&str]) {
    let typed_lower = target.to_lowercase();
    let matches: Vec<&str> = candidates
        .iter()
        .filter(|c| c.to_lowercase().starts_with(&typed_lower) && c.to_lowercase() != typed_lower)
        .take(6)
        .copied()
        .collect();
    if matches.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("suggestions:").weak().small());
        for m in matches {
            if ui.small_button(m).clicked() {
                *target = m.to_string();
            }
        }
    });
}

fn rows_to_text(rows: &[(String, String)]) -> String {
    rows.iter()
        .filter(|(k, v)| !k.trim().is_empty() || !v.trim().is_empty())
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl ApiTesterApp {
    pub(in crate::app) fn render_headers_tab(&mut self, ui: &mut egui::Ui) {
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
            let toggled = ui.checkbox(&mut self.headers_as_text, "Edit as raw text").changed();
            // Back to the table: rebuild the rows from the text, or edits made
            // in raw mode would be lost (and overwritten on the next table edit).
            if toggled && !self.headers_as_text {
                self.header_rows = parse_headers(&self.state.headers_text);
            }
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

        edit_rows(
            ui,
            &mut self.header_rows,
            || (String::new(), String::new()),
            |(k, v)| k.is_empty() && v.is_empty(),
            |ui, (key, value)| {
                let (key_resp, val_resp, remove) = ui
                    .horizontal(|ui| {
                        let key_resp = ui.add(
                            egui::TextEdit::singleline(key)
                                .desired_width(200.0)
                                .hint_text("Header name"),
                        );
                        let val_resp = ui.add(
                            egui::TextEdit::singleline(value)
                                .desired_width(ui.available_width() - 34.0)
                                .hint_text("Value"),
                        );
                        (key_resp, val_resp, remove_button(ui))
                    })
                    .inner;

                if key_resp.has_focus() {
                    suggestion_chips(ui, key, COMMON_HEADERS);
                } else if val_resp.has_focus() && key.eq_ignore_ascii_case("content-type") {
                    suggestion_chips(ui, value, COMMON_CONTENT_TYPES);
                }
                remove
            },
        );

        let text = rows_to_text(&self.header_rows);
        if text != self.state.headers_text {
            self.state.headers_text = text;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rows_to_text;

    #[test]
    fn blank_rows_are_dropped_and_others_joined() {
        let rows = vec![
            ("Accept".to_string(), "*/*".to_string()),
            (String::new(), String::new()),
            ("X-Empty".to_string(), String::new()),
            ("  ".to_string(), "  ".to_string()),
        ];
        assert_eq!(rows_to_text(&rows), "Accept: */*\nX-Empty: ");
    }
}
