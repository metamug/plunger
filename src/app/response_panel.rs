use super::ApiTesterApp;
use crate::http::format_bytes;
use crate::model::{Outcome, ResponseTab};
use crate::theme::{copy_icon_button, status_badge};
use eframe::egui;
use egui_json_tree::{DefaultExpand, JsonTree};
use std::time::{Duration, Instant};

const COPIED_FLASH: Duration = Duration::from_millis(1200);

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
