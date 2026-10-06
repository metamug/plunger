//! The request editor tabs: Params, Auth, Headers, Body, Variables, Options.

mod auth;
mod body;
mod headers;
mod options;
mod params;
mod rows;
mod suggest;

use crate::app::ApiTesterApp;
use crate::model::{BodyMode, PersistedState, RequestTab};
use crate::request::parse_headers;
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
            let labels = tab_labels(&tab.state, !self.bearer_token.is_empty());
            ui.selectable_value(&mut tab.request_tab, RequestTab::Params, &labels.params)
                .on_hover_text("Query parameters. They mirror the URL's query string: edit either one. Untick a row to leave it out of the URL.");
            ui.selectable_value(&mut tab.request_tab, RequestTab::Auth, &labels.auth)
                .on_hover_text("A Bearer token, sent as Authorization: Bearer <token> on every request from every tab.");
            ui.selectable_value(&mut tab.request_tab, RequestTab::Headers, &labels.headers)
                .on_hover_text("Request headers. {{variables}} work in the values.");
            ui.selectable_value(&mut tab.request_tab, RequestTab::Body, &labels.body)
                .on_hover_text("The request body: JSON, form-data, url-encoded or raw text.");
            ui.selectable_value(&mut tab.request_tab, RequestTab::Variables, &labels.variables).on_hover_text(
                "Use {{name}} in the URL, params, headers, body, form fields or the Bearer token.\n\
                 Built-ins: {{$uuid}}, {{$timestamp}}, {{$randomInt}} and {{$env:NAME}} (an environment variable).\n\n\
                 Secret values (lock on, or a name like token, secret, password or key) are never written to a file. \
                 Turn on the key to keep one in the system credential store; otherwise it is blank after a restart.",
            );
            let options_label = if tab.state.insecure_tls { "Options (TLS check off)" } else { "Options" };
            ui.selectable_value(&mut tab.request_tab, RequestTab::Options, options_label);
        });
        ui.add_space(4.0);
        let mut forget = false;
        let mut delete_agent = None;
        // A long request (40 headers, a big body) scrolls inside its own area
        // rather than pushing the response off the bottom of the window.
        egui::ScrollArea::vertical()
            .id_salt(("request-section", tab.id))
            .max_height((ui.available_height() * REQUEST_SHARE).max(MIN_REQUEST_HEIGHT))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                card(ui, |ui| match tab.request_tab {
                    RequestTab::Params => tab.render_params_tab(ui),
                    RequestTab::Variables => {
                        forget = tab.render_variables_tab(ui, self.secrets_error.as_deref(), &self.agent_variables, &mut delete_agent)
                    }
                    RequestTab::Auth => tab.render_auth_tab(ui, &mut self.bearer_token, self.secrets_error.as_deref()),
                    RequestTab::Headers => tab.render_headers_tab(ui),
                    RequestTab::Body => tab.render_body_tab(ui),
                    RequestTab::Options => tab.render_options_tab(ui),
                });
            });
        if forget {
            self.forget_secrets();
        }
        if let Some(name) = delete_agent {
            self.delete_agent_variable(&name);
        }
    }
}

/// The request tab titles: a count where the tab holds a list, a bullet where it holds something
/// else (a body, a token), so what is set is visible without opening each tab.
struct TabLabels {
    params: String,
    auth: String,
    headers: String,
    body: String,
    variables: String,
}

fn tab_labels(state: &PersistedState, has_bearer: bool) -> TabLabels {
    let count = |label: &str, n: usize| if n > 0 { format!("{label} ({n})") } else { label.to_string() };
    let marked = |label: &str, on: bool| if on { format!("{label} •") } else { label.to_string() };
    let has_body = match state.body_mode {
        BodyMode::None => false,
        BodyMode::Json => !state.json_body.trim().is_empty(),
        BodyMode::UrlEncoded => !state.urlencoded_body.trim().is_empty(),
        BodyMode::Raw => !state.raw_body.trim().is_empty(),
        BodyMode::Multipart => !state.multipart_fields.is_empty(),
    };
    TabLabels {
        params: count("Params", state.params.iter().filter(|p| p.enabled && !p.key.is_empty()).count()),
        auth: marked("Auth", has_bearer),
        headers: count("Headers", parse_headers(&state.headers_text).iter().filter(|(k, _)| !k.trim().is_empty()).count()),
        body: marked("Body", has_body),
        variables: count("Variables", state.variables.iter().filter(|v| !v.name.is_empty()).count()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tabs_have_plain_titles_and_filled_ones_show_what_they_hold() {
        let empty = tab_labels(&PersistedState::default(), false);
        assert_eq!(
            [&empty.params, &empty.auth, &empty.headers, &empty.body, &empty.variables],
            ["Params", "Auth", "Headers", "Body", "Variables"]
        );

        let state = PersistedState {
            headers_text: "Accept: */*\nContent-Type: application/json\n".into(),
            body_mode: BodyMode::Json,
            json_body: "{}".into(),
            ..Default::default()
        };
        let labels = tab_labels(&state, true);
        assert_eq!(labels.headers, "Headers (2)");
        assert_eq!(labels.body, "Body •");
        assert_eq!(labels.auth, "Auth •");

        let blank_body = PersistedState { body_mode: BodyMode::Json, json_body: "  ".into(), ..Default::default() };
        assert_eq!(tab_labels(&blank_body, false).body, "Body");
    }
}
