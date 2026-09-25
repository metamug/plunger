//! The Params and Variables tabs: both are plain lists of name/value rows.

use super::rows::{edit_rows, remove_button};
use crate::app::ApiTesterApp;
use crate::model::{KeyValue, Variable};
use crate::redact::is_sensitive_header;
use eframe::egui;

impl ApiTesterApp {
    pub(in crate::app) fn render_params_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Appended to the URL's query string when you send. Values are URL-encoded for you.")
                .weak()
                .small(),
        );
        ui.add_space(6.0);

        edit_rows(
            ui,
            &mut self.state.params,
            KeyValue::blank,
            KeyValue::is_blank,
            |ui, p| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.enabled, "");
                    ui.add(egui::TextEdit::singleline(&mut p.key).desired_width(200.0).hint_text("name"));
                    ui.add(
                        egui::TextEdit::singleline(&mut p.value)
                            .desired_width(ui.available_width() - 34.0)
                            .hint_text("value  (or {{variable}})"),
                    );
                    remove_button(ui)
                })
                .inner
            },
        );
    }

    pub(in crate::app) fn render_variables_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(
                "Use {{name}} in the URL, params, headers, body, form fields or Bearer token. Built-ins: {{$uuid}}, {{$timestamp}}, {{$randomInt}}.",
            )
            .weak()
            .small(),
        );
        ui.label(
            egui::RichText::new(
                "Secret values (ticked, or named like token/secret/password/key) stay in memory only and are blank after a restart.",
            )
            .weak()
            .small(),
        );
        ui.add_space(6.0);

        edit_rows(
            ui,
            &mut self.state.variables,
            Variable::default,
            Variable::is_blank,
            |ui, v| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut v.name).desired_width(160.0).hint_text("name"));
                    let mask = v.is_secret();
                    ui.add(
                        egui::TextEdit::singleline(&mut v.value)
                            .desired_width(ui.available_width() - 100.0)
                            .hint_text("value")
                            .password(mask),
                    );
                    // A credential-looking name is always secret, so its box is locked on.
                    let forced = is_sensitive_header(&v.name);
                    let mut ticked = v.secret || forced;
                    if ui.add_enabled(!forced, egui::Checkbox::new(&mut ticked, "secret")).changed() {
                        v.secret = ticked;
                    }
                    remove_button(ui)
                })
                .inner
            },
        );
    }
}
