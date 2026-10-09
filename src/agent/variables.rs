//! Variables agents set, kept between runs and shared with the window.

use super::*;

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct VariablesResult {
    pub variables: Vec<VariableInfo>,
    /// Always available: {{$uuid}}, {{$timestamp}}, {{$randomInt}}, and {{$env:NAME}} (an environment variable, read when the request is sent).
    pub built_in: Vec<String>,
    /// True when a Bearer token is saved in Plunger (send with use_saved_bearer). Its value is never shown.
    pub saved_bearer_available: bool,
    /// Problems reading remembered secrets, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
}

pub fn list_variables() -> VariablesResult {
    let session = Session::load();
    VariablesResult {
        variables: session.variables(),
        built_in: vec!["$uuid".into(), "$timestamp".into(), "$randomInt".into(), "$env:NAME".into()],
        saved_bearer_available: !session.bearer.is_empty(),
        problems: session.problems,
    }
}

pub(super) const MAX_VARIABLES: usize = 200;
pub(super) const MAX_VARIABLE_BYTES: usize = 64 * 1024;
const MAX_NAME_LEN: usize = 64;

pub(crate) fn valid_variable_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && !name.starts_with('$')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(name)
    } else {
        Err(format!(
            "`{name}` isn't a valid variable name: use letters, digits, _ - and . (at most {MAX_NAME_LEN} characters, not starting with $)."
        ))
    }
}

/// Sets a variable that later requests can use as `{{name}}`. It is kept between runs and shared
/// with the window, the command line and the MCP server. A secret (or a name that looks like a
/// credential) is kept in the system credential store, never in a file, and is masked in results.
pub fn set_variable(name: &str, value: &str, secret: Option<bool>, source: Source) -> Result<VariableInfo, String> {
    let session = Session::load();
    let history = open_history()?;
    let window: Vec<String> = session.window_variables().iter().map(|v| v.name.trim().to_string()).collect();
    set_variable_in(&history, &crate::secrets::OsStore::new(), &window, name, value, secret, source)
}

pub(super) fn set_variable_in(
    history: &History,
    store: &dyn crate::secrets::SecretStore,
    window_names: &[String],
    name: &str,
    value: &str,
    secret: Option<bool>,
    source: Source,
) -> Result<VariableInfo, String> {
    let name = valid_variable_name(name)?;
    if value.len() > MAX_VARIABLE_BYTES {
        return Err(format!("The value is too long ({} bytes; the limit is {MAX_VARIABLE_BYTES}).", value.len()));
    }
    if window_names.iter().any(|n| n == name) {
        return Err(format!(
            "`{name}` is defined by the user in the Plunger window, and an agent cannot change it. Pick another name, or pass a value for one request in `variables`."
        ));
    }
    let existing = history.list_agent_variables().map_err(|e| e.to_string())?;
    if existing.len() >= MAX_VARIABLES && !existing.iter().any(|v| v.name == name) {
        return Err(format!("There are already {MAX_VARIABLES} variables set by agents; delete some first."));
    }
    let secret = secret.unwrap_or(false) || is_sensitive_header(name);
    if secret {
        store
            .set(&engine::agent_secret_key(name), value)
            .map_err(|e| format!("Couldn't keep the secret in the system credential store: {e}"))?;
        history.set_agent_variable(name, "", true, source).map_err(|e| e.to_string())?;
    } else {
        // A plain value replacing a secret of the same name must not leave the old secret behind.
        let _ = store.delete(&engine::agent_secret_key(name));
        history.set_agent_variable(name, value, false, source).map_err(|e| e.to_string())?;
    }
    Ok(VariableInfo {
        name: name.to_string(),
        secret,
        has_value: !value.is_empty(),
        remembered: secret,
        value: (!secret).then(|| value.to_string()),
        source: "agent".to_string(),
    })
}

/// Removes a variable an agent set. The user's own variables can only be removed in the window.
pub fn delete_variable(name: &str) -> Result<(), String> {
    let session = Session::load();
    let history = open_history()?;
    let window: Vec<String> = session.window_variables().iter().map(|v| v.name.trim().to_string()).collect();
    delete_variable_in(&history, &crate::secrets::OsStore::new(), &window, name)
}

pub(super) fn delete_variable_in(
    history: &History,
    store: &dyn crate::secrets::SecretStore,
    window_names: &[String],
    name: &str,
) -> Result<(), String> {
    let name = name.trim();
    if window_names.iter().any(|n| n == name) {
        return Err(format!("`{name}` belongs to the user (it is defined in the Plunger window); delete it there."));
    }
    if !history.delete_agent_variable(name).map_err(|e| e.to_string())? {
        return Err(format!("No variable named `{name}` was set by an agent."));
    }
    let _ = store.delete(&engine::agent_secret_key(name));
    Ok(())
}

/// Removes every variable agents set; returns how many.
pub fn clear_variables() -> Result<usize, String> {
    let history = open_history()?;
    let store = crate::secrets::OsStore::new();
    let names = history.clear_agent_variables().map_err(|e| e.to_string())?;
    for name in &names {
        let _ = crate::secrets::SecretStore::delete(&store, &engine::agent_secret_key(name));
    }
    Ok(names.len())
}

