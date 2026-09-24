use super::ApiTesterApp;
use crate::theme::{self, ACCENT};
use eframe::egui;

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
const SIDE_BUTTONS_WIDTH: f32 = 150.0;

impl ApiTesterApp {
    /// The method dropdown and URL field rendered as one joined control
    /// (shared border/background, zero gap) instead of two separate boxes.
    pub(super) fn render_command_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, ctrl_enter: bool) {
        let loading = self.is_loading();
        ui.horizontal(|ui| {
            let bar_width = ui.available_width() - SIDE_BUTTONS_WIDTH;

            egui::Frame::none()
                .fill(theme::INPUT_FILL)
                .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
                .rounding(egui::Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(2.0, 2.0))
                .show(ui, |ui| {
                    ui.set_width(bar_width);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.horizontal(|ui| {
                        ui.scope(|ui| {
                            let widgets = &mut ui.visuals_mut().widgets;
                            widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                            widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            widgets.hovered.bg_fill = theme::CARD_FILL;
                            widgets.hovered.rounding = egui::Rounding::same(4.0);
                            egui::ComboBox::from_id_salt("method")
                                .selected_text(egui::RichText::new(&self.state.method).strong())
                                .width(72.0)
                                .show_ui(ui, |ui| {
                                    for m in METHODS {
                                        ui.selectable_value(&mut self.state.method, m.to_string(), m);
                                    }
                                });
                        });
                        ui.add(egui::Separator::default().vertical().spacing(2.0));
                        ui.scope(|ui| {
                            let widgets = &mut ui.visuals_mut().widgets;
                            widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            ui.add(
                                egui::TextEdit::singleline(&mut self.state.url)
                                    .desired_width(ui.available_width())
                                    .hint_text("https://api.example.com/resource  or  localhost:3000/api")
                                    .frame(false),
                            );
                        });
                    });
                });

            let send_clicked = ui
                .add_enabled(
                    !loading,
                    egui::Button::new(if loading { "Sending…" } else { "Send" }).fill(ACCENT.linear_multiply(0.35)),
                )
                .on_hover_text("Ctrl+Enter")
                .clicked();

            if loading {
                if ui.button("Cancel").clicked() {
                    self.cancel_send();
                }
            } else if ui.button("Import").clicked() {
                self.import.open = true;
            }

            if send_clicked || (ctrl_enter && !loading) {
                self.trigger_send(ctx);
            }
        });
    }
}
