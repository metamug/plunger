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
    query_history(&HistoryQuery { limit, search: search.map(str::to_string), ..Default::default() })
}

/// What to look for in the history. Every field narrows the result.
#[derive(Default, Debug, Clone)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
    /// Text in the URL, method, name or status.
    pub search: Option<String>,
    /// `401`, a class (`4xx`, `5xx`), `ok` (2xx), `fail` (4xx, 5xx or no response) or `error` (no response).
    pub status: Option<String>,
    /// Only requests that took at least this many milliseconds.
    pub min_ms: Option<i64>,
    /// `gui`, `cli` or `mcp`.
    pub source: Option<String>,
    /// Only sends of this saved request (same method and URL as it).
    pub saved_request: Option<String>,
}

/// Whether `status` fits the filter `spec` (see `HistoryQuery::status`).
fn status_matches(spec: &str, status: Option<i64>) -> bool {
    let spec = spec.trim().to_ascii_lowercase();
    match spec.as_str() {
        "error" | "none" | "no_response" => status.is_none(),
        "ok" => status.is_some_and(|s| (200..300).contains(&s)),
        "fail" | "failed" => status.is_none_or(|s| s >= 400),
        class if class.len() == 3 && class.ends_with("xx") => {
            class[..1].parse::<i64>().ok().is_some_and(|d| status.is_some_and(|s| s / 100 == d))
        }
        exact => exact.parse::<i64>().ok().is_some_and(|want| status == Some(want)),
    }
}

/// Whether `spec` is a status filter `status_matches` understands.
fn valid_status_filter(spec: &str) -> bool {
    let spec = spec.trim().to_ascii_lowercase();
    matches!(spec.as_str(), "error" | "none" | "no_response" | "ok" | "fail" | "failed")
        || (spec.len() == 3 && spec.ends_with("xx") && spec[..1].parse::<u8>().is_ok_and(|d| (1..=5).contains(&d)))
        || spec.parse::<u16>().is_ok_and(|code| (100..600).contains(&code))
}

/// The history, newest first, narrowed by `query`. The whole table is searched, not just the newest rows.
pub fn query_history(query: &HistoryQuery) -> Result<Vec<HistoryItem>, String> {
    let limit = query.limit.unwrap_or(HISTORY_DEFAULT).clamp(1, HISTORY_MAX) as usize;
    if let Some(spec) = query.status.as_deref().filter(|s| !valid_status_filter(s)) {
        return Err(format!("`{spec}` is not a status filter: use a code like 401, a class like 4xx or 5xx, or ok, fail or error."));
    }
    let history = open_history()?;
    let same_request = match query.saved_request.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => {
            let saved = engine::find_saved(&history, name)?;
            Some((saved.method.to_ascii_uppercase(), saved.url))
        }
        None => None,
    };
    let narrowed = query.status.is_some() || query.min_ms.is_some() || query.source.is_some() || same_request.is_some();
    // Without a filter the database does the limiting; with one, read more rows and filter here.
    let fetch = if narrowed { 5_000 } else { limit as i64 };
    let rows = history.search_recent(query.search.as_deref().unwrap_or(""), fetch).map_err(|e| e.to_string())?;
    let source = query.source.as_deref().map(|s| s.trim().to_ascii_lowercase());
    Ok(rows
        .iter()
        .filter(|e| query.status.as_deref().is_none_or(|spec| status_matches(spec, e.status)))
        .filter(|e| query.min_ms.is_none_or(|min| e.elapsed_ms.is_some_and(|ms| ms >= min)))
        .filter(|e| source.as_deref().is_none_or(|s| e.source.as_str() == s))
        .filter(|e| same_request.as_ref().is_none_or(|(method, url)| e.method.eq_ignore_ascii_case(method) && &e.url == url))
        .take(limit)
        .map(HistoryItem::from)
        .collect())
}

/// One history row in full: when, how long, the status, and the request as sent (credentials blanked,
/// `{{placeholders}}` kept). The response body is not stored.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct HistoryDetail {
    pub item: HistoryItem,
    pub request: ImportedRequest,
}

pub fn show_history_entry(id: i64) -> Result<HistoryDetail, String> {
    let history = open_history()?;
    let entry = history
        .get(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No request #{id} in the history. See get_history."))?;
    Ok(HistoryDetail { item: HistoryItem::from(&entry), request: imported_from_state(&entry.to_persisted_state(), entry.name.as_ref().map(|_| entry.id)) })
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

#[cfg(test)]
mod history_filter_tests {
    use super::status_matches;

    #[test]
    fn only_real_status_filters_are_accepted() {
        for good in ["401", "404", "4xx", "5XX", "ok", "fail", "error", " 200 "] {
            assert!(super::valid_status_filter(good), "{good}");
        }
        for bad in ["banana", "", "99", "700", "6xx", "0xx", "4x", "okay"] {
            assert!(!super::valid_status_filter(bad), "{bad:?}");
        }
    }

    #[test]
    fn status_filters_take_codes_classes_and_words() {
        assert!(status_matches("401", Some(401)) && !status_matches("401", Some(402)));
        assert!(status_matches("4xx", Some(404)) && !status_matches("4xx", Some(500)) && !status_matches("4xx", None));
        assert!(status_matches("5XX", Some(503)));
        assert!(status_matches("ok", Some(204)) && !status_matches("ok", Some(301)) && !status_matches("ok", None));
        assert!(status_matches("fail", Some(500)) && status_matches("fail", None) && !status_matches("fail", Some(200)));
        assert!(status_matches("error", None) && !status_matches("error", Some(200)));
        assert!(!status_matches("banana", Some(200)));
    }
}
