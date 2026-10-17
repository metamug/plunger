use crate::domain::model::{BodyMode, FormField, KeyValue, PersistedState};
use crate::domain::redact::{redact_headers_text, redact_url};
use rmcp::schemars::{self, JsonSchema};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

mod variables;
mod workflows;

use crate::store::log::{self, Event};

pub use variables::AgentVariable;
pub use workflows::{NewWorkflowRun, WorkflowRunRow, WORKFLOW_RUNS_KEPT};


/// Overrides where all app data lives (history, window state, crash log).
/// For demos and testing: run a copy against a throwaway folder without
/// touching your real history.
pub const DATA_DIR_ENV: &str = "PLUNGER_DATA_DIR";

/// Per-user application data directory (history database, crash log).
pub fn app_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("", "", "Plunger")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Who sent a request: a person in the window, or an agent through the
/// command line or MCP. Shown in the sidebar so agent activity can be audited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    Gui,
    Cli,
    Mcp,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Gui => "gui",
            Source::Cli => "cli",
            Source::Mcp => "mcp",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "cli" => Source::Cli,
            "mcp" => Source::Mcp,
            _ => Source::Gui,
        }
    }
}

#[derive(Clone)]
pub struct HistoryEntry {
    pub id: i64,
    /// Set once the user names the request; named rows are "saved" requests.
    pub name: Option<String>,
    pub created_at: String,
    pub method: String,
    pub url: String,
    pub headers_text: String,
    pub body_mode: BodyMode,
    pub json_body: String,
    pub urlencoded_body: String,
    pub raw_body: String,
    pub params: Vec<KeyValue>,
    pub multipart_fields: Vec<FormField>,
    pub status: Option<i64>,
    pub elapsed_ms: Option<i64>,
    pub source: Source,
    pub options: SavedOptions,
    /// For a request that was sent: the value each non-secret `{{placeholder}}` had when it went out
    /// (secrets are not kept, and long values are cut).
    pub sent_values: BTreeMap<String, String>,
    /// The group a saved request is filed under, `Jira` or `Jira/Issues`.
    pub group: Option<String>,
    /// Why a sent request got no response (connection refused, DNS, timeout...), scrubbed of secret values.
    pub error: Option<String>,
}

/// Settings a saved request carries besides its form: how long to wait, whether to follow redirects,
/// whether to check the certificate, and which values to read from the response (`select`). They apply
/// when it is sent by name from an agent, the command line or a workflow, and a value given in the call
/// wins. The window keeps these session-wide, so it does not apply them.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, JsonSchema)]
#[serde(default)]
pub struct SavedOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow_redirects: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insecure_tls: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub select: Vec<String>,
}

impl SavedOptions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Puts the saved timeout, redirect and TLS settings on `state` (a call's own values go on after).
    pub fn apply_to(&self, state: &mut PersistedState) {
        if let Some(t) = self.timeout_secs {
            state.timeout_secs = t;
        }
        if let Some(f) = self.follow_redirects {
            state.follow_redirects = f;
        }
        if let Some(i) = self.insecure_tls {
            state.insecure_tls = i;
        }
    }
}

/// The values the non-secret placeholders of `state` have, each cut at a few hundred characters.
/// Secrets, environment variables and the built-ins are not recorded.
fn sent_values(state: &PersistedState) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for name in crate::domain::vars::variables_used(state) {
        if name.starts_with("$env:") {
            continue;
        }
        let Some(v) = state.variables.iter().find(|v| v.name.trim() == name) else { continue };
        if v.is_secret() {
            continue;
        }
        out.insert(name, v.value.chars().take(300).collect());
    }
    out
}

/// What a committed event did.
struct Committed {
    /// The id handed out to a new request or workflow run (0 for other events).
    id: i64,
    /// Rows the event changed.
    changed: usize,
}

fn io_error(e: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

/// A group name as it is stored: parts separated by `/` (`Jira/Issues`), each trimmed, none empty, at most
/// 80 characters in all. Empty input is no group.
pub fn normalize_group(raw: &str) -> Result<Option<String>, String> {
    let parts: Vec<&str> = raw.split('/').map(str::trim).collect();
    if parts.iter().all(|p| p.is_empty()) {
        return Ok(None);
    }
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("`{raw}` is not a group name: a group is like `Jira` or `Jira/Issues`, with no empty part."));
    }
    let joined = parts.join("/");
    if joined.chars().count() > 80 {
        return Err("A group name is at most 80 characters.".to_string());
    }
    if joined.chars().any(char::is_control) {
        return Err("A group name cannot contain control characters.".to_string());
    }
    Ok(Some(joined))
}

/// True when `group` is `wanted` or lies under it (`Jira/Issues` is under `Jira`), ignoring case.
pub fn group_matches(group: Option<&str>, wanted: &str) -> bool {
    let wanted = wanted.trim().trim_matches('/').to_ascii_lowercase();
    let Some(group) = group else { return wanted.is_empty() };
    let group = group.to_ascii_lowercase();
    group == wanted || group.starts_with(&format!("{wanted}/"))
}

/// Rows that don't fit the fixed columns, stored as one JSON blob so new
/// request features don't each need a schema change.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Extra {
    params: Vec<KeyValue>,
    multipart: Vec<FormField>,
    #[serde(skip_serializing_if = "SavedOptions::is_empty")]
    options: SavedOptions,
    /// For a request that was sent: the value each non-secret `{{placeholder}}` had.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    sent: BTreeMap<String, String>,
}

/// `?1` is a LIKE pattern from `like_pattern`.
const MATCHES: &str = "url LIKE ?1 ESCAPE '\\' OR method LIKE ?1 ESCAPE '\\' \
     OR COALESCE(name, '') LIKE ?1 ESCAPE '\\' OR COALESCE(CAST(status AS TEXT), '') LIKE ?1 ESCAPE '\\'";

/// `%term%` with LIKE's own characters escaped, so searching for `100%` or `a_b` finds those.
fn like_pattern(term: &str) -> String {
    let escaped = term.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

impl HistoryEntry {
    /// Rebuild the request-shape part of app state from this history row, so
    /// clicking a sidebar entry can load it straight back into the form.
    pub fn to_persisted_state(&self) -> PersistedState {
        let mut state = PersistedState {
            method: self.method.clone(),
            url: self.url.clone(),
            headers_text: self.headers_text.clone(),
            body_mode: self.body_mode,
            json_body: self.json_body.clone(),
            urlencoded_body: self.urlencoded_body.clone(),
            raw_body: self.raw_body.clone(),
            params: self.params.clone(),
            multipart_fields: self.multipart_fields.clone(),
            // Session-wide settings are overlaid by callers with `with_session_from`.
            ..PersistedState::default()
        };
        // A raw body that is JSON, sent as JSON, opens in the JSON editor.
        state.infer_json_body();
        state
    }
}

fn body_mode_to_str(mode: BodyMode) -> &'static str {
    match mode {
        BodyMode::None => "None",
        BodyMode::Json => "Json",
        BodyMode::UrlEncoded => "UrlEncoded",
        BodyMode::Multipart => "Multipart",
        BodyMode::Raw => "Raw",
    }
}

