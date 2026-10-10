use crate::app::tab::Tab;
use crate::ui::icons::{self, Icon};
use crate::ui::theme::{self, palette};
use eframe::egui;

impl Tab {
    /// The Bearer token sent as `Authorization: Bearer <token>`. It is app-wide (shared by every
    /// tab), and only written to the system credential store when "remember" is on.
    pub(in crate::app) fn render_auth_tab(&mut self, ui: &mut egui::Ui, bearer: &mut String, secrets_error: Option<&str>) {
        ui.horizontal(|ui| {
            ui.label("Bearer token")
                .on_hover_text("Sent as \"Authorization: Bearer <token>\" on every request from every tab. {{variables}} work here.");
            let hint = if self.state.remember_bearer {
                "token — kept in the system credential store"
            } else {
                "token — not saved between runs"
            };
            let width = ui.available_width() - icons::trailing_room(ui, 1);
            ui.add(theme::field(bearer).desired_width(width).hint_text(hint).password(true));
            icons::toggle(
                ui,
                &mut self.state.remember_bearer,
                Icon::Key,
                "Remembered in the system credential store (never in a file). Click to stop remembering",
                "Not remembered: cleared when the app closes. Click to keep it in the system credential store",
            );
        });
        if let Some(err) = secrets_error {
            ui.colored_label(palette().error, err);
        }
    }
}
