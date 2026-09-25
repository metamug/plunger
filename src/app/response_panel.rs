use super::ApiTesterApp;
use crate::model::{Outcome, ResponseTab};
use crate::theme::{copy_icon_button, status_badge, AMBER};
use eframe::egui;
use egui_json_tree::{DefaultExpand, JsonTree};
use std::time::{Duration, Instant};

const COPIED_FLASH: Duration = Duration::from_millis(1200);
const LARGE_JSON_BYTES: usize = 200 * 1024;

impl ApiTesterApp {
    pub(super) fn render_response_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let resp = match &self.outcome {
            Outcome::Empty => return,
            Outcome::Failed(err) => {
                ui.colored_label(egui::Color32::from_rgb(230, 100, 90), format!("Request failed: {err}"));
                return;
            }
            Outcome::Response(resp) => resp,
        };

        ui.add_space(6.0);
        ui.label(egui::RichText::new("RESPONSE").weak().small());
        ui.add_space(2.0);

        ui.horizontal(|ui| {
            status_badge(ui, resp.status, &resp.status_text);
            ui.label(format!("{} ms", resp.elapsed_ms));
            ui.label(format_bytes(resp.size_bytes));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Save…").on_hover_text("Save the body to a file").clicked() {
                    let name = if resp.json_value.is_some() { "response.json" } else { "response.txt" };
                    if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
                        self.save_error = std::fs::write(&path, &resp.body)
                            .err()
                            .map(|e| format!("Could not save file: {e}"));
                    }
                }
                if copy_icon_button(ui) {
                    ui.output_mut(|o| o.copied_text = resp.body.clone());
                    self.copied_flash = Some(Instant::now());
                }
                if self.copied_flash.is_some_and(|t| t.elapsed() < COPIED_FLASH) {
                    ui.label(egui::RichText::new("Copied!").small().weak());
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
            });
        });

        if resp.truncated {
            let total = resp.total_size.map(|t| format!(" of {}", format_bytes(t as usize))).unwrap_or_default();
            ui.colored_label(
                AMBER,
                format!("Response too large: showing the first {}{total}.", format_bytes(resp.size_bytes)),
            );
        }
        if let Some(err) = &self.save_error {
            ui.colored_label(egui::Color32::from_rgb(230, 100, 90), err);
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
                            egui::TextEdit::multiline(&mut text)
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

fn format_bytes(n: usize) -> String {
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
