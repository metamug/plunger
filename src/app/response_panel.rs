use super::copy_button;
use super::tab::Tab;
use crate::model::{Outcome, ResponseTab};
use crate::icons::{self, Icon};
use crate::theme::{self, palette, status_badge};
use eframe::egui;
use egui_json_tree::{DefaultExpand, JsonTree};
const LARGE_JSON_BYTES: usize = 200 * 1024;

impl Tab {
    pub(super) fn render_response_section(&mut self, ui: &mut egui::Ui) {
        let resp = match &self.outcome {
            Outcome::Empty => {
                if !self.is_loading() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("Send a request to see the response here").weak());
                        ui.label(egui::RichText::new("Ctrl+Enter sends from anywhere").weak().small());
                    });
                }
                return;
            }
            Outcome::Failed(err) => {
                ui.colored_label(palette().error, format!("Request failed: {err}"));
                return;
            }
            Outcome::Response(resp) => resp,
        };

        ui.add_space(6.0);
        ui.label(egui::RichText::new("RESPONSE").weak().small());
        ui.add_space(2.0);

        ui.horizontal(|ui| {
            status_badge(ui, resp.status, &resp.status_text);
            ui.label(egui::RichText::new(format!("{} ms", resp.elapsed_ms)).weak());
            ui.label(egui::RichText::new(format_bytes(resp.size_bytes)).weak());

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Download, "Save response body to a file").clicked() {
                    let name = if resp.json_value.is_some() { "response.json" } else { "response.txt" };
                    if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
                        self.save_error = std::fs::write(&path, &resp.body)
                            .err()
                            .map(|e| format!("Could not save file: {e}"));
                    }
                }
                // Copies whatever tab is showing.
                match self.response_tab {
                    ResponseTab::Body => {
                        copy_button(ui, &mut self.copied_flash, "body", "Copy response body", &resp.body);
                    }
                    ResponseTab::Headers => {
                        let headers: String = resp.headers.iter().map(|(k, v)| format!("{k}: {v}
")).collect();
                        copy_button(ui, &mut self.copied_flash, "headers", "Copy response headers", &headers);
                    }
                }
            });
        });

        if resp.truncated {
            let total = resp.total_size.map(|t| format!(" of {}", format_bytes(t as usize))).unwrap_or_default();
            ui.colored_label(
                palette().amber,
                format!("Response too large: showing the first {}{total}.", format_bytes(resp.size_bytes)),
            );
        }
        if let Some(err) = &self.save_error {
            ui.colored_label(palette().error, err);
        }

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
                        // Expanding every node of a big document stalls the UI.
                        let expand = if resp.body.len() > LARGE_JSON_BYTES {
                            DefaultExpand::ToLevel(1)
                        } else {
                            DefaultExpand::All
                        };
                        JsonTree::new("response-json-tree", value).default_expand(expand).show(ui);
                    } else {
                        // `&str` is a read-only text buffer: selectable and copyable,
                        // but no per-frame clone of the body and no accidental edits.
                        let mut text: &str = &resp.body;
                        ui.add(
                            theme::area(&mut text)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    }
                }
                ResponseTab::Headers => {
                    egui::Grid::new("response-headers").num_columns(2).spacing([16.0, 6.0]).striped(true).show(
                        ui,
                        |ui| {
                            for (k, v) in &resp.headers {
                                ui.label(egui::RichText::new(k).monospace().weak());
                                ui.add(egui::Label::new(egui::RichText::new(v).monospace()).wrap());
                                ui.end_row();
                            }
                        },
                    );
                }
            });
    }
}

pub(super) fn format_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn format_bytes_picks_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
    }
}
