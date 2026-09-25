use super::copy_button;
use super::ApiTesterApp;
use crate::icons::{self, Icon};
use crate::theme::{self, palette, ACCENT};
use eframe::egui;

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

impl ApiTesterApp {
    /// Method + URL (one joined control), then Send, Save and Import.
    pub(super) fn render_command_bar(&mut self, ui: &mut egui::Ui) {
        let loading = self.tab().is_loading();
        let (mut send, mut cancel, mut save) = (false, false, false);
        let (mut open_curl, mut open_har) = (false, false);

        ui.horizontal(|ui| {
            // Right-hand buttons first (right to left), so the URL bar can take
            // exactly the width that is left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let import = icons::button(ui, Icon::Import, "Import a request: paste a curl command or open a HAR file");
                let popup_id = ui.make_persistent_id("import-menu");
                if import.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(popup_id));
                }
                egui::popup_below_widget(ui, popup_id, &import, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    ui.set_min_width(190.0);
                    if menu_item(ui, "Paste a curl command\u{2026}") {
                        open_curl = true;
                    }
                    if menu_item(ui, "Open a HAR file\u{2026}") {
                        open_har = true;
                    }
                });

                let save_tip = if self.tab().saved_id.is_some() {
                    "Save changes to this request (Ctrl+S)"
                } else {
                    "Save this request to the Saved list (Ctrl+S)"
                };
                save = icons::button(ui, Icon::Save, save_tip).clicked();

                if loading {
                    cancel = ui.button("Cancel").on_hover_text("Stop waiting for the response").clicked();
                }
                send = ui
                    .add_enabled(
                        !loading,
                        egui::Button::new(if loading { "Sending\u{2026}" } else { "Send" }).fill(ACCENT.linear_multiply(0.35)),
                    )
                    .on_hover_text("Send the request (Ctrl+Enter)")
                    .clicked();

                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| self.render_url_bar(ui));
            });
        });

        if send {
            self.trigger_send(ui.ctx());
        }
        if cancel {
            self.tab_mut().cancel();
        }
        if save {
            self.save_active();
        }
        if open_curl {
            self.open_curl_dialog();
        }
        if open_har {
            self.open_har_file();
        }
    }

    /// The method dropdown and URL field rendered as one joined control
    /// (shared border/background, zero gap) instead of two separate boxes.
    fn render_url_bar(&mut self, ui: &mut egui::Ui) {
        let p = palette();
        let tab = &mut self.tabs[self.active];
        egui::Frame::none()
            .fill(p.input)
            .stroke(egui::Stroke::new(1.0_f32, p.border))
            .rounding(egui::Rounding::same(6.0))
            .inner_margin(egui::Margin::symmetric(3.0, 2.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal(|ui| {
                    ui.scope(|ui| {
                        let widgets = &mut ui.visuals_mut().widgets;
                        widgets.inactive.bg_stroke = egui::Stroke::NONE;
                        widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                        widgets.hovered.bg_stroke = egui::Stroke::NONE;
                        widgets.hovered.weak_bg_fill = p.hover;
                        widgets.hovered.rounding = egui::Rounding::same(4.0);
                        egui::ComboBox::from_id_salt(("method", tab.id))
                            .selected_text(
                                egui::RichText::new(&tab.state.method).strong().color(theme::method_color(&tab.state.method)),
                            )
                            .width(72.0)
                            .show_ui(ui, |ui| {
                                for m in METHODS {
                                    ui.selectable_value(&mut tab.state.method, m.to_string(), m);
                                }
                            });
                    });
                    ui.add(egui::Separator::default().vertical().spacing(2.0));
                    ui.scope(|ui| {
                        let widgets = &mut ui.visuals_mut().widgets;
                        widgets.inactive.bg_stroke = egui::Stroke::NONE;
                        widgets.hovered.bg_stroke = egui::Stroke::NONE;
                        ui.add(
                            theme::field(&mut tab.state.url)
                                .id_salt(("url", tab.id))
                                .desired_width(ui.available_width() - icons::SIZE - theme::FIELD_MARGIN_X)
                                .hint_text("https://api.example.com/resource  or  localhost:3000/api")
                                .frame(false),
                        );
                    });
                    if !tab.state.url.is_empty() {
                        copy_button(ui, &mut tab.copied_flash, "url", "Copy URL", &tab.state.url);
                    }
                });
            });
    }
}

/// A full-width, frameless menu entry.
fn menu_item(ui: &mut egui::Ui, label: &str) -> bool {
    ui.add(egui::Button::new(label).frame(false).min_size(egui::vec2(ui.available_width(), 0.0)))
        .clicked()
}
