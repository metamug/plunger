//! The Params and Variables tabs: both are plain lists of name/value rows.

use super::rows::{edit_rows, enabled_checkbox, remove_button};
use super::suggest::variable_chips;
use crate::app::tab::Tab;
use crate::icons::{self, Icon};
use crate::model::{KeyValue, Variable};
use crate::query::{parse_query, url_with_params};
use crate::redact::is_sensitive_header;
use crate::theme::{self, palette};
use eframe::egui;

impl Tab {
    pub(in crate::app) fn render_params_tab(&mut self, ui: &mut egui::Ui) {
        let before = self.state.params.clone();
        let variables = &self.state.variables;
        edit_rows(
            ui,
            &mut self.state.params,
            KeyValue::blank,
            KeyValue::is_blank,
            |ui, p, spare| {
                let (value_resp, remove) = ui
                    .horizontal(|ui| {
                        enabled_checkbox(ui, &mut p.enabled);
                        ui.add(theme::field(&mut p.key).desired_width(200.0).hint_text("name"));
                        let width = ui.available_width() - icons::trailing_room(ui, 1);
                        let value_resp =
                            ui.add(
                                theme::field(&mut p.value)
                                    .desired_width(width)
                                    .hint_text("value  (or {{variable}})")
                                    .layouter(&mut crate::highlight::variable_layouter(variables)),
                            );
                        (value_resp, remove_button(ui, "parameter", spare))
                    })
                    .inner;
                variable_chips(ui, &value_resp, &mut p.value, variables);
                remove
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
    pub(in crate::app) fn render_variables_tab(
        &mut self,
        ui: &mut egui::Ui,
        secrets_error: Option<&str>,
        agent_variables: &[crate::history::AgentVariable],
        delete_agent: &mut Option<String>,
    ) -> bool {
        let mut forget = false;
        if let Some(err) = secrets_error {
            ui.colored_label(palette().error, err);
        }

        edit_rows(
            ui,
            &mut self.state.variables,
            Variable::default,
            Variable::is_blank,
            |ui, v, spare| {
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
                    remove_button(ui, "variable", spare)
                })
                .inner
            },
        );
        if self.state.variables.iter().any(|v| v.remember && v.is_secret()) || self.state.remember_bearer {
            ui.add_space(4.0);
            if ui
                .small_button("Forget remembered secrets")
                .on_hover_text("Remove every remembered secret (including the Bearer token) from the system credential store")
                .clicked()
            {
                forget = true;
            }
        }
        if !agent_variables.is_empty() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Set by agents").weak().small())
                .on_hover_text("Variables an agent set from the command line or over MCP. Requests use them like your own; remove one with the trash icon.");
            for var in agent_variables {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&var.name).monospace());
                    let shown = if var.secret { "••••".to_string() } else { var.value.clone() };
                    ui.add(egui::Label::new(egui::RichText::new(shown).monospace().weak()).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Icon::Trash, "Remove this variable").clicked() {
                            *delete_agent = Some(var.name.clone());
                        }
                        ui.label(egui::RichText::new(&var.source).weak().small());
                    });
                });
            }
        }
        forget
    }
}
