//! The Params and Variables tabs: both are plain lists of name/value rows.

use super::rows::{edit_rows, enabled_checkbox, remove_button};
use crate::app::tab::Tab;
use crate::icons::{self, Icon};
use crate::model::{KeyValue, Variable};
use crate::query::{parse_query, url_with_params};
use crate::redact::is_sensitive_header;
use crate::theme::{self, palette};
use eframe::egui;

impl Tab {
    pub(in crate::app) fn render_params_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Mirrors the URL's query string: edit either one. Untick a row to leave it out of the URL.")
                .weak()
                .small(),
        );
        ui.add_space(6.0);

        let before = self.state.params.clone();
        edit_rows(
            ui,
            &mut self.state.params,
            KeyValue::blank,
            KeyValue::is_blank,
            |ui, p| {
                ui.horizontal(|ui| {
                    enabled_checkbox(ui, &mut p.enabled);
                    ui.add(theme::field(&mut p.key).desired_width(200.0).hint_text("name"));
                    let width = ui.available_width() - icons::trailing_room(ui, 1);
                    ui.add(theme::field(&mut p.value).desired_width(width).hint_text("value  (or {{variable}})"));
                    remove_button(ui, "parameter")
                })
                .inner
            },
        );
        if self.state.params != before {
            self.sync_url_from_params();
        }
    }

    /// Writes the table back into the URL's query string. Only rewrites the
    /// URL when its query actually changes, so clicking into the table never
    /// reformats what the user typed.
    fn sync_url_from_params(&mut self) {
        let url = url_with_params(&self.state.url, &self.state.params);
        if parse_query(&url) != parse_query(&self.state.url) {
            self.state.url = url;
        }
        self.synced_url = self.state.url.clone();
    }

    #[cfg(test)]
    pub(in crate::app) fn render_params_rows_changed_for_test(&mut self) {
        self.sync_url_from_params();
    }

    /// Returns true when "forget saved secrets" was clicked; the app owns the
    /// credential store, so it does the forgetting.
    pub(in crate::app) fn render_variables_tab(&mut self, ui: &mut egui::Ui, secrets_error: Option<&str>) -> bool {
        let mut forget = false;
        ui.label(
            egui::RichText::new(
                "Use {{name}} in the URL, params, headers, body, form fields or Bearer token. Built-ins: {{$uuid}}, {{$timestamp}}, {{$randomInt}}.",
            )
            .weak()
            .small(),
        );
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(
                    "Secret values (lock on, or named like token/secret/password/key) are never written to a file. Turn on the key to keep one in the system credential store; otherwise it is blank after a restart.",
                )
                .weak()
                .small(),
            );
            // Next to the text it explains, not in the per-row column, so it
            // can't be mistaken for "remove this variable".
            if icons::button(
                ui,
                Icon::Trash,
                "Forget saved secrets: remove every remembered secret (including the Bearer token) from the system credential store",
            )
            .clicked()
            {
                forget = true;
            }
        });
        if let Some(err) = secrets_error {
            ui.colored_label(palette().error, err);
        }
        ui.add_space(6.0);

        edit_rows(
            ui,
            &mut self.state.variables,
            Variable::default,
            Variable::is_blank,
            |ui, v| {
                ui.horizontal(|ui| {
                    ui.add(theme::field(&mut v.name).desired_width(160.0).hint_text("name"));
                    let mask = v.is_secret();
                    let width = ui.available_width() - icons::trailing_room(ui, 3);
                    ui.add(theme::field(&mut v.value).desired_width(width).hint_text("value").password(mask));
                    // A credential-looking name is always secret, so its lock is stuck on.
                    let forced = is_sensitive_header(&v.name);
                    let mut secret = v.secret || forced;
                    let lock = ui.add_enabled_ui(!forced, |ui| {
                        icons::toggle(
                            ui,
                            &mut secret,
                            Icon::Lock,
                            if forced {
                                "Secret: the name looks like a credential, so it is always masked and never written to a file"
                            } else {
                                "Secret: masked and never written to a file. Click to make it a plain value"
                            },
                            "Plain value. Click to make it secret (masked, never written to a file)",
                        )
                    });
                    if lock.inner.changed() {
                        v.secret = secret;
                    }
                    ui.add_enabled_ui(mask, |ui| {
                        icons::toggle(
                            ui,
                            &mut v.remember,
                            Icon::Key,
                            "Remembered in the system credential store. Click to stop remembering",
                            if mask {
                                "Not remembered: blank after a restart. Click to keep it in the system credential store"
                            } else {
                                "Only secret values can be remembered"
                            },
                        )
                    });
                    remove_button(ui, "variable")
                })
                .inner
            },
        );
        forget
    }
}
