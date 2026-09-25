//! The request editor tabs: Params, Headers, Body, Variables, Options.

mod body;
mod headers;
mod options;
mod params;
mod rows;

use crate::app::ApiTesterApp;
use crate::model::RequestTab;
use crate::theme::card;
use eframe::egui;

/// At most this share of the space below the tabs goes to the request editor;
/// the response gets the rest.
const REQUEST_SHARE: f32 = 0.45;
const MIN_REQUEST_HEIGHT: f32 = 160.0;

impl ApiTesterApp {
    /// The Params / Headers / Body / Variables / Options strip and the
    /// selected panel. Headers and Variables also touch app-wide things (the
    /// shared Bearer token, the credential store), passed in explicitly.
    pub(in crate::app) fn render_request_section(&mut self, ui: &mut egui::Ui) {
        let tab = &mut self.tabs[self.active];
        ui.horizontal(|ui| {
            let params = tab.state.params.iter().filter(|p| p.enabled && !p.key.is_empty()).count();
            let vars = tab.state.variables.iter().filter(|v| !v.name.is_empty()).count();
            let count = |label: &str, n: usize| if n > 0 { format!("{label} ({n})") } else { label.to_string() };
            ui.selectable_value(&mut tab.request_tab, RequestTab::Params, count("Params", params));
            ui.selectable_value(&mut tab.request_tab, RequestTab::Headers, "Headers");
            ui.selectable_value(&mut tab.request_tab, RequestTab::Body, "Body");
            ui.selectable_value(&mut tab.request_tab, RequestTab::Variables, count("Variables", vars));
            let options_label = if tab.state.insecure_tls { "Options (TLS check off)" } else { "Options" };
            ui.selectable_value(&mut tab.request_tab, RequestTab::Options, options_label);
        });
        ui.add_space(4.0);
        let mut forget = false;
        // A long request (40 headers, a big body) scrolls inside its own area
        // rather than pushing the response off the bottom of the window.
        egui::ScrollArea::vertical()
            .id_salt(("request-section", tab.id))
            .max_height((ui.available_height() * REQUEST_SHARE).max(MIN_REQUEST_HEIGHT))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                card(ui, |ui| match tab.request_tab {
                    RequestTab::Params => tab.render_params_tab(ui),
                    RequestTab::Variables => forget = tab.render_variables_tab(ui, self.secrets_error.as_deref()),
                    RequestTab::Headers => {
                        tab.render_headers_tab(ui, &mut self.bearer_token, self.secrets_error.as_deref())
                    }
                    RequestTab::Body => tab.render_body_tab(ui),
                    RequestTab::Options => tab.render_options_tab(ui),
                });
            });
        if forget {
            self.forget_secrets();
        }
    }
}