fn body_mode_from_str(s: &str) -> BodyMode {
    match s {
        "Json" => BodyMode::Json,
        "UrlEncoded" => BodyMode::UrlEncoded,
        "Multipart" => BodyMode::Multipart,
        "Raw" => BodyMode::Raw,
        _ => BodyMode::None,
    }
}

pub struct History {
    conn: Connection,
    /// The audit log this database is the result of; None for a database that lives only in memory.
    log_path: Option<PathBuf>,
    /// The size at which the log is moved aside and started again.
    rotate_at: u64,
}

const COLUMNS: &str = "id, created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body, \
                       status, elapsed_ms, extra_json, name, source, group_name, error";

/// A request's columns as written to disk: credentials already blanked.
struct StoredRequest {
    method: String,
    url: String,
    headers_text: String,
    body_mode: &'static str,
    json_body: String,
    urlencoded_body: String,
    raw_body: String,
    extra: String,
}

impl StoredRequest {
    fn into_row(self) -> log::RequestRow {
        log::RequestRow {
            method: self.method,
            url: self.url,
            headers_text: self.headers_text,
            body_mode: self.body_mode.to_string(),
            json_body: self.json_body,
            urlencoded_body: self.urlencoded_body,
            raw_body: self.raw_body,
            extra: self.extra,
        }
    }
}

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

impl From<&PersistedState> for StoredRequest {
    fn from(state: &PersistedState) -> Self {
        // Same credential rules as the saved form state: blank secrets before they hit disk.
        let safe = state.redacted();
        let extra = serde_json::to_string(&Extra {
            params: safe.params,
            multipart: safe.multipart_fields,
            options: SavedOptions::default(),
            sent: BTreeMap::new(),
        })
        .unwrap_or_default();
        Self {
            method: safe.method,
            url: safe.url,
            headers_text: safe.headers_text,
            body_mode: body_mode_to_str(safe.body_mode),
            json_body: safe.json_body,
            urlencoded_body: safe.urlencoded_body,
            raw_body: safe.raw_body,
            extra,
        }
    }
}

impl History {
    /// Opens (creating if needed) the SQLite database in the platform's
    /// standard per-app data directory, next to eframe's own persistence file.
    pub fn open() -> rusqlite::Result<Self> {
        let dir = app_data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let database = dir.join("history.sqlite3");
        Self::with_connection(Connection::open(&database)?, Some(log::path_for(&database)))
    }

