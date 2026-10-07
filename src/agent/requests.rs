//! Saved requests, curl import and the history, as agents use them.

use super::*;

/// The form state for a curl command, exactly as the window's import builds it.
pub(super) fn state_from_curl(curl: &str) -> Result<PersistedState, String> {
    Ok(parse_curl(curl)?.into_state())
}

/// A curl command parsed into a Plunger request.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ImportedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<NameValue>,
    /// none, json, form-data or raw.
    pub body_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// multipart -F fields: "name=value" or "name=@file".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form_fields: Vec<String>,
    /// {{variables}} the request uses.
    pub variables_used: Vec<String>,
    /// Set when saved under `save_as`; it now shows in Plunger's Saved list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_id: Option<i64>,
}

/// A request in the shape an agent reads: what it sends, and which `{{variables}}` it needs.
pub(super) fn imported_from_state(state: &PersistedState, saved_id: Option<i64>) -> ImportedRequest {
    let body = match state.body_mode {
        BodyMode::Json => Some(state.json_body.clone()),
        BodyMode::Raw => Some(state.raw_body.clone()),
        BodyMode::UrlEncoded => Some(state.urlencoded_body.clone()),
        _ => None,
    };
    ImportedRequest {
        method: state.method.clone(),
        url: state.url.clone(),
        headers: parse_headers(&state.headers_text).into_iter().map(|(name, value)| NameValue { name, value }).collect(),
        body_type: engine::body_type_name(state.body_mode).to_string(),
        body,
        form_fields: state
            .multipart_fields
            .iter()
            .map(|f| match f.kind {
                FieldKind::Text => format!("{}={}", f.key, f.value),
                FieldKind::File => format!("{}=@{}", f.key, f.value),
            })
            .collect(),
        variables_used: engine::variables_used(state),
        saved_id,
    }
}

/// Parses a curl command (without sending it), optionally saving it by name.
pub fn import_curl(curl: &str, save_as: Option<&str>, source: Source) -> Result<ImportedRequest, String> {
    let state = state_from_curl(curl)?;
    let saved_id = match save_as.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => Some(
            open_history()?
                .save_new_from(&state, name, source)
                .map_err(|e| format!("Couldn't save the request: {e}"))?,
        ),
        None => None,
    };
    Ok(imported_from_state(&state, saved_id))
}

/// Saves the request described by `params` (nothing is sent) under `name`, keeping its
/// `{{placeholders}}`. An existing name is refused unless `overwrite` is set, which replaces it.
pub fn save_request(name: &str, overwrite: bool, params: &SendParams, source: Source) -> Result<ImportedRequest, String> {
    let session = Session::load();
    let history = open_history()?;
    let state = params.to_state(&session, &history)?;
    let id = save_request_in(&history, name, overwrite, &state, source)?;
    Ok(imported_from_state(&state, Some(id)))
}

pub(super) fn save_request_in(history: &History, name: &str, overwrite: bool, state: &PersistedState, source: Source) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the request a name.".into());
    }
    let existing = history.list_saved().map_err(|e| e.to_string())?.into_iter().find(|e| e.name.as_deref() == Some(name));
    match existing {
        Some(entry) if overwrite => {
            history.update_request(entry.id, state).map_err(|e| e.to_string())?;
            Ok(entry.id)
        }
        Some(_) => Err(format!("A saved request named \"{name}\" already exists. Pass overwrite: true to replace it, or choose another name.")),
        None => history.save_new_from(state, name, source).map_err(|e| format!("Couldn't save the request: {e}")),
    }
}

/// One saved request in full: method, URL, headers, body and the variables it needs.
pub fn show_saved_request(name: &str) -> Result<ImportedRequest, String> {
    let history = open_history()?;
    let entry = engine::find_saved(&history, name)?;
    Ok(imported_from_state(&entry.to_persisted_state(), Some(entry.id)))
}

/// Removes a saved request (its history entries stay) and returns what was removed.
pub fn delete_saved_request(name: &str) -> Result<ImportedRequest, String> {
    let history = open_history()?;
    delete_saved_in(&history, name)
}

pub(super) fn delete_saved_in(history: &History, name: &str) -> Result<ImportedRequest, String> {
    let entry = engine::find_saved(history, name)?;
    history.set_name(entry.id, None).map_err(|e| e.to_string())?;
    Ok(imported_from_state(&entry.to_persisted_state(), Some(entry.id)))
}

pub fn list_saved_requests() -> Result<Vec<StoredRequestInfo>, String> {
    let saved = open_history()?.list_saved().map_err(|e| e.to_string())?;
    Ok(saved.iter().map(StoredRequestInfo::from).collect())
}

pub fn get_history(limit: Option<i64>, search: Option<&str>) -> Result<Vec<HistoryItem>, String> {
    let limit = limit.unwrap_or(HISTORY_DEFAULT).clamp(1, HISTORY_MAX);
    let rows = open_history()?.search_recent(search.unwrap_or(""), limit).map_err(|e| e.to_string())?;
    Ok(rows.iter().map(HistoryItem::from).collect())
}

/// The curl command for a saved request (by name) or a history row (by id).
/// Placeholders stay placeholders, so no secret is written out.
pub fn export_curl(saved_request: Option<&str>, history_id: Option<i64>) -> Result<String, String> {
    let history = open_history()?;
    let entry = match (saved_request, history_id) {
        (Some(name), None) => engine::find_saved(&history, name)?,
        (None, Some(id)) => history
            .get(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("No request #{id} in the history or saved requests."))?,
        _ => return Err("Give exactly one of `saved_request` or `history_id`.".into()),
    };
    Ok(to_curl(&entry.to_persisted_state()))
}

