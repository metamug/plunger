//! The request editor tabs: Params, Auth, Headers, Body, Variables, Options.

mod auth;
mod body;
mod headers;
mod options;
mod params;
mod rows;
mod suggest;

use crate::app::tab::Pane;
use crate::model::Outcome;
use crate::app::ApiTesterApp;
use crate::icons::{self, Icon};
use crate::model::{BodyMode, PersistedState, RequestTab};
use crate::request::parse_headers;
use crate::theme::compact_card;
use eframe::egui;

/// How much of the space below the tabs the request editor may take. Before there is a response the
/// request is what you are working on, so it gets most of it; once a response is on screen, the
/// response is. Dragging the divider overrides both.
const SHARE_BEFORE_RESPONSE: f32 = 0.72;
const SHARE_WITH_RESPONSE: f32 = 0.38;
pub(in crate::app) const MIN_REQUEST_HEIGHT: f32 = 150.0;
/// Room kept for the response header row when the editor is dragged as far as it goes.
pub(in crate::app) const MIN_RESPONSE_HEIGHT: f32 = 120.0;
const LINE_HEIGHT: f32 = 17.0;
/// Space the editor needs besides its text lines: the body mode row, margins, the scroll bar.
const EDITOR_CHROME: f32 = 110.0;

impl ApiTesterApp {
    /// The Params / Headers / Body / Variables / Options strip and the
    /// selected panel. Headers and Variables also touch app-wide things (the
    /// shared Bearer token, the credential store), passed in explicitly.
    pub(in crate::app) fn render_request_section(&mut self, ui: &mut egui::Ui) {
        let tab = &mut self.tabs[self.active];
        ui.horizontal(|ui| {
            let labels = tab_labels(&tab.state, !self.bearer_token.is_empty(), self.agent_variables.len());
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
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // From the right: fold the response away, fold this editor away, then the Headers view switch.
                let mut hide_response = tab.pane == Pane::ResponseHidden;
                if icons::toggle(
                    ui,
                    &mut hide_response,
                    Icon::ChevronDown,
                    "The request is expanded. Click to show the response again (it folds back when you send)",
                    "Expand the request editor to the whole height (it folds back when you send)",
                )
                .changed()
                {
                    tab.pane = if hide_response { Pane::ResponseHidden } else { Pane::Both };
                }
                let mut hide_request = tab.pane == Pane::RequestHidden;
                if icons::toggle(
                    ui,
                    &mut hide_request,
                    Icon::ChevronUp,
                    "The request editor is hidden. Click to show it again",
                    "Hide the request editor to give the response the whole height",
                )
                .changed()
                {
                    tab.pane = if hide_request { Pane::RequestHidden } else { Pane::Both };
                }
                if tab.request_tab == RequestTab::Headers && tab.pane != Pane::RequestHidden {
                    tab.headers_view_toggle(ui);
                }
            });
        });
        if tab.pane == Pane::RequestHidden {
            return;
        }
        ui.add_space(2.0);
        let mut forget = false;
        let mut delete_agent = None;
        // A long request (40 headers, a big body) scrolls inside its own area
        // rather than pushing the response off the bottom of the window.
        let available = ui.available_height();
        let has_response = !matches!(tab.outcome, Outcome::Empty);
        let (height, fill) = match (tab.pane, tab.request_height) {
            // Leave room for the line that says the response is folded away.
            (Pane::ResponseHidden, _) => ((available - 50.0).max(MIN_REQUEST_HEIGHT), true),
            // A height the user dragged to is kept, within what the window can hold.
            (_, Some(dragged)) => (dragged.clamp(MIN_REQUEST_HEIGHT, (available - MIN_RESPONSE_HEIGHT).max(MIN_REQUEST_HEIGHT)), true),
            _ => {
                let share = if has_response { SHARE_WITH_RESPONSE } else { SHARE_BEFORE_RESPONSE };
                ((available * share).max(MIN_REQUEST_HEIGHT), false)
            }
        };
        // Body editors take the lines the room allows, so a JSON body is not squeezed into three rows.
        tab.editor_rows = if fill { (((height - EDITOR_CHROME) / LINE_HEIGHT) as usize).max(8) } else { 8 };
        let shown = egui::ScrollArea::vertical()
            .id_salt(("request-section", tab.id))
            .max_height(height)
            .min_scrolled_height(if fill { height } else { 0.0 })
            .auto_shrink([false, !fill])
            .show(ui, |ui| {
                compact_card(ui, |ui| match tab.request_tab {
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
        tab.request_shown_height = shown.inner_rect.height();
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

fn tab_labels(state: &PersistedState, has_bearer: bool, agent_variables: usize) -> TabLabels {
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
        variables: count("Variables", state.variables.iter().filter(|v| !v.name.is_empty()).count() + agent_variables),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tabs_have_plain_titles_and_filled_ones_show_what_they_hold() {
        let empty = tab_labels(&PersistedState::default(), false, 0);
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
        let labels = tab_labels(&state, true, 0);
        assert_eq!(labels.headers, "Headers (2)");
        assert_eq!(labels.body, "Body •");
        assert_eq!(labels.auth, "Auth •");

        let blank_body = PersistedState { body_mode: BodyMode::Json, json_body: "  ".into(), ..Default::default() };
        assert_eq!(tab_labels(&blank_body, false, 0).body, "Body");
        assert_eq!(tab_labels(&PersistedState::default(), false, 3).variables, "Variables (3)", "variables agents set count too");
    }
}
