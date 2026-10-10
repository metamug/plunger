use super::rows::{edit_rows, enabled_checkbox, remove_button};
use super::suggest::variable_chips;
use crate::app::tab::Tab;
use crate::ui::icons::{self, Icon};
use crate::ui::json_view::highlight_json;
use crate::domain::model::{BodyMode, FieldKind, FormField};
use crate::ui::theme::{self, palette};
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

/// What a JSON body's check says, for the little status chip in the tab strip.
#[derive(Debug, PartialEq)]
enum JsonCheck {
    Empty,
    /// Too long to parse on every frame, so left unchecked.
    Large,
    Valid,
    Invalid {
        /// "line 1, column 258" style position and the parser's message.
        message: String,
        position: (usize, usize),
        /// The body has an unquoted `{{variable}}`, which is filled in before sending.
        has_variable: bool,
    },
}

fn check_json(text: &str) -> JsonCheck {
    if text.len() > crate::ui::highlight::LARGE_TEXT_BYTES {
        return JsonCheck::Large;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return JsonCheck::Empty;
    }
    match serde_json::from_str::<serde::de::IgnoredAny>(trimmed) {
        Ok(_) => JsonCheck::Valid,
        Err(e) => JsonCheck::Invalid { message: e.to_string(), position: (e.line(), e.column()), has_variable: trimmed.contains("{{") },
    }
}

fn mode_label(mode: BodyMode) -> &'static str {
    match mode {
        BodyMode::None => "No body",
        BodyMode::Json => "JSON",
        BodyMode::Multipart => "form-data",
        BodyMode::UrlEncoded => "urlencoded",
        BodyMode::Raw => "Raw",
    }
}

impl Tab {
    /// The body's controls, drawn at the right end of the request tab strip (right to left): Prettify,
    /// whether the JSON is valid, and the type of body. They live there, not in rows of their own, so the
    /// editor gets the room.
    pub(in crate::app) fn body_controls(&mut self, ui: &mut egui::Ui) {
        if self.state.body_mode == BodyMode::Json {
            let check = check_json(&self.state.json_body);
            // Always there, so it can be found; greyed out while the JSON does not parse.
            let can_prettify = matches!(check, JsonCheck::Valid);
            let tip = if can_prettify { "Prettify: re-indent the JSON" } else { "Prettify needs valid JSON: fix the error first" };
            let clicked = ui.add_enabled_ui(can_prettify, |ui| icons::button(ui, Icon::Format, tip).clicked()).inner;
            if clicked {
                // Formatting only happens on click, not on every frame.
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&self.state.json_body) {
                    if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                        self.state.json_body = pretty;
                    }
                }
            }
            match check {
                JsonCheck::Empty => {}
                JsonCheck::Large => {
                    ui.label(egui::RichText::new("Large").weak().small()).on_hover_text(format!(
                        "Large body ({}): colouring and validation are off.",
                        crate::app::response_panel::format_bytes(self.state.json_body.len())
                    ));
                }
                JsonCheck::Valid => {
                    ui.label(egui::RichText::new("Valid").color(palette().ok).small()).on_hover_text("The body is valid JSON");
                }
                JsonCheck::Invalid { message, position, has_variable } => {
                    let hint = if has_variable { "\nAn unquoted {{variable}} is not valid JSON as typed, but is filled in when sent." } else { "" };
                    ui.label(egui::RichText::new(format!("Invalid {}:{}", position.0, position.1)).color(palette().error).small())
                        .on_hover_text(format!("Invalid JSON: {message}{hint}"));
                }
            }
        }
        let before = self.state.body_mode;
        egui::ComboBox::from_id_salt(("body-mode", self.id))
            .selected_text(mode_label(before))
            .width(104.0)
            .show_ui(ui, |ui| {
                for (mode, label) in [
                    (BodyMode::None, "No body"),
                    (BodyMode::Json, "JSON"),
                    (BodyMode::Multipart, "form-data"),
                    (BodyMode::UrlEncoded, "x-www-form-urlencoded"),
                    (BodyMode::Raw, "Raw"),
                ] {
                    ui.selectable_value(&mut self.state.body_mode, mode, label);
                }
            })
            .response
            .on_hover_text("The type of the request body");
    }

    pub(in crate::app) fn render_body_tab(&mut self, ui: &mut egui::Ui) {
        match self.state.body_mode {
            BodyMode::None => {
                ui.label(egui::RichText::new("This request has no body.").weak());
            }
            BodyMode::Json => self.render_json_editor(ui),
            BodyMode::Multipart => self.render_multipart_editor(ui),
            BodyMode::UrlEncoded => {
                ui.label(egui::RichText::new("One key=value per line").weak());
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                    let text = text.as_str();
                    let mut job = crate::ui::highlight::form_body(text);
                    job.wrap.max_width = wrap_width;
                    ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
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
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                    let text = text.as_str();
                    let mut job = crate::ui::highlight::body(text);
                    job.wrap.max_width = wrap_width;
                    ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
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
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| -> std::sync::Arc<egui::Galley> {
            let text = text.as_str();
            let mut job = if text.len() > crate::ui::highlight::LARGE_TEXT_BYTES {
                crate::ui::highlight::plain(text, palette().json[5])
            } else {
                highlight_json(text)
            };
            job.wrap.max_width = wrap_width;
            ui.ctx().fonts_mut(|f| f.layout_job(job))
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
                                    .layouter(&mut crate::ui::highlight::variable_layouter(variables)),
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
    use super::{check_json, editor_lines, JsonCheck};

    #[test]
    fn the_json_chip_tells_valid_invalid_empty_and_large_apart() {
        assert_eq!(check_json("  "), JsonCheck::Empty);
        assert_eq!(check_json("{\"a\": [1, 2]}"), JsonCheck::Valid);
        assert_eq!(check_json(&"1".repeat(crate::ui::highlight::LARGE_TEXT_BYTES + 1)), JsonCheck::Large);
        match check_json("{\"a\": 1,
  \"b\": }") {
            JsonCheck::Invalid { position, has_variable, .. } => {
                assert_eq!(position.0, 2, "the line the parser stopped on");
                assert!(!has_variable);
            }
            other => panic!("expected invalid, got {other:?}"),
        }
        match check_json("{\"n\": {{count}}}") {
            JsonCheck::Invalid { has_variable, .. } => assert!(has_variable, "an unquoted variable is explained, not just an error"),
            other => panic!("expected invalid, got {other:?}"),
        }
    }

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
