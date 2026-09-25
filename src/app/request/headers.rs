use super::rows::{edit_rows, remove_button};
use super::suggest::{chips, variable_chips};
use crate::app::tab::Tab;
use crate::icons::{self, Icon};
use crate::request::parse_headers;
use crate::theme::{self, accented_card, palette};
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

fn rows_to_text(rows: &[(String, String)]) -> String {
    rows.iter()
        .filter(|(k, v)| !k.trim().is_empty() || !v.trim().is_empty())
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Tab {
    pub(in crate::app) fn render_headers_tab(&mut self, ui: &mut egui::Ui, bearer: &mut String, secrets_error: Option<&str>) {
        accented_card(ui, palette().amber, |ui| {
            ui.horizontal(|ui| {
                ui.label("Authorization: Bearer");
                let hint = if self.state.remember_bearer {
                    "token — kept in the system credential store"
                } else {
                    "token — not saved between runs"
                };
                let width = ui.available_width() - icons::trailing_room(ui, 1);
                ui.add(theme::field(bearer).desired_width(width).hint_text(hint).password(true));
                icons::toggle(
                    ui,
                    &mut self.state.remember_bearer,
                    Icon::Key,
                    "Remembered in the system credential store (never in a file). Click to stop remembering",
                    "Not remembered: cleared when the app closes. Click to keep it in the system credential store",
                );
            });
            if let Some(err) = secrets_error {
                ui.colored_label(palette().error, err);
            }
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("The Bearer token above is added automatically — no need to repeat it here.")
                    .weak()
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let toggled = icons::toggle(
                    ui,
                    &mut self.headers_as_text,
                    Icon::Code,
                    "Editing as raw text. Click to go back to the table",
                    "Edit as raw text (one \"Name: value\" per line)",
                )
                .changed();
                // Back to the table: rebuild the rows from the text, or edits made
                // in raw mode would be lost (and overwritten on the next table edit).
                if toggled && !self.headers_as_text {
                    self.header_rows = parse_headers(&self.state.headers_text);
                }
            });
        });
        ui.add_space(6.0);

        if self.headers_as_text {
            ui.add(
                theme::area(&mut self.state.headers_text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("Content-Type: application/json"),
            );
            return;
        }

        let variables = &self.state.variables;
        edit_rows(
            ui,
            &mut self.header_rows,
            || (String::new(), String::new()),
            |(k, v)| k.is_empty() && v.is_empty(),
            |ui, (key, value), spare| {
                let (key_resp, val_resp, remove) = ui
                    .horizontal(|ui| {
                        let key_resp = ui.add(
                            theme::field(key)
                                .desired_width(200.0)
                                .hint_text("Header name"),
                        );
                        let width = ui.available_width() - icons::trailing_room(ui, 1);
                        let val_resp = ui.add(theme::field(value).desired_width(width).hint_text("Value"));
                        (key_resp, val_resp, remove_button(ui, "header", spare))
                    })
                    .inner;

                chips(ui, &key_resp, key, COMMON_HEADERS);
                if key.eq_ignore_ascii_case("content-type") {
                    chips(ui, &val_resp, value, COMMON_CONTENT_TYPES);
                } else {
                    variable_chips(ui, &val_resp, value, variables);
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
