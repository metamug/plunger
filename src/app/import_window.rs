use super::ApiTesterApp;
use crate::curl_import::{parse_curl, parse_har};
use eframe::egui;

impl ApiTesterApp {
    pub(super) fn render_import_window(&mut self, ctx: &egui::Context) {
        if !self.import.open {
            return;
        }
        let mut still_open = true;
        egui::Window::new("Import request")
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .open(&mut still_open)
            .show(ctx, |ui| {
                ui.label("Paste a curl command:");
                ui.add(
                    egui::TextEdit::multiline(&mut self.import.curl_text)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("curl 'https://api.example.com/resource' -H 'Authorization: Bearer ...' -d '{...}'"),
                );
                ui.horizontal(|ui| {
                    if ui.button("Parse curl").clicked() {
                        match parse_curl(&self.import.curl_text) {
                            Ok(parsed) => self.apply_parsed_request(parsed),
                            Err(e) => self.import.error = Some(e),
                        }
                    }
                    if ui.button("Import HAR file...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("HAR", &["har"]).pick_file() {
                            match parse_har(&path) {
                                Ok(entries) if entries.is_empty() => {
                                    self.import.error = Some("No requests found in that HAR file.".to_string());
                                }
                                Ok(entries) => {
                                    self.import.har_candidates = entries
                                        .into_iter()
                                        .map(|p| (format!("{} {}", p.method, p.url), p))
                                        .collect();
                                    self.import.error = None;
                                }
                                Err(e) => self.import.error = Some(e),
                            }
                        }
                    }
                });

                if let Some(err) = &self.import.error {
                    ui.colored_label(egui::Color32::from_rgb(230, 100, 90), err);
                }

                if !self.import.har_candidates.is_empty() {
                    ui.separator();
                    ui.label(format!(
                        "{} request(s) found — pick one to import:",
                        self.import.har_candidates.len()
                    ));
                    let mut pick: Option<usize> = None;
                    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                        for (i, (label, _)) in self.import.har_candidates.iter().enumerate() {
                            if ui.selectable_label(false, label).clicked() {
                                pick = Some(i);
                            }
                        }
                    });
                    if let Some(i) = pick {
                        let (_, parsed) = self.import.har_candidates.remove(i);
                        self.apply_parsed_request(parsed);
                    }
                }
            });
        if !still_open {
            self.import.reset();
        }
    }
}
