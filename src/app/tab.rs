//! One open request: its form, its response, and its in-flight send. The app
//! holds several of these as tabs.

use crate::store::history::HistoryEntry;
use crate::domain::http::send_request;
use crate::domain::model::{Outcome, ParsedRequest, PersistedState, RequestTab, ResponseTab, SendResult, Variable};
use crate::domain::query::{params_from_url, reconcile};
use crate::domain::request::{parse_headers, prepare_to_send};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

/// Which copy button was clicked last, and when — so only that button shows
/// the "copied" check mark.
pub(super) type CopiedFlash = Option<(Instant, &'static str)>;

/// The request lifecycle as one value, so "loading", "who to poll" and "what
/// was sent" can't drift out of sync with each other.
/// How the window's height is shared between the request editor and the response.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Pane {
    /// Both are shown: the request as tall as its content, the response gets the rest.
    #[default]
    Both,
    /// The request fills the window; the response is folded away until you ask for it.
    RequestExpanded,
    /// The response fills the window; the request editor is folded away (its tabs stay). This is what
    /// you see after sending, and clicking a request tab brings the editor back.
    ResponseExpanded,
}

pub(super) enum RequestStatus {
    Idle,
    InFlight {
        rx: Receiver<SendResult>,
        /// Snapshot taken when Send was clicked, so the history row reflects
        /// what was actually sent even if the form is edited before the
        /// response arrives.
        sent: Box<PersistedState>,
    },
}

/// A tab as remembered between runs (credentials already redacted).
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct SavedTab {
    pub state: PersistedState,
    pub name: Option<String>,
    pub saved_id: Option<i64>,
    pub history_id: Option<i64>,
}

/// A send that just completed, for the history.
pub(super) struct Finished {
    pub sent: Box<PersistedState>,
    pub status: Option<u16>,
    pub elapsed_ms: Option<u128>,
}

pub(super) struct Tab {
    /// Stable for the life of the tab; used for egui ids.
    pub id: u64,
    pub state: PersistedState,
    /// The saved request's name, shown as the tab title.
    pub name: Option<String>,
    /// The saved request this tab edits; Save writes back to it.
    pub saved_id: Option<i64>,
    /// The history row this tab came from (or last sent), highlighted in the sidebar.
    pub history_id: Option<i64>,

    pub request_tab: RequestTab,
    /// The URL as of the last URL <-> Params sync; when the field no longer
    /// matches, the user edited the URL and the table is rebuilt from it.
    pub synced_url: String,
    pub headers_as_text: bool,
    pub header_rows: Vec<(String, String)>,
    pub response_tab: ResponseTab,
    pub status: RequestStatus,
    pub outcome: Outcome,
    pub copied_flash: CopiedFlash,
    pub save_error: Option<String>,
    pub response_search_open: bool,
    pub response_search_query: String,
    pub response_search_index: usize,
    /// Set by Ctrl+F: focus the search box on its next frame.
    pub response_search_focus: bool,
    /// Frames left to keep trying to scroll the current match into view (a tree node may need a frame to expand).
    pub response_search_scroll: u8,
    pub response_search_cache: super::response_search::Cache,
    /// The formatted XML/HTML view of the response on screen.
    pub markup_cache: Option<super::response_panel::MarkupView>,
    pub pane: Pane,
    /// The height the user dragged the divider to; None lets the window decide from what is on screen.
    pub request_height: Option<f32>,
    /// How tall the request editor was drawn last frame, so a drag starts from what the user sees.
    pub request_shown_height: f32,
    /// Lines the body editors ask for: a few normally, more when the editor has been given the room.
    pub editor_rows: usize,
    /// The body editors fill the panel (it was expanded or dragged) rather than hugging their text.
    pub editor_fill: bool,
    /// Set when this tab was opened from a history row: when it was sent and how it went (the
    /// response itself is not stored).
    pub opened_from: Option<HistoryMeta>,
    /// The values this request's placeholders had when it was sent (a tab opened from the history).
    pub sent_values: std::collections::BTreeMap<String, String>,
}

