//! One open request: its form, its response, and its in-flight send. The app
//! holds several of these as tabs.

use crate::history::HistoryEntry;
use crate::http::send_request;
use crate::model::{Outcome, ParsedRequest, PersistedState, RequestTab, ResponseTab, SendResult};
use crate::query::{params_from_url, reconcile};
use crate::request::{parse_headers, prepare_to_send};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

/// Which copy button was clicked last, and when — so only that button shows
/// the "copied" check mark.
pub(super) type CopiedFlash = Option<(Instant, &'static str)>;

/// The request lifecycle as one value, so "loading", "who to poll" and "what
/// was sent" can't drift out of sync with each other.
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
        tab
    }

    pub fn from_saved(id: u64, saved: SavedTab) -> Self {
        let mut tab = Self::new(id, saved.state);
        tab.name = saved.name;
        tab.saved_id = saved.saved_id;
        tab.history_id = saved.history_id;
        tab
    }

    pub fn to_saved(&self) -> SavedTab {
        SavedTab {
            state: self.state.redacted(),
            name: self.name.clone(),
            saved_id: self.saved_id,
            history_id: self.history_id,
        }
    }

    /// The saved name, or something recognisable made from the URL.
    pub fn title(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        let url = self.state.url.trim();
        if url.is_empty() {
            return "New request".to_string();
        }
        let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
        let path = without_scheme.split(['?', '#']).next().unwrap_or(without_scheme);
        let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path);
        if last.is_empty() {
            path.to_string()
        } else {
            last.to_string()
        }
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

    pub fn send(&mut self, bearer_token: &str) {
        // The scheme is filled in in the field itself, where the user sees it.
        let req = match prepare_to_send(&mut self.state, bearer_token) {
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
                Outcome::Response(data)
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
    fn titles_prefer_the_name_then_the_last_path_segment() {
        assert_eq!(tab("").title(), "New request");
        assert_eq!(tab("https://h.com/api/v2/fixtures/42?x=1").title(), "42");
        assert_eq!(tab("https://h.com/").title(), "h.com");
        assert_eq!(tab("localhost:3000").title(), "localhost:3000");
        let mut named = tab("https://h.com/a");
        named.name = Some("Fixtures".into());
        assert_eq!(named.title(), "Fixtures");
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
}
