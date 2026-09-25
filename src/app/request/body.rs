use super::rows::{edit_rows, enabled_checkbox, remove_button};
use super::suggest::variable_chips;
use crate::app::tab::Tab;
use crate::icons::{self, Icon};
use crate::json_view::highlight_json;
use crate::model::{BodyMode, FieldKind, FormField};
use crate::theme::{self, palette};
use eframe::egui;

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
                ui.add(
                    theme::area(&mut self.state.urlencoded_body)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY),
                );
            }
            BodyMode::Raw => {
                ui.add(
                    theme::area(&mut self.state.raw_body)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            }
        }
    }

    fn render_json_editor(&mut self, ui: &mut egui::Ui) {
        let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| -> std::sync::Arc<egui::Galley> {
            let mut job = highlight_json(text);
            job.wrap.max_width = wrap_width;
            ui.fonts(|f| f.layout_job(job))
        };
        ui.add(
            theme::area(&mut self.state.json_body)
                .desired_rows(5)
                .desired_width(f32::INFINITY)
                .layouter(&mut layouter),
        );

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
                            value_resp = Some(
                                ui.add(theme::field(&mut f.value).desired_width(width).hint_text("value  (or {{variable}})")),
                            );
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