/// What the history knows about a request that was sent earlier.
pub struct HistoryMeta {
    pub created_at: String,
    pub status: Option<i64>,
    pub elapsed_ms: Option<i64>,
    pub source: crate::store::history::Source,
    /// Why no response came back, if that is what happened.
    pub error: Option<String>,
}

/// A short name for a request that has none: the last two path segments (or the host when the URL has no
/// path), so two requests to different ids or resources are told apart.
pub fn title_from_url(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() {
        return "New request".to_string();
    }
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = without_scheme.split(['?', '#']).next().unwrap_or(without_scheme);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        [] => path.to_string(),
        // only a host (or a lone segment): that is all there is
        [only] => (*only).to_string(),
        // the first segment is the host or `{{base}}`: it is not what tells requests apart
        [_host, rest @ ..] if rest.len() == 1 => rest[0].to_string(),
        [_host, rest @ ..] => rest[rest.len() - 2..].join("/"),
    }
}

/// The host (with port) and the query of a URL written with or without a scheme.
fn host_and_query(url: &str) -> (String, String) {
    let without_scheme = url.trim().split_once("://").map_or(url.trim(), |(_, rest)| rest);
    let before_fragment = without_scheme.split('#').next().unwrap_or(without_scheme);
    let (path, query) = before_fragment.split_once('?').unwrap_or((before_fragment, ""));
    let host = path.split('/').next().unwrap_or("").to_string();
    (host, query.to_string())
}

/// The titles to show on the tab strip. Tabs that would read the same (same method and title) are told
/// apart by their host, then by their query, then by a count, so two `GET items` are never two identical chips.
pub fn display_titles(tabs: &[Tab]) -> Vec<String> {
    let base: Vec<(String, String)> = tabs.iter().map(|t| (t.state.method.clone(), t.title())).collect();
    let mut out: Vec<String> = base.iter().map(|(_, title)| title.clone()).collect();
    for (i, key) in base.iter().enumerate() {
        let group: Vec<usize> = (0..base.len()).filter(|&j| base[j] == *key).collect();
        if group.len() < 2 || group[0] != i {
            continue;
        }
        let parts: Vec<(String, String)> = group.iter().map(|&j| host_and_query(&tabs[j].state.url)).collect();
        let distinct = |values: Vec<&String>| {
            let mut seen = std::collections::HashSet::new();
            values.into_iter().all(|v| seen.insert(v.clone()))
        };
        if distinct(parts.iter().map(|(host, _)| host).collect()) {
            for (&j, (host, _)) in group.iter().zip(&parts) {
                out[j] = format!("{} \u{b7} {host}", key.1);
            }
        } else if distinct(parts.iter().map(|(_, query)| query).collect()) {
            for (&j, (_, query)) in group.iter().zip(&parts) {
                out[j] = if query.is_empty() { key.1.clone() } else { format!("{}?{query}", key.1) };
            }
        } else {
            for (n, &j) in group.iter().enumerate().skip(1) {
                out[j] = format!("{} ({})", key.1, n + 1);
            }
        }
    }
    out
}

impl Tab {
    pub fn new(id: u64, mut state: PersistedState) -> Self {
        reconcile(&mut state.url, &mut state.params);
        Self {
            id,
            synced_url: state.url.clone(),
            header_rows: parse_headers(&state.headers_text),
            state,
            name: None,
            saved_id: None,
            history_id: None,
            request_tab: RequestTab::Params,
            headers_as_text: false,
            response_tab: ResponseTab::Body,
            status: RequestStatus::Idle,
            outcome: Outcome::Empty,
            copied_flash: None,
            save_error: None,
            response_search_open: false,
            response_search_query: String::new(),
            response_search_index: 0,
            response_search_focus: false,
            response_search_scroll: 0,
            response_search_cache: Default::default(),
            markup_cache: None,
            pane: Pane::Both,
            request_height: None,
            request_shown_height: 0.0,
            editor_rows: 8,
            editor_fill: false,
            opened_from: None,
            sent_values: Default::default(),
        }
    }

    /// A blank request, carrying over the session-wide settings from `session`.
    pub fn blank(id: u64, session: &PersistedState) -> Self {
        let state = PersistedState { url: String::new(), ..Default::default() }.with_session_from(session);
        Self::new(id, state)
    }

