//! What the window has configured and an agent inherits: variables, options and remembered secrets.

use crate::history::{app_data_dir, History};
use crate::model::{PersistedState, Variable};
use crate::secrets::{OsStore, SecretStore, SecretSync};
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// What the window has configured and an agent inherits: variables, options,
/// and the remembered secrets from the credential store.
pub struct Session {
    pub state: PersistedState,
    pub bearer: String,
    /// Non-fatal problems, e.g. the credential store couldn't be read.
    pub problems: Vec<String>,
    /// Names of the variables an agent set (`state.variables` also holds the window's own).
    pub agent_variables: std::collections::BTreeSet<String>,
}

/// The credential store key for the value of a secret variable an agent set.
pub fn agent_secret_key(name: &str) -> String {
    format!("agentvar:{name}")
}

impl Session {
    /// Reads the window's saved state (variables and options) and fills in
    /// remembered secrets. The window writes its state when it closes and
    /// about every 30 seconds, so a variable edited a moment ago may lag.
    pub fn load() -> Self {
        let history = History::open().ok();
        Self::load_with(&app_data_dir().join("app.ron"), &OsStore::new(), history.as_ref())
    }

    /// The window's state plus the variables agents set (kept in `history`). A variable the user
    /// defined in the window wins over an agent's of the same name.
    pub fn load_with(state_file: &Path, store: &dyn SecretStore, history: Option<&History>) -> Self {
        let mut state = read_window_state(state_file).unwrap_or_default();
        let mut bearer = String::new();
        let mut problems = SecretSync::default().restore(store, &mut state, &mut bearer);
        let mut agent_variables = std::collections::BTreeSet::new();
        if let Some(history) = history {
            match history.list_agent_variables() {
                Ok(list) => {
                    for var in list {
                        if state.variables.iter().any(|v| v.name.trim() == var.name) {
                            continue;
                        }
                        let value = if var.secret {
                            match store.get(&agent_secret_key(&var.name)) {
                                Ok(found) => found.unwrap_or_default(),
                                Err(e) => {
                                    problems.push(format!("Couldn't read the secret value of {}: {e}", var.name));
                                    String::new()
                                }
                            }
                        } else {
                            var.value
                        };
                        state.variables.push(Variable { name: var.name.clone(), value, secret: var.secret, remember: var.secret });
                        agent_variables.insert(var.name);
                    }
                }
                Err(e) => problems.push(format!("Couldn't read the variables agents set: {e}")),
            }
        }
        Self { state, bearer, problems, agent_variables }
    }

    /// Variables the user defined in the window (not the ones agents set).
    pub fn window_variables(&self) -> Vec<&Variable> {
        self.state.variables.iter().filter(|v| !self.agent_variables.contains(v.name.trim())).collect()
    }

    /// Variables as an agent may see them: names always, values only when not secret.
    pub fn variables(&self) -> Vec<VariableInfo> {
        self.state
            .variables
            .iter()
            .filter(|v| !v.name.trim().is_empty())
            .map(|v| VariableInfo {
                name: v.name.trim().to_string(),
                secret: v.is_secret(),
                has_value: !v.value.is_empty(),
                remembered: v.remember && v.is_secret(),
                value: (!v.is_secret()).then(|| v.value.clone()),
                source: if self.agent_variables.contains(v.name.trim()) { "agent" } else { "window" }.to_string(),
            })
            .collect()
    }
}

/// eframe's state file is a RON map of key -> RON string; the request form
/// (with the variables) is under eframe's `APP_KEY`.
fn read_window_state(path: &Path) -> Option<PersistedState> {
    let text = std::fs::read_to_string(path).ok()?;
    let map: BTreeMap<String, String> = ron::from_str(&text).ok()?;
    ron::from_str(map.get(eframe::APP_KEY)?).ok()
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct VariableInfo {
    pub name: String,
    /// Secret values are masked in the window and never returned here.
    pub secret: bool,
    pub has_value: bool,
    /// Kept in the operating system's credential store between runs.
    pub remembered: bool,
    /// Only for non-secret variables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// `window` for a variable the user defined, `agent` for one set from the CLI or MCP.
    pub source: String,
}