    /// Redacts rows written before redaction existed. Idempotent, so it is
    /// cheap to run on every start.
    fn scrub_credentials(&self) -> rusqlite::Result<()> {
        let rows: Vec<(i64, String, String)> = self
            .conn
            .prepare("SELECT id, url, headers_text FROM requests")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, url, headers) in rows {
            let (clean_url, clean_headers) = (redact_url(&url), redact_headers_text(&headers));
            if clean_url != url || clean_headers != headers {
                self.conn.execute(
                    "UPDATE requests SET url = ?1, headers_text = ?2 WHERE id = ?3",
                    params![clean_url, clean_headers, id],
                )?;
            }
        }
        Ok(())
    }

    /// A database at a specific path (tests: two connections, like the window and an agent).
    #[cfg(test)]
    pub fn open_at(path: &std::path::Path) -> Self {
        Self::with_connection(Connection::open(path).unwrap(), Some(log::path_for(path))).unwrap()
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::with_connection(Connection::open_in_memory().unwrap(), None).unwrap()
    }

    fn with_connection(conn: Connection, log_path: Option<PathBuf>) -> rusqlite::Result<Self> {
        // The window and an agent (CLI / MCP) can use the database at the same
        // time: write-ahead logging lets readers and a writer coexist, and the
        // busy timeout makes a second writer wait instead of failing.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS requests (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at      TEXT NOT NULL,
                method          TEXT NOT NULL,
                url             TEXT NOT NULL,
                headers_text    TEXT NOT NULL,
                body_mode       TEXT NOT NULL,
                json_body       TEXT NOT NULL,
                urlencoded_body TEXT NOT NULL,
                raw_body        TEXT NOT NULL,
                status          INTEGER,
                elapsed_ms      INTEGER
            )",
            [],
        )?;
        // Variables set by agents (CLI / MCP). Secret values never live here: they are kept in the
        // credential store and this row only records that the variable exists.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS agent_variables (
                name       TEXT PRIMARY KEY,
                value      TEXT NOT NULL DEFAULT '',
                secret     INTEGER NOT NULL DEFAULT 0,
                source     TEXT NOT NULL DEFAULT 'mcp',
                updated_at TEXT NOT NULL DEFAULT ''
            )",
            [],
        )?;
        // Workflows: an ordered list of steps (JSON), run by name from the CLI or MCP.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS workflows (
                name       TEXT PRIMARY KEY,
                steps      TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT ''
            )",
            [],
        )?;
        // One row per workflow run: who ran it, when, and each step's result (names of the variables it set,
        // never their values, and no response bodies).
        conn.execute(
            "CREATE TABLE IF NOT EXISTS workflow_runs (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                workflow   TEXT NOT NULL,
                started_at TEXT NOT NULL,
                source     TEXT NOT NULL,
                ok         INTEGER NOT NULL,
                cancelled  INTEGER NOT NULL DEFAULT 0,
                steps      TEXT NOT NULL
            )",
            [],
        )?;
        conn.execute("CREATE INDEX IF NOT EXISTS workflow_runs_by_name ON workflow_runs (workflow, id)", [])?;
        let workflow_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(workflows)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        if !workflow_columns.iter().any(|c| c == "group_name") {
            conn.execute("ALTER TABLE workflows ADD COLUMN group_name TEXT", [])?;
        }
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(requests)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        // Columns added after the first release; each is added once, in place.
        for (column, definition) in [
            ("extra_json", "extra_json TEXT NOT NULL DEFAULT ''"),
            ("name", "name TEXT"),
            // 0 once the history is cleared: a saved request outlives that.
            ("in_history", "in_history INTEGER NOT NULL DEFAULT 1"),
            ("source", "source TEXT NOT NULL DEFAULT 'gui'"),
            // the group a saved request is filed under ("Jira" or "Jira/Issues")
            ("group_name", "group_name TEXT"),
            // why a sent request got no response, scrubbed of secret values
            ("error", "error TEXT"),
        ] {
            if !columns.iter().any(|c| c == column) {
                conn.execute(&format!("ALTER TABLE requests ADD COLUMN {definition}"), [])?;
            }
        }
        // Triggers enforce exact-name uniqueness without rejecting databases
        // that already contain duplicate names from older versions.
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS unique_saved_name_on_insert
             BEFORE INSERT ON requests
             WHEN NEW.name IS NOT NULL
                  AND EXISTS (SELECT 1 FROM requests WHERE name = NEW.name)
             BEGIN
                 SELECT RAISE(ABORT, 'a saved request with this exact name already exists');
             END;

             CREATE TRIGGER IF NOT EXISTS unique_saved_name_on_update
             BEFORE UPDATE OF name ON requests
             WHEN NEW.name IS NOT NULL
                  AND EXISTS (SELECT 1 FROM requests WHERE name = NEW.name AND id != NEW.id)
             BEGIN
                 SELECT RAISE(ABORT, 'a saved request with this exact name already exists');
             END;",
        )?;
        let history = Self { conn, log_path, rotate_at: log::ROTATE_BYTES };
        // rows from before redaction existed are cleaned first, so the log never holds a credential
        history.scrub_credentials()?;
        history.init_log()?;
        Ok(history)
    }

    /// Records a request sent from the window. Returns the new row's id.
    pub fn insert(
        &self,
        state: &PersistedState,
        status: Option<u16>,
        elapsed_ms: Option<u128>,
    ) -> rusqlite::Result<i64> {
        self.insert_from(state, status, elapsed_ms, Source::Gui)
    }

    // ---- the audit log: every change is an event, appended before it is committed ----------------

    /// Applies `event` and records it in the audit log, in one transaction that also serialises writers
    /// across processes: apply (an error here rolls back and logs nothing), append the line and sync it,
    /// then commit. A crash after the append and before the commit leaves the log ahead of the tables,
    /// and the next open replays the difference.
    fn commit(&self, mut event: Event) -> rusqlite::Result<Committed> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match self.commit_in_transaction(&mut event) {
            Ok(done) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(done)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    fn commit_in_transaction(&self, event: &mut Event) -> rusqlite::Result<Committed> {
        let mut state = match log::read_state(&self.conn)? {
            Some(state) => state,
            None => self.fresh_state()?,
        };
        // lines another process appended and did not apply (it stopped in between)
        self.catch_up(&mut state)?;
        self.rotate_if_large(&mut state)?;
        // An older build (or a restored file) can have added rows the log never saw; ids continue after the
        // highest one in the table, or the next insert would collide with it and be lost.
        let top: i64 = self.conn.query_row("SELECT COALESCE(MAX(id), 0) FROM requests", [], |r| r.get(0))?;
        state.last_request_id = state.last_request_id.max(top);
        let top_run: i64 = self.conn.query_row("SELECT COALESCE(MAX(id), 0) FROM workflow_runs", [], |r| r.get(0))?;
        state.last_run_id = state.last_run_id.max(top_run);
        let mut id = 0;
        match event {
            Event::RequestAdded { id: slot, .. } => {
                state.last_request_id += 1;
                *slot = state.last_request_id;
                id = *slot;
            }
            Event::WorkflowRunRecorded { id: slot, .. } => {
                state.last_run_id += 1;
                *slot = state.last_run_id;
                id = *slot;
            }
            _ => {}
        }
        let applied = log::apply(&self.conn, event)?;
        self.append_entry(&mut state, event.clone())?;
        log::write_state(&self.conn, &state)?;
        Ok(Committed { id, changed: applied.changed })
    }

    /// A log that has grown past `rotate_at` is kept as `audit.<seq>.log` and a new one is started with a
    /// snapshot that names the hash it continues, so the chain stays verifiable across the files.
    fn rotate_if_large(&self, state: &mut log::State) -> rusqlite::Result<()> {
        let Some(path) = &self.log_path else { return Ok(()) };
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size < self.rotate_at || state.seq == 0 {
            return Ok(());
        }
        let archived = log::archive(path, state.seq).map_err(io_error)?;
        let name = archived.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut snapshot = log::snapshot_of(&self.conn, Some(format!("continues the log kept as {name}")))?;
        snapshot.continues = Some(state.hash.clone());
        state.offset = 0;
        self.append_entry(state, Event::Snapshot(snapshot))
    }

    #[cfg(test)]
    pub fn rotate_at(mut self, bytes: u64) -> Self {
        self.rotate_at = bytes;
        self
    }

    fn append_entry(&self, state: &mut log::State, event: Event) -> rusqlite::Result<()> {
        match &self.log_path {
            Some(path) => {
                let (entry, offset) = log::append(path, state, event).map_err(io_error)?;
                state.seq = entry.seq;
                state.hash = entry.hash;
                state.offset = offset;
            }
            None => state.seq += 1,
        }
        Ok(())
    }

    /// The state for a database the log has not seen: ids continue after the ones already there.
    fn fresh_state(&self) -> rusqlite::Result<log::State> {
        let mut state = log::State::fresh();
        state.last_request_id = self.conn.query_row("SELECT COALESCE(MAX(id), 0) FROM requests", [], |r| r.get(0))?;
        state.last_run_id = self.conn.query_row("SELECT COALESCE(MAX(id), 0) FROM workflow_runs", [], |r| r.get(0))?;
        Ok(state)
    }

    /// Applies the log lines after `state.offset` that the tables do not have yet.
    fn catch_up(&self, state: &mut log::State) -> rusqlite::Result<usize> {
        let Some(path) = &self.log_path else { return Ok(0) };
        let mut applied = 0;
        for (entry, offset) in log::read_from(path, state.offset).map_err(io_error)? {
            // the first line of a rotated log follows the file that was set aside: its snapshot says so
            let continues_archive = state.seq == 0
                && matches!(&entry.event, Event::Snapshot(s) if s.continues.as_deref() == Some(entry.prev.as_str()));
            if (entry.prev != state.hash && !continues_archive) || !entry.is_sound() {
                // not the next line of this chain: leave it for `audit verify` to report
                break;
            }
            log::apply(&self.conn, &entry.event)?;
            match &entry.event {
                Event::RequestAdded { id, .. } => state.last_request_id = state.last_request_id.max(*id),
                Event::WorkflowRunRecorded { id, .. } => state.last_run_id = state.last_run_id.max(*id),
                Event::Snapshot(s) => {
                    state.last_request_id = state.last_request_id.max(s.requests.iter().map(|r| r.id).max().unwrap_or(0));
                    state.last_run_id = state.last_run_id.max(s.runs.iter().map(|r| r.id).max().unwrap_or(0));
                }
                _ => {}
            }
            state.seq = entry.seq;
            state.hash = entry.hash;
            state.offset = offset;
            applied += 1;
        }
        Ok(applied)
    }

    /// At open: start the log on a database that has none (a snapshot as its first line), or catch the
    /// tables up with a log that is ahead of them.
    fn init_log(&self) -> rusqlite::Result<()> {
        log::ensure_state_table(&self.conn)?;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = self.init_log_in_transaction();
        match result {
            Ok(()) => self.conn.execute_batch("COMMIT"),
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    fn init_log_in_transaction(&self) -> rusqlite::Result<()> {
        match log::read_state(&self.conn)? {
            Some(mut state) => {
                // the log was deleted under a running database: say so, in a new line that does not
                // follow the old chain, so the gap shows when it is verified
                let missing = self.log_path.as_ref().is_some_and(|p| !p.exists()) && state.seq > 0;
                if missing {
                    let note = Some("the audit log file was missing and has been started again".to_string());
                    let snapshot = log::snapshot_of(&self.conn, note)?;
                    state.offset = 0;
                    self.append_entry(&mut state, Event::Snapshot(snapshot))?;
                    return log::write_state(&self.conn, &state);
                }
                if self.catch_up(&mut state)? > 0 {
                    log::write_state(&self.conn, &state)?;
                }
                Ok(())
            }
            None => {
                let mut state = self.fresh_state()?;
                match &self.log_path {
                    Some(path) => {
                        let existing = log::read_from(path, 0).map_err(io_error)?;
                        if existing.is_empty() {
                            // a database with no log: the log starts with everything that is in it
                            let snapshot = log::snapshot_of(&self.conn, None)?;
                            self.append_entry(&mut state, Event::Snapshot(snapshot))?;
                        } else {
                            // a log with no database state: the tables are what the log says
                            for table in ["requests", "workflows", "workflow_runs", "agent_variables"] {
                                self.conn.execute(&format!("DELETE FROM {table}"), [])?;
                            }
                            state = log::State::fresh();
                            self.catch_up(&mut state)?;
                        }
                    }
                    None => state.seq = 0,
                }
                log::write_state(&self.conn, &state)
            }
        }
    }

    /// Rebuilds the tables from the audit log: they are emptied and every entry is applied again. Returns
    /// the number of entries applied. The log is not changed.
    pub fn rebuild_from_log(&self) -> rusqlite::Result<u64> {
        let Some(path) = &self.log_path else { return Ok(0) };
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> rusqlite::Result<u64> {
            for table in ["requests", "workflows", "workflow_runs", "agent_variables"] {
                self.conn.execute(&format!("DELETE FROM {table}"), [])?;
            }
            let mut state = log::State::fresh();
            let applied = self.catch_up(&mut state)? as u64;
            if state.offset != std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) {
                return Err(io_error(std::io::Error::other(
                    "the audit log does not verify, so the database was left as it was (run `plunger audit verify`)",
                )));
            }
            log::write_state(&self.conn, &state)?;
            Ok(applied)
        })();
        match result {
            Ok(n) => self.conn.execute_batch("COMMIT").map(|_| n),
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// The audit log of this database, when it has one.
    pub fn log_path(&self) -> Option<&std::path::Path> {
        self.log_path.as_deref()
    }

    /// Records a sent request, noting who sent it. Returns the new row's id.
    pub fn insert_from(
        &self,
        state: &PersistedState,
        status: Option<u16>,
        elapsed_ms: Option<u128>,
        source: Source,
    ) -> rusqlite::Result<i64> {
        // only unnamed history is pruned (when the event is applied); saved requests are kept however many there are
        self.insert_row(state, status, elapsed_ms, None, None, source)
    }

    /// Records a request that was sent but got no response, with the error text the agent was given.
    pub fn record_failure(&self, state: &PersistedState, error: &str, source: Source) -> rusqlite::Result<i64> {
        self.insert_row(state, None, None, None, Some(error), source)
    }

    /// Saves a request under `name` without adding it to the history list.
    pub fn save_new(&self, state: &PersistedState, name: &str) -> rusqlite::Result<i64> {
        self.insert_row(state, None, None, Some(name), None, Source::Gui)
    }

    /// Saves a request under `name`, noting who saved it.
    pub fn save_new_from(&self, state: &PersistedState, name: &str, source: Source) -> rusqlite::Result<i64> {
        self.insert_row(state, None, None, Some(name), None, source)
    }

    /// Changes whenever any connection (another process included) commits to
    /// the database, so the window can refresh its lists when an agent sends.
    pub fn data_version(&self) -> Option<i64> {
        self.conn.query_row("PRAGMA data_version", [], |r| r.get(0)).ok()
    }

    fn insert_row(
        &self,
        state: &PersistedState,
        status: Option<u16>,
        elapsed_ms: Option<u128>,
        name: Option<&str>,
        error: Option<&str>,
        source: Source,
    ) -> rusqlite::Result<i64> {
        let created_at = now();
        let mut row = StoredRequest::from(state);
        if name.is_none() {
            // a sent request remembers what its placeholders stood for, so a parameterised one can be
            // compared with what actually went out
            let sent = sent_values(state);
            if !sent.is_empty() {
                let mut extra: Extra = serde_json::from_str(&row.extra).unwrap_or_default();
                extra.sent = sent;
                row.extra = serde_json::to_string(&extra).unwrap_or(row.extra);
            }
        }
        let applied = self.commit(Event::RequestAdded {
            id: 0, // handed out by `commit`
            created_at,
            source: source.as_str().to_string(),
            name: name.map(str::to_string),
            // A request saved directly (not from a send) isn't part of the history.
            in_history: name.is_none(),
            status: status.map(|s| s as i64),
            elapsed_ms: elapsed_ms.map(|e| e as i64),
            error: error.map(str::to_string),
            row: row.into_row(),
        })?;
        Ok(applied.id)
    }

    /// Overwrites a saved request's contents with `state`. Returns false if
    /// the row no longer exists.
    pub fn update_request(&self, id: i64, state: &PersistedState) -> rusqlite::Result<bool> {
        let Some(current) = self.get(id)? else { return Ok(false) };
        let mut row = StoredRequest::from(state);
        // saving the form again (the window's Ctrl+S) must not drop the options an agent saved with it
        if !current.options.is_empty() {
            let mut extra: Extra = serde_json::from_str(&row.extra).unwrap_or_default();
            extra.options = current.options;
            row.extra = serde_json::to_string(&extra).unwrap_or(row.extra);
        }
        Ok(self.commit(Event::RequestUpdated { id, row: row.into_row() })?.changed > 0)
    }

    /// Names (saves) a row, or with `None` un-saves it. A saved request that is
    /// no longer in the history list has nothing left to show, so it's deleted.
    pub fn set_name(&self, id: i64, name: Option<&str>) -> rusqlite::Result<()> {
        self.commit(Event::RequestNamed { id, name: name.map(str::to_string) })?;
        Ok(())
    }

    /// Saved requests, alphabetically.
    pub fn list_saved(&self) -> rusqlite::Result<Vec<HistoryEntry>> {
        self.search_saved("")
    }

    /// Saved requests whose name, URL, method or status contains `term` (case-insensitive).
    pub fn search_saved(&self, term: &str) -> rusqlite::Result<Vec<HistoryEntry>> {
        self.query(
            &format!(
                "SELECT {COLUMNS} FROM requests WHERE name IS NOT NULL AND ({MATCHES}) ORDER BY COALESCE(group_name, '') COLLATE NOCASE, name COLLATE NOCASE, id"
            ),
            params![like_pattern(term)],
        )
    }

    /// Replaces the options a saved request carries (empty clears them).
    pub fn set_saved_options(&self, id: i64, options: &SavedOptions) -> rusqlite::Result<bool> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT extra_json FROM requests WHERE id = ?1", params![id], |row| row.get(0))
            .optional()?;
        let Some(raw) = raw else { return Ok(false) };
        let mut extra: Extra = serde_json::from_str(&raw).unwrap_or_default();
        extra.options = options.clone();
        let text = serde_json::to_string(&extra).unwrap_or_default();
        Ok(self.commit(Event::RequestExtraSet { id, extra: text })?.changed > 0)
    }

    /// Files a saved request under `group` (None takes it out of its group). Returns false when there
    /// is no such request.
    pub fn set_group(&self, id: i64, group: Option<&str>) -> rusqlite::Result<bool> {
        if self.get(id)?.is_none() {
            return Ok(false);
        }
        Ok(self.commit(Event::RequestGrouped { id, group: group.map(str::to_string) })?.changed > 0)
    }

    /// One request (history or saved) by id.
    pub fn get(&self, id: i64) -> rusqlite::Result<Option<HistoryEntry>> {
        Ok(self.query(&format!("SELECT {COLUMNS} FROM requests WHERE id = ?1"), params![id])?.pop())
    }

    /// Sent requests whose name, URL, method or status contains `term` (all of them when it
    /// is empty), newest first. Saved ones appear here too until the history is cleared. The
    /// whole table is searched, not just the rows the sidebar shows.
    pub fn search_recent(&self, term: &str, limit: i64) -> rusqlite::Result<Vec<HistoryEntry>> {
        self.query(
            &format!("SELECT {COLUMNS} FROM requests WHERE in_history = 1 AND ({MATCHES}) ORDER BY id DESC LIMIT ?2"),
            params![like_pattern(term), limit],
        )
    }

    fn query(&self, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(args, |row| {
            let extra: Extra = serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default();
            Ok(HistoryEntry {
                params: extra.params,
                multipart_fields: extra.multipart,
                options: extra.options,
                sent_values: extra.sent,
                id: row.get(0)?,
                created_at: row.get(1)?,
                method: row.get(2)?,
                url: row.get(3)?,
                headers_text: row.get(4)?,
                body_mode: body_mode_from_str(&row.get::<_, String>(5)?),
                json_body: row.get(6)?,
                urlencoded_body: row.get(7)?,
                raw_body: row.get(8)?,
                status: row.get(9)?,
                elapsed_ms: row.get(10)?,
                name: row.get(12)?,
                source: Source::parse(&row.get::<_, String>(13)?),
                group: row.get(14)?,
                error: row.get(15)?,
            })
        })?;
        rows.collect()
    }

    /// How many requests are saved (named) and how many are in the history list.
    pub fn counts(&self) -> rusqlite::Result<(i64, i64)> {
        let saved = self.conn.query_row("SELECT COUNT(*) FROM requests WHERE name IS NOT NULL", [], |r| r.get(0))?;
        let listed = self.conn.query_row("SELECT COUNT(*) FROM requests WHERE in_history = 1", [], |r| r.get(0))?;
        Ok((saved, listed))
    }

    /// Empties the history list. Saved requests are kept (and stay in the
    /// Saved list); they just stop appearing under History.
    pub fn clear(&self) -> rusqlite::Result<()> {
        self.commit(Event::HistoryCleared)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_runs_are_recorded_listed_newest_first_and_pruned() {
        let h = history();
        for i in 0..25 {
            let run = NewWorkflowRun { workflow: "shop", started_at: &format!("2026-10-10T09:00:{i:02}Z"), source: if i % 2 == 0 { Source::Mcp } else { Source::Cli }, ok: i != 24, cancelled: false, steps: "[]" };
            h.record_workflow_run(&run).unwrap();
        }
        h.record_workflow_run(&NewWorkflowRun { workflow: "other", started_at: "2026-10-10T10:00:00Z", source: Source::Gui, ok: true, cancelled: true, steps: "[]" }).unwrap();
        let runs = h.workflow_runs("shop", 100).unwrap();
        assert_eq!(runs.len() as i64, WORKFLOW_RUNS_KEPT, "only the newest are kept");
        assert_eq!(runs[0].started_at, "2026-10-10T09:00:24Z");
        assert!(!runs[0].ok && runs[1].ok);
        let latest = h.latest_workflow_runs().unwrap();
        assert_eq!(latest.len(), 2, "one per workflow");
        assert!(latest.iter().any(|r| r.workflow == "other" && r.cancelled && r.source == Source::Gui));
        h.save_workflow("shop", "[]").unwrap();
        assert!(h.delete_workflow("shop").unwrap());
        assert!(h.workflow_runs("shop", 10).unwrap().is_empty(), "deleting a workflow removes its runs");
        assert_eq!(h.workflow_runs("other", 10).unwrap().len(), 1);
    }

    #[test]
    fn a_raw_json_body_comes_back_in_the_json_editor() {
        let h = history();
        let mut s = state("http://a/raw", BodyMode::Raw);
        s.headers_text = "Content-Type: application/json".into();
        s.raw_body = "{\"k\": 1}".into();
        h.insert(&s, Some(200), Some(1)).unwrap();
        let restored = h.search_recent("", 5).unwrap()[0].to_persisted_state();
        assert!(restored.body_mode == BodyMode::Json);
        assert_eq!(restored.json_body, "{\"k\": 1}");
        assert!(restored.raw_body.is_empty());
    }

    #[test]
    fn a_row_an_older_build_added_does_not_block_new_history() {
        let h = history();
        h.insert(&state("http://a", BodyMode::Json), Some(200), Some(1)).unwrap();
        // an older build wrote rows the log's counter never saw
        h.conn.execute("INSERT INTO requests (id, created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body) VALUES (50, '2026-01-01T00:00:00Z', 'GET', 'http://old', '', 'None', '', '', '')", []).unwrap();
        let id = h.insert(&state("http://new", BodyMode::Json), Some(200), Some(1)).unwrap();
        assert!(id > 50, "the new row continues after the highest id, got {id}");
        assert_eq!(h.search_recent("", 10).unwrap()[0].url, "http://new");
    }

    fn history() -> History {
        History::with_connection(Connection::open_in_memory().unwrap(), None).unwrap()
    }

    fn state(url: &str, mode: BodyMode) -> PersistedState {
        PersistedState {
            method: "POST".into(),
            url: url.into(),
            body_mode: mode,
            json_body: "{\"k\":1}".into(),
            ..Default::default()
        }
    }

    #[test]
    fn insert_then_list_newest_first_and_round_trips_state() {
        let h = history();
        h.insert(&state("http://a", BodyMode::Json), Some(200), Some(12)).unwrap();
        h.insert(&state("http://b", BodyMode::Raw), None, None).unwrap();

        let rows = h.search_recent("", 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].url, "http://b");
        assert_eq!(rows[0].status, None);
        assert_eq!(rows[1].status, Some(200));
        assert_eq!(rows[1].elapsed_ms, Some(12));

        let restored = rows[1].to_persisted_state();
        assert_eq!(restored.url, "http://a");
        assert!(restored.body_mode == BodyMode::Json);
        assert_eq!(restored.json_body, "{\"k\":1}");
    }

    #[test]
    fn recent_honours_limit_and_clear_empties() {
        let h = history();
        for i in 0..5 {
            h.insert(&state(&format!("http://x/{i}"), BodyMode::None), Some(200), Some(1)).unwrap();
        }
        assert_eq!(h.search_recent("", 3).unwrap().len(), 3);
        h.clear().unwrap();
        assert!(h.search_recent("", 10).unwrap().is_empty());
    }

    #[test]
    fn credentials_never_reach_the_database() {
        let h = history();
        let mut s = state("https://bob:pw@a.com/x?token=T&page=1", BodyMode::None);
        s.headers_text = "Accept: */*\nAuthorization: Bearer SECRET\nCookie: sid=1".into();
        h.insert(&s, Some(200), Some(1)).unwrap();

        let row = &h.search_recent("", 1).unwrap()[0];
        assert_eq!(row.url, "https://bob@a.com/x?token=&page=1");
        assert_eq!(row.headers_text, "Accept: */*\nAuthorization:\nCookie:");
        assert!(!format!("{} {}", row.url, row.headers_text).contains("SECRET"));
    }

    #[test]
    fn agent_variables_are_stored_listed_updated_and_removed() {
        let h = history();
        assert!(h.list_agent_variables().unwrap().is_empty());
        h.set_agent_variable("base", "http://h", false, Source::Mcp).unwrap();
        h.set_agent_variable("token", "", true, Source::Cli).unwrap();
        h.set_agent_variable("base", "http://other", false, Source::Cli).unwrap();
        let list = h.list_agent_variables().unwrap();
        assert_eq!(list.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["base", "token"]);
        assert_eq!((list[0].value.as_str(), list[0].source.as_str()), ("http://other", "cli"));
        assert!(list[1].secret && list[1].value.is_empty());
        assert!(h.delete_agent_variable("base").unwrap());
        assert!(!h.delete_agent_variable("base").unwrap());
        assert_eq!(h.clear_agent_variables().unwrap(), vec!["token".to_string()]);
        assert!(h.list_agent_variables().unwrap().is_empty());
    }

    #[test]
    fn agent_variables_are_shared_between_connections() {
        let path = std::env::temp_dir().join(format!("plunger-agentvars-{}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let writer = History::open_at(&path);
        let reader = History::open_at(&path);
        writer.set_agent_variable("a", "1", false, Source::Mcp).unwrap();
        assert_eq!(reader.list_agent_variables().unwrap().len(), 1);
        drop((writer, reader));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_saved_request_keeps_its_variable_placeholders_in_credential_headers() {
        let h = history();
        let mut s = state("https://a.com/x?token={{t}}", BodyMode::None);
        s.headers_text = "Authorization: Bearer {{token}}
X-Api-Key: literal".into();
        let id = h.save_new(&s, "Keeps refs").unwrap();
        let row = h.list_saved().unwrap().into_iter().find(|r| r.id == id).unwrap();
        assert_eq!(row.headers_text, "Authorization: Bearer {{token}}
X-Api-Key:");
        assert_eq!(row.url, "https://a.com/x?token={{t}}");
    }

    #[test]
    fn rows_saved_before_redaction_existed_get_scrubbed() {
        let h = history();
        h.conn
            .execute(
                "INSERT INTO requests (created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body)
                 VALUES ('t', 'GET', 'https://a.com/?api_key=OLD', 'Authorization: Bearer OLD', 'None', '', '', '')",
                [],
            )
            .unwrap();
        h.scrub_credentials().unwrap();
        h.scrub_credentials().unwrap(); // idempotent
        let row = &h.search_recent("", 1).unwrap()[0];
        assert_eq!(row.url, "https://a.com/?api_key=");
        assert_eq!(row.headers_text, "Authorization:");
    }

    #[test]
    fn params_and_multipart_round_trip_with_secrets_blanked() {
        use crate::domain::model::FieldKind;
        let h = history();
        let mut s = state("http://a", BodyMode::Multipart);
        s.params = vec![
            KeyValue { key: "page".into(), value: "2".into(), enabled: true },
            KeyValue { key: "api_key".into(), value: "SECRET".into(), enabled: false },
        ];
        s.multipart_fields = vec![
            FormField { key: "title".into(), kind: FieldKind::Text, value: "hi".into(), enabled: true },
            FormField { key: "password".into(), kind: FieldKind::Text, value: "pw".into(), enabled: true },
            FormField { key: "doc".into(), kind: FieldKind::File, value: "C:/x/a.pdf".into(), enabled: true },
        ];
        h.insert(&s, Some(200), Some(1)).unwrap();

        let restored = h.search_recent("", 1).unwrap()[0].to_persisted_state();
        assert!(restored.body_mode == BodyMode::Multipart);
        assert_eq!(restored.params[0].value, "2");
        assert_eq!(restored.params[1].value, "");
        assert!(!restored.params[1].enabled);
        assert_eq!(restored.multipart_fields[0].value, "hi");
        assert_eq!(restored.multipart_fields[1].value, "");
        assert_eq!(restored.multipart_fields[2].value, "C:/x/a.pdf");
    }

    #[test]
    fn a_database_created_before_extra_json_is_migrated_in_place() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT, created_at TEXT NOT NULL, method TEXT NOT NULL,
                url TEXT NOT NULL, headers_text TEXT NOT NULL, body_mode TEXT NOT NULL,
                json_body TEXT NOT NULL, urlencoded_body TEXT NOT NULL, raw_body TEXT NOT NULL,
                status INTEGER, elapsed_ms INTEGER)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO requests (created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body)
             VALUES ('t', 'GET', 'http://legacy', '', 'None', '', '', '')",
            [],
        )
        .unwrap();

        let h = History::with_connection(conn, None).unwrap();
        let rows = h.search_recent("", 10).unwrap();
        assert_eq!(rows[0].url, "http://legacy");
        assert!(rows[0].params.is_empty() && rows[0].multipart_fields.is_empty());
        // opening again must not try to add the column twice
        let h2 = History::with_connection(h.conn, None).unwrap();
        h2.insert(&state("http://new", BodyMode::None), None, None).unwrap();
    }

    #[test]
    fn a_failed_send_keeps_its_error_and_an_old_database_gets_the_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT, created_at TEXT NOT NULL, method TEXT NOT NULL,
                url TEXT NOT NULL, headers_text TEXT NOT NULL, body_mode TEXT NOT NULL,
                json_body TEXT NOT NULL, urlencoded_body TEXT NOT NULL, raw_body TEXT NOT NULL,
                status INTEGER, elapsed_ms INTEGER)",
            [],
        )
        .unwrap();
        let h = History::with_connection(conn, None).unwrap();
        let id = h.record_failure(&state("http://down", BodyMode::None), "connection refused", Source::Mcp).unwrap();
        let entry = h.get(id).unwrap().unwrap();
        assert_eq!(entry.error.as_deref(), Some("connection refused"));
        assert_eq!(entry.status, None);
        // opening again must not try to add the column twice, and the error is still there
        let h2 = History::with_connection(h.conn, None).unwrap();
        assert_eq!(h2.get(id).unwrap().unwrap().error.as_deref(), Some("connection refused"));
    }

    #[test]
    fn old_rows_are_pruned_beyond_the_cap() {
        let h = history();
        for i in 0..(crate::store::log::MAX_HISTORY_ROWS + 25) {
            h.insert(&state(&format!("http://x/{i}"), BodyMode::None), Some(200), Some(1)).unwrap();
        }
        let rows = h.search_recent("", crate::store::log::MAX_HISTORY_ROWS + 100).unwrap();
        assert_eq!(rows.len() as i64, crate::store::log::MAX_HISTORY_ROWS);
        assert_eq!(rows[0].url, format!("http://x/{}", crate::store::log::MAX_HISTORY_ROWS + 24));
        assert_eq!(rows.last().unwrap().url, "http://x/25");
    }

    #[test]
    fn naming_a_row_saves_it_and_it_survives_clearing_the_history() {
        let h = history();
        let a = h.insert(&state("http://a", BodyMode::None), Some(200), Some(1)).unwrap();
        h.insert(&state("http://b", BodyMode::None), Some(200), Some(1)).unwrap();
        h.set_name(a, Some("Get A")).unwrap();

        let saved = h.list_saved().unwrap();
        assert_eq!((saved.len(), saved[0].name.as_deref()), (1, Some("Get A")));
        // Still in the history list, now carrying its name.
        assert_eq!(h.search_recent("", 10).unwrap().iter().filter(|r| r.name.is_some()).count(), 1);

        h.clear().unwrap();
        assert!(h.search_recent("", 10).unwrap().is_empty());
        assert_eq!(h.list_saved().unwrap()[0].url, "http://a");

        // Un-saving something no longer in the history deletes it for good.
        h.set_name(a, None).unwrap();
        assert!(h.list_saved().unwrap().is_empty());
    }

    #[test]
    fn unsaving_a_row_still_in_the_history_keeps_it_there() {
        let h = history();
        let a = h.insert(&state("http://a", BodyMode::None), Some(200), Some(1)).unwrap();
        h.set_name(a, Some("A")).unwrap();
        h.set_name(a, None).unwrap();
        let rows = h.search_recent("", 10).unwrap();
        assert_eq!((rows.len(), rows[0].name.clone()), (1, None));
    }

    #[test]
    fn a_saved_request_is_updated_in_place_and_not_listed_as_history() {
        let h = history();
        let id = h.save_new(&state("http://v1", BodyMode::None), "Mine").unwrap();
        assert!(h.search_recent("", 10).unwrap().is_empty());
        let mut s = state("http://v2?token=T", BodyMode::Json);
        s.headers_text = "Authorization: Bearer X".into();
        assert!(h.update_request(id, &s).unwrap());
        let row = &h.list_saved().unwrap()[0];
        assert_eq!((row.url.as_str(), row.headers_text.as_str()), ("http://v2?token=", "Authorization:"));
        assert!(!h.update_request(9999, &s).unwrap());
    }

    #[test]
    fn exact_saved_names_are_rejected_on_save_and_rename() {
        let h = history();
        let first = h.save_new(&state("http://a", BodyMode::None), "Duplicate").unwrap();

        let error = h
            .save_new_from(&state("http://b", BodyMode::None), "Duplicate", Source::Mcp)
            .unwrap_err();
        assert!(error.to_string().contains("exact name already exists"));

        let second = h.save_new(&state("http://c", BodyMode::None), "Other").unwrap();
        let error = h.set_name(second, Some("Duplicate")).unwrap_err();
        assert!(error.to_string().contains("exact name already exists"));
        assert_eq!(h.get(second).unwrap().unwrap().name.as_deref(), Some("Other"));

        // Updating a row without changing its name must remain valid.
        h.set_name(first, Some("Duplicate")).unwrap();
        assert_eq!(h.list_saved().unwrap().len(), 2);
    }

    #[test]
    fn saved_requests_are_never_pruned() {
        let h = history();
        let keep = h.insert(&state("http://keep", BodyMode::None), None, None).unwrap();
        h.set_name(keep, Some("Keep")).unwrap();
        for i in 0..(crate::store::log::MAX_HISTORY_ROWS + 5) {
            h.insert(&state(&format!("http://x/{i}"), BodyMode::None), None, None).unwrap();
        }
        assert_eq!(h.list_saved().unwrap().len(), 1);
    }

    #[test]
    fn a_row_is_found_by_id() {
        let h = history();
        let id = h.insert(&state("http://a", BodyMode::None), Some(200), Some(1)).unwrap();
        assert_eq!(h.get(id).unwrap().unwrap().url, "http://a");
        assert!(h.get(id + 99).unwrap().is_none());
    }

    #[test]
    fn the_sender_is_recorded_and_read_back() {
        let h = history();
        h.insert(&state("http://gui", BodyMode::None), Some(200), Some(1)).unwrap();
        h.insert_from(&state("http://agent", BodyMode::None), Some(200), Some(1), Source::Mcp).unwrap();
        let rows = h.search_recent("", 10).unwrap();
        assert_eq!((rows[0].url.as_str(), rows[0].source), ("http://agent", Source::Mcp));
        assert_eq!(rows[1].source, Source::Gui);
    }

    #[test]
    fn a_write_from_another_connection_changes_the_data_version() {
        let dir = std::env::temp_dir().join(format!("plunger-dv-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("h.sqlite3");
        let _ = std::fs::remove_file(&path);
        let window = History::with_connection(Connection::open(&path).unwrap(), Some(crate::store::log::path_for(&path))).unwrap();
        let agent = History::with_connection(Connection::open(&path).unwrap(), Some(crate::store::log::path_for(&path))).unwrap();
        let before = window.data_version();
        agent.insert_from(&state("http://x", BodyMode::None), Some(200), Some(1), Source::Cli).unwrap();
        assert_ne!(window.data_version(), before);
        assert_eq!(window.search_recent("", 5).unwrap().len(), 1);
    }

    #[test]
    fn body_mode_string_round_trip() {
        for m in [BodyMode::None, BodyMode::Json, BodyMode::UrlEncoded, BodyMode::Raw] {
            assert!(body_mode_from_str(body_mode_to_str(m)) == m);
        }
        assert!(body_mode_from_str("garbage") == BodyMode::None);
    }

    #[test]
    fn search_matches_url_name_method_and_status_across_the_whole_history() {
        let h = history();
        h.insert(&state("http://api/users", BodyMode::None), Some(200), Some(1)).unwrap();
        h.insert(&state("http://api/orders?x=100%", BodyMode::None), Some(404), Some(1)).unwrap();
        let named = h.save_new(&state("http://other/a_b", BodyMode::None), "Login flow").unwrap();
        // Far more rows than a sidebar would show: the match must still be found.
        for i in 0..80 {
            h.insert(&state(&format!("http://filler/{i}"), BodyMode::None), Some(200), Some(1)).unwrap();
        }
        let urls = |rows: Vec<HistoryEntry>| rows.into_iter().map(|r| r.url).collect::<Vec<_>>();
        assert_eq!(urls(h.search_recent("USERS", 10).unwrap()), ["http://api/users"]);
        assert_eq!(urls(h.search_recent("404", 10).unwrap()), ["http://api/orders?x=100%"]);
        // LIKE's own characters are literal.
        assert_eq!(urls(h.search_recent("100%", 10).unwrap()), ["http://api/orders?x=100%"]);
        assert_eq!(urls(h.search_saved("a_b").unwrap()), ["http://other/a_b"]);
        assert_eq!(urls(h.search_saved("login").unwrap()), ["http://other/a_b"]);
        assert!(h.search_saved("zzz").unwrap().is_empty());
        assert_eq!(h.search_recent("", 5).unwrap().len(), 5);
        assert!(h.search_recent("a_b", 5).unwrap().iter().all(|r| r.id != named + 1000));
    }

    #[test]
    fn a_sent_request_remembers_what_its_placeholders_stood_for_but_never_a_secret() {
        use crate::domain::model::Variable;
        let h = history();
        let state = PersistedState {
            url: "http://h/{{key}}?t={{api_token}}&n={{missing}}".into(),
            headers_text: "X-Env: {{$env:HOME}}".into(),
            variables: vec![
                Variable { name: "key".into(), value: "PRJ-42".into(), secret: false, remember: false },
                Variable { name: "api_token".into(), value: "s3cret".into(), secret: false, remember: false },
                Variable { name: "long".into(), value: "x".repeat(900), secret: false, remember: false },
            ],
            ..Default::default()
        };
        h.insert(&state, Some(200), Some(5)).unwrap();
        let entry = h.search_recent("", 1).unwrap().remove(0);
        assert_eq!(entry.sent_values.get("key").map(String::as_str), Some("PRJ-42"));
        assert!(!entry.sent_values.contains_key("api_token"), "a credential-looking name is never kept");
        assert!(!entry.sent_values.contains_key("missing") && !entry.sent_values.contains_key("$env:HOME"));
        // a saved request is a template: it keeps no values
        let id = h.save_new(&state, "tmpl").unwrap();
        assert!(h.get(id).unwrap().unwrap().sent_values.is_empty());
    }

    #[test]
    fn group_names_are_tidied_and_matched_with_what_is_under_them() {
        assert_eq!(normalize_group("  Jira / Issues ").unwrap(), Some("Jira/Issues".to_string()));
        assert_eq!(normalize_group("Jira").unwrap(), Some("Jira".to_string()));
        assert_eq!(normalize_group("   ").unwrap(), None);
        assert_eq!(normalize_group("").unwrap(), None);
        for bad in ["Jira//Issues", "/Jira", "Jira/", "a\u{7}b", &"x".repeat(81)] {
            assert!(normalize_group(bad).is_err(), "{bad:?}");
        }
        assert!(group_matches(Some("Jira"), "jira"));
        assert!(group_matches(Some("Jira/Issues"), "Jira"));
        assert!(group_matches(Some("Jira/Issues"), "Jira/Issues"));
        assert!(!group_matches(Some("Jira/Issues"), "Jira/Boards"));
        assert!(!group_matches(Some("Jiraffe"), "Jira"), "a prefix of the name is not the group");
        assert!(!group_matches(None, "Jira"));
    }

    #[test]
    fn a_saved_request_and_a_workflow_keep_their_group_and_the_list_is_ordered_by_it() {
        let h = history();
        let state = PersistedState { url: "http://h/a".into(), ..Default::default() };
        let a = h.save_new(&state, "b request").unwrap();
        let b = h.save_new(&state, "a request").unwrap();
        let c = h.save_new(&state, "c request").unwrap();
        assert!(h.set_group(a, Some("Jira")).unwrap());
        assert!(h.set_group(b, Some("GitLab")).unwrap());
        assert!(!h.set_group(9999, Some("x")).unwrap());
        let names: Vec<(String, Option<String>)> =
            h.list_saved().unwrap().into_iter().map(|e| (e.name.unwrap(), e.group)).collect();
        assert_eq!(
            names,
            vec![
                ("c request".to_string(), None),
                ("a request".to_string(), Some("GitLab".to_string())),
                ("b request".to_string(), Some("Jira".to_string())),
            ],
            "ungrouped first, then by group, then by name"
        );
        assert!(h.set_group(c, Some("Jira/Issues")).unwrap());
        assert!(h.set_group(a, None).unwrap());
        assert_eq!(h.get(a).unwrap().unwrap().group, None);

        h.save_workflow("flow", "[]").unwrap();
        assert!(h.set_workflow_group("flow", Some("Jira")).unwrap());
        assert_eq!(h.workflow_groups().unwrap().get("flow").map(String::as_str), Some("Jira"));
        // saving the workflow again does not drop its group
        h.save_workflow("flow", "[]").unwrap();
        assert_eq!(h.workflow_groups().unwrap().get("flow").map(String::as_str), Some("Jira"));
        // and saving the form again does not drop a request's group
        h.update_request(c, &state).unwrap();
        assert_eq!(h.get(c).unwrap().unwrap().group.as_deref(), Some("Jira/Issues"));
    }
}