    pub fn from_entry(id: u64, entry: &HistoryEntry, session: &PersistedState) -> Self {
        let mut tab = Self::new(id, entry.to_persisted_state().with_session_from(session));
        tab.name = entry.name.clone();
        tab.saved_id = entry.name.as_ref().map(|_| entry.id);
        tab.history_id = Some(entry.id);
        tab.sent_values = entry.sent_values.clone();
        tab.opened_from = Some(HistoryMeta {
            created_at: entry.created_at.clone(),
            status: entry.status,
            elapsed_ms: entry.elapsed_ms,
            source: entry.source,
            error: entry.error.clone(),
        });
        tab
    }

    pub fn from_saved(id: u64, saved: SavedTab) -> Self {
        let mut tab = Self::new(id, saved.state);
        tab.name = saved.name;
        tab.saved_id = saved.saved_id;
        tab.history_id = saved.history_id;
        tab
    }

    /// A request an agent sent (from MCP or the command line) that is open only to be looked at: not saved, not
    /// sent again from here. It is in the history, so it is not brought back at the next start.
    pub fn is_agent_preview(&self) -> bool {
        self.saved_id.is_none()
            && self.opened_from.as_ref().is_some_and(|m| m.source != crate::store::history::Source::Gui)
    }

    pub fn to_saved(&self) -> SavedTab {
        SavedTab {
            state: self.state.redacted(),
            name: self.name.clone(),
            saved_id: self.saved_id,
            history_id: self.history_id,
        }
    }

    /// The saved name, or something recognisable made from the URL: its last two path segments, so
    /// `/api/v1/users/42` is `users/42` and not just `42`.
    pub fn title(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        title_from_url(&self.state.url)
    }

