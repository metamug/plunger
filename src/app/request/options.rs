use crate::app::ApiTesterApp;
use crate::theme::AMBER;
use eframe::egui;

impl ApiTesterApp {
    pub(in crate::app) fn render_options_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Timeout");
            ui.add(egui::DragValue::new(&mut self.state.timeout_secs).range(1..=600).suffix(" s"));
        });
        ui.add_space(4.0);
        ui.checkbox(&mut self.state.follow_redirects, "Follow redirects");
        ui.add_space(4.0);
        ui.checkbox(&mut self.state.insecure_tls, "Skip TLS certificate verification");
        if self.state.insecure_tls {
            ui.colored_label(
                AMBER,
                "Certificates are not checked. Use this only for servers you trust, e.g. a local API with a self-signed certificate.",
            );
        }
    }
}
