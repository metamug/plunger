//! The request editor tabs: Params, Auth, Headers, Body, Variables, Options.

mod auth;
mod body;
mod headers;
mod options;
mod params;
mod rows;
mod suggest;

use crate::app::tab::Pane;
use crate::app::ApiTesterApp;
use crate::ui::icons;
use crate::domain::model::{BodyMode, PersistedState, RequestTab};
use crate::domain::request::parse_headers;
use crate::ui::theme::compact_card;
use eframe::egui;

/// The request panel is as tall as its content and grows until the response would be squeezed below
/// `MIN_RESPONSE_HEIGHT`. Dragging the divider overrides that.
pub(in crate::app) const MIN_REQUEST_HEIGHT: f32 = 150.0;
/// Room kept for the response header row when the editor is dragged as far as it goes.
pub(in crate::app) const MIN_RESPONSE_HEIGHT: f32 = 120.0;
const LINE_HEIGHT: f32 = 17.0;
/// Space the editor needs besides its text lines: the body mode row, margins, the scroll bar.
const EDITOR_CHROME: f32 = 110.0;

/// A request tab pill; true when it was clicked (even if it was already the selected one).
fn request_tab_pill(ui: &mut egui::Ui, current: &mut RequestTab, value: RequestTab, label: &str, tip: &str) -> bool {
    ui.selectable_value(current, value, label).on_hover_text(tip).clicked()
}

impl ApiTesterApp {
    /// The Params / Auth / Headers / Body / Variables / Options strip and the selected panel. The
    /// panel grows with what is in it, up to the room there is (the response keeps a minimum), and is
    /// folded away when a request is sent so the response gets the room. Clicking a tab brings it back.
    pub(in crate::app) fn render_request_section(&mut self, ui: &mut egui::Ui) {
        let tab = &mut self.tabs[self.active];
        let mut clicked_tab = false;
        ui.horizontal(|ui| {
            let labels = tab_labels(&tab.state, !self.bearer_token.is_empty(), self.agent_variables.len());
            let current = &mut tab.request_tab;
            clicked_tab |= request_tab_pill(
                ui,
                current,
                RequestTab::Params,
                &labels.params,
                "Query parameters. They mirror the URL's query string: edit either one. Untick a row to leave it out of the URL.",
            );
            clicked_tab |= request_tab_pill(
                ui,
                current,
                RequestTab::Auth,
                &labels.auth,
                "A Bearer token, sent as Authorization: Bearer <token> on every request from every tab.",
            );
            clicked_tab |= request_tab_pill(ui, current, RequestTab::Headers, &labels.headers, "Request headers. {{variables}} work in the values.");
            clicked_tab |= request_tab_pill(ui, current, RequestTab::Body, &labels.body, "The request body: JSON, form-data, url-encoded or raw text.");
            clicked_tab |= request_tab_pill(
                ui,
                current,
                RequestTab::Variables,
                &labels.variables,
                "Use {{name}} in the URL, params, headers, body, form fields or the Bearer token.\n\
                 Built-ins: {{$uuid}}, {{$timestamp}}, {{$randomInt}} and {{$env:NAME}} (an environment variable).\n\n\
                 Secret values (lock on, or a name like token, secret, password or key) are never written to a file. \
                 Turn on the key to keep one in the system credential store; otherwise it is blank after a restart.",
            );
            let options_label = if tab.state.insecure_tls { "Options (TLS check off)" } else { "Options" };
            clicked_tab |= ui.selectable_value(&mut tab.request_tab, RequestTab::Options, options_label).clicked();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // The two-arrow icon says which panel is expanded: arrows pointing in on the one that is.
                let expanded = tab.pane == Pane::RequestExpanded;
                if icons::panel_toggle(
                    ui,
                    expanded,
                    "The request fills the window. Click to share it with the response again (it folds away when you send)",
                    "Expand the request to the whole window (it folds away when you send)",
                )
                .clicked()
                {
                    tab.pane = if expanded { Pane::Both } else { Pane::RequestExpanded };
                }
                if tab.pane != Pane::ResponseExpanded {
                    match tab.request_tab {
                        RequestTab::Headers => tab.headers_view_toggle(ui),
                        RequestTab::Body => tab.body_controls(ui),
                        _ => {}
                    }
                }
            });
        });
        // A tab clicked while the editor is folded away brings it back.
        if clicked_tab && tab.pane == Pane::ResponseExpanded {
            tab.pane = Pane::Both;
        }
        if tab.pane == Pane::ResponseExpanded {
            return;
        }
        ui.add_space(2.0);
        let mut forget = false;
        let mut delete_agent = None;
        let available = ui.available_height();
        let (height, fill) = match (tab.pane, tab.request_height) {
            // Leave room for the line that says the response is folded away.
            (Pane::RequestExpanded, _) => ((available - 50.0).max(MIN_REQUEST_HEIGHT), true),
            // A height the user dragged to is kept, within what the window can hold.
            (_, Some(dragged)) => (dragged.clamp(MIN_REQUEST_HEIGHT, (available - MIN_RESPONSE_HEIGHT).max(MIN_REQUEST_HEIGHT)), true),
            // Otherwise the panel is as tall as what is in it, and grows until the response would be squeezed.
            _ => ((available - MIN_RESPONSE_HEIGHT).max(MIN_REQUEST_HEIGHT), false),
        };
        // Body editors: the whole surface when the panel is given the room, else just their text.
        tab.editor_fill = fill;
        tab.editor_rows = if fill { (((height - EDITOR_CHROME) / LINE_HEIGHT) as usize).max(8) } else { 60 };
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