    /// Untouched blank tab: opening something reuses it instead of adding a tab.
    pub fn is_pristine(&self) -> bool {
        self.state.url.trim().is_empty()
            && self.saved_id.is_none()
            && self.history_id.is_none()
            && matches!(self.outcome, Outcome::Empty)
            && !self.is_loading()
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.status, RequestStatus::InFlight { .. })
    }

    /// Rebuilds the Params table after the URL was edited (typed, imported, or
    /// given a scheme on send).
    pub fn sync_params_from_url(&mut self) {
        if self.state.url != self.synced_url {
            self.state.params = params_from_url(&self.state.url, &self.state.params);
            self.synced_url = self.state.url.clone();
        }
    }

    /// Replaces the request with an imported one, keeping this tab's session
    /// settings (variables, options).
    pub fn apply_parsed_request(&mut self, parsed: ParsedRequest) {
        self.state = parsed.into_state().with_session_from(&self.state);
        self.header_rows = parse_headers(&self.state.headers_text);
        self.sync_params_from_url();
        self.outcome = Outcome::Empty;
    }

    /// `agent_variables` are the ones agents set; they count as defined for this request only and
    /// are never added to the tab's own variables (which are saved with the window state).
    pub fn send(&mut self, bearer_token: &str, agent_variables: &[Variable]) {
        // The request folds away when it is sent, so the answer is what you see. A split the user dragged
        // to is theirs and stays.
        self.pane = if self.request_height.is_some() { Pane::Both } else { Pane::ResponseExpanded };
        self.opened_from = None;
        let own = self.state.variables.len();
        for var in agent_variables {
            if !self.state.variables.iter().any(|v| v.name.trim() == var.name) {
                self.state.variables.push(var.clone());
            }
        }
        // The scheme is filled in in the field itself, where the user sees it.
        let prepared = prepare_to_send(&mut self.state, bearer_token);
        self.state.variables.truncate(own);
        let req = match prepared {
            Ok(req) => req,
            Err(message) => {
                self.outcome = Outcome::Failed(message);
                return;
            }
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.status = RequestStatus::InFlight {
            rx,
            sent: Box::new(self.state.clone()),
        };
        self.outcome = Outcome::Empty;
        self.save_error = None;
        send_request(req, tx);
    }

    /// Cancelling only stops the UI waiting: `reqwest::blocking` can't be
    /// interrupted mid-flight, so the background thread's result is dropped.
    pub fn cancel(&mut self) {
        self.status = RequestStatus::Idle;
    }

    /// Checks for a finished send. Returns what was sent and how it went, once,
    /// when it completes; the caller records it in the history.
    pub fn poll(&mut self) -> Option<Finished> {
        let RequestStatus::InFlight { rx, .. } = &self.status else {
            return None;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("The request thread ended unexpectedly.".to_string()),
        };
        let RequestStatus::InFlight { sent, .. } = std::mem::replace(&mut self.status, RequestStatus::Idle) else {
            return None;
        };
        let finished = Finished {
            status: result.as_ref().ok().map(|d| d.status),
            elapsed_ms: result.as_ref().ok().map(|d| d.elapsed_ms),
            sent,
        };
        self.outcome = match result {
            Ok(data) => {
                self.response_tab = ResponseTab::Body;
                Outcome::Response(Box::new(data))
            }
            Err(err) => Outcome::Failed(err),
        };
        Some(finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(url: &str) -> Tab {
        Tab::new(1, PersistedState { url: url.into(), ..Default::default() })
    }

    #[test]
    fn titles_prefer_the_name_then_the_last_two_path_segments() {
        assert_eq!(tab("").title(), "New request");
        assert_eq!(tab("https://h.com/api/v2/fixtures/42?x=1").title(), "fixtures/42");
        assert_eq!(tab("https://h.com/").title(), "h.com");
        assert_eq!(tab("localhost:3000").title(), "localhost:3000");
        let mut named = tab("https://h.com/a");
        named.name = Some("Fixtures".into());
        assert_eq!(named.title(), "Fixtures");
    }

    #[test]
    fn tabs_that_would_read_the_same_are_told_apart() {
        let make = |method: &str, url: &str| {
            let mut t = Tab::new(1, PersistedState { url: url.into(), method: method.into(), ..Default::default() });
            t.name = None;
            t
        };
        // different hosts
        let titles = display_titles(&[make("GET", "http://a:1/api/items"), make("GET", "http://b:2/api/items")]);
        assert_eq!(titles, ["api/items \u{b7} a:1", "api/items \u{b7} b:2"]);
        // same host, different query
        let titles = display_titles(&[make("GET", "http://a/items?x=1"), make("GET", "http://a/items?x=2")]);
        assert_eq!(titles, ["items?x=1", "items?x=2"]);
        // identical: a count
        let titles = display_titles(&[make("GET", "http://a/items"), make("GET", "http://a/items"), make("GET", "http://a/items")]);
        assert_eq!(titles, ["items", "items (2)", "items (3)"]);
        // a different method already tells them apart
        assert_eq!(display_titles(&[make("GET", "http://a/items"), make("POST", "http://a/items")]), ["items", "items"]);
    }

    #[test]
    fn an_agent_request_open_only_to_look_at_is_not_brought_back() {
        use crate::store::history::Source;
        let meta = |source| HistoryMeta { created_at: "2026-10-09T06:11:03Z".into(), status: Some(200), elapsed_ms: Some(5), source, error: None };
        let mut tab = Tab::new(1, PersistedState { url: "http://h/x".into(), ..Default::default() });
        assert!(!tab.is_agent_preview(), "a tab you typed in is yours");
        tab.opened_from = Some(meta(Source::Mcp));
        assert!(tab.is_agent_preview());
        tab.opened_from = Some(meta(Source::Gui));
        assert!(!tab.is_agent_preview(), "a request you sent yourself is kept");
        tab.opened_from = Some(meta(Source::Cli));
        tab.saved_id = Some(7);
        assert!(!tab.is_agent_preview(), "once saved it is kept");
    }

    #[test]
    fn sending_folds_the_request_away_and_forgets_the_history_note() {
        let mut tab = Tab::new(1, PersistedState { url: "http://127.0.0.1:1/x".into(), ..Default::default() });
        tab.pane = Pane::RequestExpanded;
        tab.opened_from = Some(HistoryMeta { created_at: "2026-10-09T06:11:03Z".into(), status: Some(200), elapsed_ms: Some(5), source: crate::store::history::Source::Mcp, error: None });
        tab.send("", &[]);
        assert_eq!(tab.pane, Pane::ResponseExpanded, "the response is what you see after a send");
        assert!(tab.opened_from.is_none());
        tab.cancel();
        // a split the user dragged to is kept
        tab.pane = Pane::Both;
        tab.request_height = Some(300.0);
        tab.send("", &[]);
        assert_eq!(tab.pane, Pane::Both);
        tab.cancel();
    }

    #[test]
    fn a_tab_opened_from_history_remembers_how_that_request_went() {
        let entry = crate::store::history::HistoryEntry {
            id: 4,
            name: None,
            created_at: "2026-10-09T06:11:03Z".into(),
            method: "GET".into(),
            url: "http://h/x".into(),
            headers_text: String::new(),
            body_mode: crate::domain::model::BodyMode::None,
            json_body: String::new(),
            urlencoded_body: String::new(),
            raw_body: String::new(),
            params: vec![],
            multipart_fields: vec![],
            status: Some(404),
            elapsed_ms: Some(12),
            source: crate::store::history::Source::Cli,
            options: Default::default(),
            sent_values: Default::default(),
            group: None,
            error: None,
        };
        let tab = Tab::from_entry(1, &entry, &PersistedState::default());
        let meta = tab.opened_from.expect("opened from history");
        assert_eq!((meta.status, meta.elapsed_ms, meta.source), (Some(404), Some(12), crate::store::history::Source::Cli));
    }

    #[test]
    fn agent_variables_count_for_a_send_but_are_never_added_to_the_tabs_own_variables() {
        let agent = vec![Variable { name: "base".into(), value: "http://127.0.0.1:1".into(), secret: false, remember: false }];

        let mut without = tab("{{base}}/x");
        without.send("", &[]);
        assert!(matches!(&without.outcome, Outcome::Failed(m) if m.contains("Undefined variable")), "refused without the variable");

        let mut with = tab("{{base}}/x");
        with.send("", &agent);
        assert!(with.is_loading(), "sent once an agent variable defines it");
        assert!(with.state.variables.is_empty(), "the tab's own variables are untouched");
        let RequestStatus::InFlight { sent, .. } = &with.status else { panic!("not in flight") };
        assert!(sent.variables.is_empty(), "and so is the record of what was sent");

        // the user's own variable of the same name wins
        let mut own = tab("{{base}}/x");
        own.state.variables = vec![Variable { name: "base".into(), value: "http://127.0.0.1:2".into(), secret: false, remember: false }];
        own.send("", &agent);
        assert_eq!(own.state.variables.len(), 1);
        assert_eq!(own.state.variables[0].value, "http://127.0.0.1:2");
    }

    #[test]
    fn only_an_untouched_blank_tab_is_pristine() {
        assert!(tab("").is_pristine());
        assert!(!tab("https://h").is_pristine());
        let mut from_history = tab("");
        from_history.history_id = Some(3);
        assert!(!from_history.is_pristine());
    }

    #[test]
    fn a_saved_tab_round_trips_without_its_secrets() {
        let mut t = tab("https://h/x?api_key=SECRET&page=2");
        t.name = Some("N".into());
        t.saved_id = Some(7);
        let saved = t.to_saved();
        assert_eq!(saved.state.url, "https://h/x?api_key=&page=2");
        let back = Tab::from_saved(2, saved);
        assert_eq!((back.name.as_deref(), back.saved_id), (Some("N"), Some(7)));
        assert_eq!(back.state.params.len(), 2);
    }

    #[test]
    fn a_tab_without_a_name_is_titled_by_the_last_two_path_segments() {
        for (url, title) in [
            ("https://h/api/v1/users/42?x=1", "users/42"),
            ("https://h/orders", "orders"),
            ("https://h/orders/", "orders"),
            ("{{base}}/items/{{id}}", "items/{{id}}"),
            ("{{base}}/x", "x"),
            ("https://api.example.com", "api.example.com"),
            ("localhost:3000/a/b/c", "b/c"),
            ("localhost:3000/a", "a"),
            ("https://h/search?q=a/b", "search"),
            ("  ", "New request"),
        ] {
            assert_eq!(super::title_from_url(url), title, "{url}");
        }
    }
}