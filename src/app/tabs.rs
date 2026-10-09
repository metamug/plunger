//! Opening, closing and switching tabs.

use super::*;

impl ApiTesterApp {
    // ---- tabs -------------------------------------------------------------

    pub(super) fn next_id(&mut self) -> u64 {
        self.next_tab_id += 1;
        self.next_tab_id
    }

    /// Switches tabs. Variables and options are session-wide, so the tab being
    /// shown takes them over from the one being left.
    pub(super) fn activate(&mut self, index: usize) {
        if index == self.active || index >= self.tabs.len() {
            return;
        }
        let session = self.tabs[self.active].state.clone();
        self.active = index;
        let tab = &mut self.tabs[index];
        tab.state = std::mem::take(&mut tab.state).with_session_from(&session);
    }

    pub(super) fn push_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
        self.activate(self.tabs.len() - 1);
    }

    pub(super) fn new_tab(&mut self) {
        let id = self.next_id();
        let tab = Tab::blank(id, &self.tab().state);
        self.push_tab(tab);
    }

    pub(super) fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            // Never leave zero tabs: closing the last one leaves a blank one.
            let id = self.next_id();
            self.tabs[0] = Tab::blank(id, &self.tabs[0].state);
            return;
        }
        let session = self.tabs[self.active].state.clone();
        self.tabs.remove(index);
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        let tab = &mut self.tabs[self.active];
        tab.state = std::mem::take(&mut tab.state).with_session_from(&session);
    }

    /// Replaces the current tab if it is an untouched blank one, otherwise opens a new tab.
    pub(super) fn open_in_tab(&mut self, mut tab: Tab) {
        if self.tab().is_pristine() {
            let session = self.tab().state.clone();
            tab.state = std::mem::take(&mut tab.state).with_session_from(&session);
            self.tabs[self.active] = tab;
        } else {
            self.push_tab(tab);
        }
    }

    /// Opens a sidebar entry, or switches to the tab already showing it.
    pub(super) fn open_entry(&mut self, entry: &HistoryEntry) {
        let existing = self.tabs.iter().position(|t| {
            t.history_id == Some(entry.id) || (entry.name.is_some() && t.saved_id == Some(entry.id))
        });
        match existing {
            Some(i) => self.activate(i),
            None => {
                let id = self.next_id();
                let tab = Tab::from_entry(id, entry, &self.tab().state);
                self.open_in_tab(tab);
            }
        }
    }

    pub(super) fn open_parsed(&mut self, parsed: ParsedRequest) {
        let id = self.next_id();
        let mut tab = Tab::blank(id, &self.tab().state);
        tab.apply_parsed_request(parsed);
        self.open_in_tab(tab);
        self.import = ImportDialog::Closed;
    }
}
