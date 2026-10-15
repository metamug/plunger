use super::copy_button;
use super::ApiTesterApp;
use crate::ui::icons::{self};
use crate::ui::theme::{self, palette};
use eframe::egui;

pub(super) fn url_field_id(tab_id: u64) -> egui::Id {
    egui::Id::new(("url", tab_id))
}

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

impl ApiTesterApp {
    /// Method + URL (one joined control), then Send. Save and Import live in the File menu.
    pub(super) fn render_command_bar(&mut self, ui: &mut egui::Ui) {
        let loading = self.tab().is_loading();
        let (mut send, mut cancel) = (false, false);

        ui.horizontal(|ui| {
            // Right-hand buttons first (right to left), so the URL bar can take
            // exactly the width that is left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if loading {
                    cancel = ui.button("Cancel").on_hover_text("Stop waiting for the response").clicked();
                }
                // as tall as the URL box beside it (its text row, field margin and frame)
                let height = ui.text_style_height(&egui::TextStyle::Body) + 2.0 * 6.0 + 2.0 * 2.0 + 2.0;
                send = ui
                    .add_enabled(
                        !loading,
                        egui::Button::new(egui::RichText::new(if loading { "Sending\u{2026}" } else { "Send" }).strong())
                            .fill(palette().accent_soft)
                            .min_size(egui::vec2(96.0, height)),
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
    }

    /// The method dropdown and URL field rendered as one joined control
    /// (shared border/background, zero gap) instead of two separate boxes.
    fn render_url_bar(&mut self, ui: &mut egui::Ui) {
        let p = palette();
        let tab = &mut self.tabs[self.active];
        egui::Frame::NONE
            .fill(p.input)
            .stroke(egui::Stroke::new(1.0_f32, p.border))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(3, 2))
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
                        widgets.hovered.corner_radius = egui::CornerRadius::same(4);
                        egui::ComboBox::from_id_salt(("method", tab.id))
                            .selected_text(
                                egui::RichText::new(&tab.state.method).strong().color(theme::method_color(&tab.state.method)),
                            )
                            .width(72.0)
                            // The default popup height (200 px) fits five methods and hid HEAD and OPTIONS.
                            .height(300.0)
                            .show_ui(ui, |ui| {
                                for m in METHODS {
                                    ui.selectable_value(&mut tab.state.method, m.to_string(), egui::RichText::new(m).color(theme::method_color(m)));
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
                                .layouter(&mut crate::ui::highlight::variable_layouter(&tab.state.variables))
                                .id(url_field_id(tab.id))
                                .desired_width(ui.available_width() - icons::SIZE - theme::FIELD_MARGIN_X)
                                .hint_text("https://api.example.com/resource  or  localhost:3000/api")
                                .frame(egui::Frame::NONE),
                        );
                    });
                    if !tab.state.url.is_empty() {
                        copy_button(ui, &mut tab.copied_flash, "url", "Copy URL", &tab.state.url);
                    }
                });
            });
    }
}
