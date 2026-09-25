//! Importing a request from a pasted curl command or from a HAR file. Each
//! has its own dialog; the imported request opens in a tab.

use super::{ApiTesterApp, ImportDialog};
use crate::curl_import::{parse_curl, parse_har};
use crate::theme::{self, palette};
use eframe::egui;

/// Tall enough for a long curl command; beyond this the box scrolls instead of
/// pushing the dialog off the screen.
const CURL_BOX_HEIGHT: f32 = 260.0;

impl ApiTesterApp {
    pub(super) fn open_curl_dialog(&mut self) {
        self.import = ImportDialog::Curl { text: String::new(), error: None, focus: true };
    }

    /// Picks a HAR file; one request imports straight away, several open a
    /// list to choose from.
    pub(super) fn open_har_file(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("HAR", &["har"]).pick_file() else {
            return;
        };
        match parse_har(&path) {
            Ok(entries) if entries.is_empty() => self.notify("No requests found in that HAR file."),
            Ok(mut entries) if entries.len() == 1 => self.open_parsed(entries.remove(0)),
            Ok(entries) => {
                let candidates = entries.into_iter().map(|p| (p.url.clone(), p)).collect();
                self.import = ImportDialog::Har { candidates };
            }
            Err(e) => self.notify(format!("Couldn't read the HAR file: {e}")),
        }
    }

    pub(super) fn render_import_windows(&mut self, ctx: &egui::Context) {
        match &self.import {
            ImportDialog::Closed => {}
            ImportDialog::Curl { .. } => self.render_curl_window(ctx),
            ImportDialog::Har { .. } => self.render_har_window(ctx),
        }
    }

    fn render_curl_window(&mut self, ctx: &egui::Context) {
        let ImportDialog::Curl { text, error, focus } = &mut self.import else { return };
        let mut open = true;
        let mut import = false;
        let mut cancel = false;
        egui::Window::new("Import a curl command")
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("Paste the command, e.g. from your browser DevTools: right-click a request > Copy > Copy as cURL.").weak().small());
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(CURL_BOX_HEIGHT).show(ui, |ui| {
                    let edit = ui.add(
                        theme::area(text)
                            .desired_rows(8)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("curl 'https://api.example.com/resource' -H 'Authorization: Bearer ...' -d '{...}'"),
                    );
                    if std::mem::take(focus) {
                        edit.request_focus();
                    }
                });
                if let Some(err) = error {
                    ui.colored_label(palette().error, err.as_str());
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    import = ui
                        .add_enabled(!text.trim().is_empty(), egui::Button::new("Import"))
                        .on_hover_text("Open this request in a tab")
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if import {
            let result = parse_curl(text);
            match result {
                Ok(parsed) => self.open_parsed(parsed),
                Err(e) => *error = Some(e),
            }
        } else if cancel || !open {
            self.import = ImportDialog::Closed;
        }
    }

    fn render_har_window(&mut self, ctx: &egui::Context) {
        let ImportDialog::Har { candidates } = &mut self.import else { return };
        let mut open = true;
        let mut pick = None;
        egui::Window::new("Import from HAR")
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("{} requests in this file. Pick one to open it in a tab:", candidates.len()));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (i, (label, parsed)) in candidates.iter().enumerate() {
                        let text = egui::RichText::new(label.as_str()).monospace();
                        let method = egui::RichText::new(&parsed.method).color(theme::method_color(&parsed.method));
                        ui.horizontal(|ui| {
                            ui.label(method);
                            if ui.add(egui::Label::new(text).truncate().sense(egui::Sense::click())).clicked() {
                                pick = Some(i);
                            }
                        });
                    }
                });
            });
        if let Some(i) = pick {
            let (_, parsed) = candidates.remove(i);
            self.open_parsed(parsed);
        } else if !open {
            self.import = ImportDialog::Closed;
        }
    }
}
