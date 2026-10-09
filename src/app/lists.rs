//! The sidebar's lists (history and saved requests), the variables agents set, and keeping them in step with
//! the database the CLI and MCP server write to.

use super::*;

impl ApiTesterApp {
    // ---- saved requests and history ---------------------------------------

    pub(super) fn refresh_lists(&mut self) {
        if let Some(h) = &self.history {
            self.db_version = h.data_version();
            if let Ok(entries) = h.search_recent(&self.sidebar_filter, HISTORY_LIMIT) {
                self.history_entries = entries;
            }
            if let Ok(entries) = h.search_saved(&self.sidebar_filter) {
                self.saved_entries = entries;
            }
            if let Ok(vars) = h.list_agent_variables() {
                crate::highlight::set_agent_variable_names(vars.iter().map(|v| v.name.clone()).collect());
                self.agent_variables = vars;
            }
        }
    }

    /// The agent variables as values a request can use; a secret's value comes from the credential store.
    pub(super) fn agent_variable_values(&self) -> Vec<crate::model::Variable> {
        self.agent_variables
            .iter()
            .map(|v| crate::model::Variable {
                name: v.name.clone(),
                value: if v.secret {
                    self.secrets.get(&crate::engine::agent_secret_key(&v.name)).ok().flatten().unwrap_or_default()
                } else {
                    v.value.clone()
                },
                secret: v.secret,
                remember: false,
            })
            .collect()
    }

    /// Removes a variable an agent set (the Variables tab's trash icon).
    pub(super) fn delete_agent_variable(&mut self, name: &str) {
        if let Some(h) = &self.history {
            let _ = h.delete_agent_variable(name);
            let _ = self.secrets.delete(&crate::engine::agent_secret_key(name));
        }
        self.refresh_lists();
        self.notify(format!("Removed {name}"));
    }

    /// Ctrl+S: writes the tab back to its saved request, or saves it as a new
    /// one and lets the user name it right away in the sidebar.
    pub(super) fn save_active(&mut self) {
        let Some(h) = &self.history else {
            self.notify("Can't save: the local database couldn't be opened.");
            return;
        };
        let tab = &self.tabs[self.active];
        let name = tab.title();
        if let Some(id) = tab.saved_id {
            if h.update_request(id, &tab.state).unwrap_or(false) {
                self.refresh_lists();
                self.notify(format!("Saved \u{201c}{name}\u{201d}"));
                return;
            }
        }
        match h.save_new(&tab.state, &name) {
            Ok(id) => {
                let tab = &mut self.tabs[self.active];
                tab.saved_id = Some(id);
                tab.name = Some(name.clone());
                self.refresh_lists();
                self.saved_open = true;
                self.renaming = Some(Rename { id, list: "saved", text: name, focus: true });
                self.notify("Saved. Type a name for it and press Enter.");
            }
            Err(e) => self.notify(format!("Could not save: {e}")),
        }
    }

    pub(super) fn start_rename(&mut self, entry: &HistoryEntry, list: &'static str) {
        let text = entry.name.clone().unwrap_or_default();
        self.renaming = Some(Rename { id: entry.id, list, text, focus: true });
    }

    /// Naming a row saves it; the tabs showing it take the new name.
    pub(super) fn commit_rename(&mut self, id: i64, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(h) = &self.history {
            if let Err(error) = h.set_name(id, Some(name)) {
                self.notify(format!("Could not save the name: {error}"));
                return;
            }
        }
        for tab in &mut self.tabs {
            if tab.saved_id == Some(id) || tab.history_id == Some(id) {
                tab.saved_id = Some(id);
                tab.name = Some(name.to_string());
            }
        }
        self.refresh_lists();
        self.notify(format!("Saved \u{201c}{name}\u{201d}"));
    }

    pub(super) fn unsave(&mut self, id: i64) {
        if let Some(h) = &self.history {
            let _ = h.set_name(id, None);
        }
        for tab in &mut self.tabs {
            if tab.saved_id == Some(id) {
                tab.saved_id = None;
                tab.name = None;
            }
        }
        self.refresh_lists();
    }

    pub(super) fn clear_history(&mut self) {
        if let Some(h) = &self.history {
            let _ = h.clear();
        }
        self.refresh_lists();
    }

    /// Picks up requests an agent sent through the CLI or MCP server. One
    /// cheap PRAGMA every couple of seconds; the lists are only re-read when
    /// another process actually changed the database.
    pub(super) fn poll_database(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(DB_POLL);
        if self.last_db_poll.elapsed() < DB_POLL {
            return;
        }
        self.last_db_poll = Instant::now();
        let current = self.history.as_ref().and_then(History::data_version);
        if current.is_some() && current != self.db_version {
            self.refresh_lists();
        }
    }
}
