use super::rows::{edit_rows, enabled_checkbox, remove_button};
use super::suggest::variable_chips;
use crate::app::tab::Tab;
use crate::icons::{self, Icon};
use crate::json_view::highlight_json;
use crate::model::{BodyMode, FieldKind, FormField};
use crate::theme::{self, palette};
use eframe::egui;

/// How many lines a body editor asks for: its text plus a spare line to click into, never fewer than
/// four and never more than `room` (what the pane can hold). A short body gets a short box, not a block
/// of blank lines the cursor cannot enter; a long one grows as far as the pane allows.
fn editor_lines(text: &str, room: usize, fill: bool) -> usize {
    if fill {
        return room.max(MIN_EDITOR_LINES);
    }
    (text.split('\n').count() + 1).clamp(MIN_EDITOR_LINES, room.max(MIN_EDITOR_LINES))
}

const MIN_EDITOR_LINES: usize = 4;

/// A body editor is not wrapped in a scroll area of its own: the request pane scrolls, so there is one
/// scroll bar, and the editor can use all the room the pane has instead of stopping at a fixed height.
fn body_box(ui: &mut egui::Ui, _salt: &str, add: impl FnOnce(&mut egui::Ui)) {
    add(ui);
}

impl Tab {
    pub(in crate::app) fn render_body_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.state.body_mode, BodyMode::None, "None");
            ui.selectable_value(&mut self.state.body_mode, BodyMode::Json, "JSON");
            ui.selectable_value(&mut self.state.body_mode, BodyMode::Multipart, "form-data");
            ui.selectable_value(&mut self.state.body_mode, BodyMode::UrlEncoded, "x-www-form-urlencoded");
            ui.selectable_value(&mut self.state.body_mode, BodyMode::Raw, "Raw");
        });
        ui.add_space(4.0);
        match self.state.body_mode {
            BodyMode::None => {
                ui.label(egui::RichText::new("This request has no body.").weak());
            }
            BodyMode::Json => self.render_json_editor(ui),
            BodyMode::Multipart => self.render_multipart_editor(ui),
            BodyMode::UrlEncoded => {
                ui.label(egui::RichText::new("One key=value per line").weak());
                let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
                    let mut job = crate::highlight::form_body(text);
                    job.wrap.max_width = wrap_width;
                    ui.fonts(|fonts| fonts.layout_job(job))
                };
                let rows = editor_lines(&self.state.urlencoded_body, self.editor_rows, self.editor_fill);
                body_box(ui, "body-urlencoded", |ui| {
                    ui.add(
                        theme::area(&mut self.state.urlencoded_body)
                            .desired_rows(rows)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .layouter(&mut layouter),
                    );
                });
            }
            BodyMode::Raw => {
                // JSON, XML/HTML and forms are told apart by what the text looks like.
                let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
                    let mut job = crate::highlight::body(text);
                    job.wrap.max_width = wrap_width;
                    ui.fonts(|fonts| fonts.layout_job(job))
                };
                let rows = editor_lines(&self.state.raw_body, self.editor_rows, self.editor_fill);
                body_box(ui, "body-raw", |ui| {
                    ui.add(
                        theme::area(&mut self.state.raw_body)
                            .desired_rows(rows)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .layouter(&mut layouter),
                    );
                });
            }
        }
    }

    fn render_json_editor(&mut self, ui: &mut egui::Ui) {
        let large = self.state.json_body.len() > crate::highlight::LARGE_TEXT_BYTES;
        let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| -> std::sync::Arc<egui::Galley> {
            let mut job = if text.len() > crate::highlight::LARGE_TEXT_BYTES {
                crate::highlight::plain(text, palette().json[5])
            } else {
                highlight_json(text)
            };
            job.wrap.max_width = wrap_width;
            ui.fonts(|f| f.layout_job(job))
        };
        let rows = editor_lines(&self.state.json_body, self.editor_rows, self.editor_fill);
        body_box(ui, "body-json", |ui| {
            ui.add(
                theme::area(&mut self.state.json_body)
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY)
                    .layouter(&mut layouter),
            );
        });

        if large {
            // Parsing a body this size on every frame would keep a core busy.
            ui.label(
                egui::RichText::new(format!(
                    "Large body ({}): colouring and validation are off.",
                    crate::app::response_panel::format_bytes(self.state.json_body.len())
                ))
                .weak(),
            );
            return;
        }
        let trimmed = self.state.json_body.trim();
        if trimmed.is_empty() {
            return;
        }
        let has_variable = trimmed.contains("{{");
        let verdict = serde_json::from_str::<serde::de::IgnoredAny>(trimmed).map_err(|e| e.to_string());
        ui.horizontal(|ui| match verdict {
            Ok(_) => {
                ui.colored_label(palette().ok, "Valid JSON");
                if icons::button(ui, Icon::Format, "Prettify: re-indent the JSON").clicked() {
                    // Formatting only happens on click, not on every frame.
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&self.state.json_body) {
                        if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                            self.state.json_body = pretty;
                        }
                    }
                }
            }
            Err(e) => {
                // An unquoted {{variable}} isn't valid JSON as typed, but is
                // substituted before sending, so don't leave it as a bare error.
                let hint = if has_variable {
                    " (an unquoted {{variable}} is filled in when sent)"
                } else {
                    ""
                };
                ui.colored_label(palette().error, format!("Invalid JSON: {e}{hint}"));
            }
        });
    }

    fn render_multipart_editor(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Sent as multipart/form-data; the Content-Type and boundary are set automatically.")
                .weak()
                .small(),
        );
        ui.add_space(4.0);

        let mut next_id = 0;
        let variables = &self.state.variables;
        edit_rows(
            ui,
            &mut self.state.multipart_fields,
            FormField::blank,
            // A file row with nothing chosen yet still counts as "in use", so a fresh blank row follows it.
            |f| f.is_blank() && f.kind == FieldKind::Text,
            |ui, f, spare| {
                next_id += 1;
                let (value_resp, remove) = ui.horizontal(|ui| {
                    let mut value_resp = None;
                    enabled_checkbox(ui, &mut f.enabled);
                    ui.add(theme::field(&mut f.key).desired_width(140.0).hint_text("field name"));
                    egui::ComboBox::from_id_salt(("field-kind", next_id))
                        .selected_text(if f.kind == FieldKind::Text { "Text" } else { "File" })
                        .width(60.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut f.kind, FieldKind::Text, "Text");
                            ui.selectable_value(&mut f.kind, FieldKind::File, "File");
                        });
                    let width = ui.available_width() - icons::trailing_room(ui, 1);
                    match f.kind {
                        FieldKind::Text => {
                            value_resp = Some(ui.add(
                                theme::field(&mut f.value)
                                    .desired_width(width)
                                    .hint_text("value  (or {{variable}})")
                                    .layouter(&mut crate::highlight::variable_layouter(variables)),
                            ));
                        }
                        FieldKind::File => {
                            // Same width as a text value, so the remove icon lines up across rows.
                            let size = egui::vec2(width + theme::FIELD_MARGIN_X, icons::SIZE);
                            ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.set_min_size(size);
                                if icons::button(ui, Icon::Folder, "Choose a file to upload").clicked() {
                                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                                        f.value = path.to_string_lossy().into_owned();
                                    }
                                }
                                let shown = std::path::Path::new(&f.value)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                if shown.is_empty() {
                                    ui.label(egui::RichText::new("no file chosen").weak());
                                } else {
                                    ui.label(shown).on_hover_text(&f.value);
                                }
                            });
                        }
                    }
                    (value_resp, remove_button(ui, "field", spare))
                })
                .inner;
                if let Some(resp) = value_resp {
                    variable_chips(ui, &resp, &mut f.value, variables);
                }
                remove
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::editor_lines;

    #[test]
    fn a_short_body_gets_a_short_box() {
        assert_eq!(editor_lines("", 30, false), 4, "never fewer than four lines");
        assert_eq!(editor_lines("{\"a\": 1}", 30, false), 4);
        assert_eq!(editor_lines("a\nb\nc\nd\ne", 30, false), 6, "the text and one spare line");
        assert_eq!(editor_lines("a", 30, true), 30, "an expanded editor is the whole surface");
    }

    #[test]
    fn a_long_body_grows_only_as_far_as_the_pane_allows() {
        let long = "x\n".repeat(100);
        assert_eq!(editor_lines(&long, 12, false), 12);
        assert_eq!(editor_lines("a", 2, false), 4, "a tiny pane does not push it below four lines");
    }
}
