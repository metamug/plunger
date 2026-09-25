use crate::model::{BodyMode, FormField, KeyValue, PersistedState};
use crate::redact::{redact_headers_text, redact_url};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Oldest rows beyond this are pruned on insert so the database can't grow forever.
const MAX_ROWS: i64 = 1000;

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
}

/// Rows that don't fit the fixed columns, stored as one JSON blob so new
/// request features don't each need a schema change.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Extra {
    params: Vec<KeyValue>,
    multipart: Vec<FormField>,
}

impl HistoryEntry {
    /// Rebuild the request-shape part of app state from this history row, so
    /// clicking a sidebar entry can load it straight back into the form.
    pub fn to_persisted_state(&self) -> PersistedState {
        PersistedState {
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
        }
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
}

const COLUMNS: &str = "id, created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body, \
                       status, elapsed_ms, extra_json, name, source";

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

impl From<&PersistedState> for StoredRequest {
    fn from(state: &PersistedState) -> Self {
        // Same credential rules as the saved form state: blank secrets before they hit disk.
        let safe = state.redacted();
        let extra = serde_json::to_string(&Extra {
            params: safe.params,
            multipart: safe.multipart_fields,
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
        let history = Self::with_connection(Connection::open(dir.join("history.sqlite3"))?)?;
        history.scrub_credentials()?;
        Ok(history)
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
        Self::with_connection(Connection::open(path).unwrap()).unwrap()
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::with_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn with_connection(conn: Connection) -> rusqlite::Result<Self> {
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
        ] {
            if !columns.iter().any(|c| c == column) {
                conn.execute(&format!("ALTER TABLE requests ADD COLUMN {definition}"), [])?;
            }
        }
        Ok(Self { conn })
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

    /// Records a sent request, noting who sent it. Returns the new row's id.
    pub fn insert_from(
        &self,
        state: &PersistedState,
        status: Option<u16>,
        elapsed_ms: Option<u128>,
        source: Source,
    ) -> rusqlite::Result<i64> {
        let id = self.insert_row(state, status, elapsed_ms, None, source)?;
        // Only unnamed history is pruned; saved requests are kept however many there are.
        self.conn.execute(
            "DELETE FROM requests WHERE name IS NULL AND id NOT IN
                (SELECT id FROM requests WHERE name IS NULL ORDER BY id DESC LIMIT ?1)",
            params![MAX_ROWS],
        )?;
        Ok(id)
    }

    /// Saves a request under `name` without adding it to the history list.
    pub fn save_new(&self, state: &PersistedState, name: &str) -> rusqlite::Result<i64> {
        self.insert_row(state, None, None, Some(name), Source::Gui)
    }

    /// Saves a request under `name`, noting who saved it.
    pub fn save_new_from(&self, state: &PersistedState, name: &str, source: Source) -> rusqlite::Result<i64> {
        self.insert_row(state, None, None, Some(name), source)
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
        source: Source,
    ) -> rusqlite::Result<i64> {
        let created_at = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let row = StoredRequest::from(state);
        self.conn.execute(
            "INSERT INTO requests
                (created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body,
                 status, elapsed_ms, extra_json, name, in_history, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                created_at,
                row.method,
                row.url,
                row.headers_text,
                row.body_mode,
                row.json_body,
                row.urlencoded_body,
                row.raw_body,
                status.map(|s| s as i64),
                elapsed_ms.map(|e| e as i64),
                row.extra,
                name,
                // A request saved directly (not from a send) isn't part of the history.
                name.is_none(),
                source.as_str(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Overwrites a saved request's contents with `state`. Returns false if
    /// the row no longer exists.
    pub fn update_request(&self, id: i64, state: &PersistedState) -> rusqlite::Result<bool> {
        let row = StoredRequest::from(state);
        let changed = self.conn.execute(
            "UPDATE requests SET method = ?1, url = ?2, headers_text = ?3, body_mode = ?4, json_body = ?5,
                urlencoded_body = ?6, raw_body = ?7, extra_json = ?8
             WHERE id = ?9",
            params![
                row.method,
                row.url,
                row.headers_text,
                row.body_mode,
                row.json_body,
                row.urlencoded_body,
                row.raw_body,
                row.extra,
                id
            ],
        )?;
        Ok(changed > 0)
    }

    /// Names (saves) a row, or with `None` un-saves it. A saved request that is
    /// no longer in the history list has nothing left to show, so it's deleted.
    pub fn set_name(&self, id: i64, name: Option<&str>) -> rusqlite::Result<()> {
        match name {
            Some(name) => {
                self.conn.execute("UPDATE requests SET name = ?1 WHERE id = ?2", params![name, id])?;
            }
            None => {
                self.conn.execute("DELETE FROM requests WHERE id = ?1 AND in_history = 0", params![id])?;
                self.conn.execute("UPDATE requests SET name = NULL WHERE id = ?1", params![id])?;
            }
        }
        Ok(())
    }

    /// Saved requests, alphabetically.
    pub fn list_saved(&self) -> rusqlite::Result<Vec<HistoryEntry>> {
        self.query(
            &format!("SELECT {COLUMNS} FROM requests WHERE name IS NOT NULL ORDER BY name COLLATE NOCASE, id"),
            [],
        )
    }

    /// One request (history or saved) by id.
    pub fn get(&self, id: i64) -> rusqlite::Result<Option<HistoryEntry>> {
        Ok(self.query(&format!("SELECT {COLUMNS} FROM requests WHERE id = ?1"), params![id])?.pop())
    }

    /// Sent requests, newest first. Saved ones appear here too (with their
    /// name) until the history is cleared.
    pub fn list_recent(&self, limit: i64) -> rusqlite::Result<Vec<HistoryEntry>> {
        self.query(
            &format!("SELECT {COLUMNS} FROM requests WHERE in_history = 1 ORDER BY id DESC LIMIT ?1"),
            params![limit],
        )
    }

    fn query(&self, sql: &str, args: impl rusqlite::Params) -> rusqlite::Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(args, |row| {
            let extra: Extra = serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default();
            Ok(HistoryEntry {
                params: extra.params,
                multipart_fields: extra.multipart,
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
            })
        })?;
        rows.collect()
    }

    /// Empties the history list. Saved requests are kept (and stay in the
    /// Saved list); they just stop appearing under History.
    pub fn clear(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM requests WHERE name IS NULL", [])?;
        self.conn.execute("UPDATE requests SET in_history = 0", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> History {
        History::with_connection(Connection::open_in_memory().unwrap()).unwrap()
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

        let rows = h.list_recent(10).unwrap();
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
    fn list_recent_honours_limit_and_clear_empties() {
        let h = history();
        for i in 0..5 {
            h.insert(&state(&format!("http://x/{i}"), BodyMode::None), Some(200), Some(1)).unwrap();
        }
        assert_eq!(h.list_recent(3).unwrap().len(), 3);
        h.clear().unwrap();
        assert!(h.list_recent(10).unwrap().is_empty());
    }

    #[test]
    fn credentials_never_reach_the_database() {
        let h = history();
        let mut s = state("https://bob:pw@a.com/x?token=T&page=1", BodyMode::None);
        s.headers_text = "Accept: */*\nAuthorization: Bearer SECRET\nCookie: sid=1".into();
        h.insert(&s, Some(200), Some(1)).unwrap();

        let row = &h.list_recent(1).unwrap()[0];
        assert_eq!(row.url, "https://bob@a.com/x?token=&page=1");
        assert_eq!(row.headers_text, "Accept: */*\nAuthorization:\nCookie:");
        assert!(!format!("{} {}", row.url, row.headers_text).contains("SECRET"));
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
        let row = &h.list_recent(1).unwrap()[0];
        assert_eq!(row.url, "https://a.com/?api_key=");
        assert_eq!(row.headers_text, "Authorization:");
    }

    #[test]
    fn params_and_multipart_round_trip_with_secrets_blanked() {
        use crate::model::FieldKind;
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

        let restored = h.list_recent(1).unwrap()[0].to_persisted_state();
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

        let h = History::with_connection(conn).unwrap();
        let rows = h.list_recent(10).unwrap();
        assert_eq!(rows[0].url, "http://legacy");
        assert!(rows[0].params.is_empty() && rows[0].multipart_fields.is_empty());
        // opening again must not try to add the column twice
        let h2 = History::with_connection(h.conn).unwrap();
        h2.insert(&state("http://new", BodyMode::None), None, None).unwrap();
    }

    #[test]
    fn old_rows_are_pruned_beyond_the_cap() {
        let h = history();
        for i in 0..(MAX_ROWS + 25) {
            h.insert(&state(&format!("http://x/{i}"), BodyMode::None), Some(200), Some(1)).unwrap();
        }
        let rows = h.list_recent(MAX_ROWS + 100).unwrap();
        assert_eq!(rows.len() as i64, MAX_ROWS);
        assert_eq!(rows[0].url, format!("http://x/{}", MAX_ROWS + 24));
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
        assert_eq!(h.list_recent(10).unwrap().iter().filter(|r| r.name.is_some()).count(), 1);

        h.clear().unwrap();
        assert!(h.list_recent(10).unwrap().is_empty());
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
        let rows = h.list_recent(10).unwrap();
        assert_eq!((rows.len(), rows[0].name.clone()), (1, None));
    }

    #[test]
    fn a_saved_request_is_updated_in_place_and_not_listed_as_history() {
        let h = history();
        let id = h.save_new(&state("http://v1", BodyMode::None), "Mine").unwrap();
        assert!(h.list_recent(10).unwrap().is_empty());
        let mut s = state("http://v2?token=T", BodyMode::Json);
        s.headers_text = "Authorization: Bearer X".into();
        assert!(h.update_request(id, &s).unwrap());
        let row = &h.list_saved().unwrap()[0];
        assert_eq!((row.url.as_str(), row.headers_text.as_str()), ("http://v2?token=", "Authorization:"));
        assert!(!h.update_request(9999, &s).unwrap());
    }

    #[test]
    fn saved_requests_are_never_pruned() {
        let h = history();
        let keep = h.insert(&state("http://keep", BodyMode::None), None, None).unwrap();
        h.set_name(keep, Some("Keep")).unwrap();
        for i in 0..(MAX_ROWS + 5) {
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
        let rows = h.list_recent(10).unwrap();
        assert_eq!((rows[0].url.as_str(), rows[0].source), ("http://agent", Source::Mcp));
        assert_eq!(rows[1].source, Source::Gui);
    }

    #[test]
    fn a_write_from_another_connection_changes_the_data_version() {
        let dir = std::env::temp_dir().join(format!("plunger-dv-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("h.sqlite3");
        let _ = std::fs::remove_file(&path);
        let window = History::with_connection(Connection::open(&path).unwrap()).unwrap();
        let agent = History::with_connection(Connection::open(&path).unwrap()).unwrap();
        let before = window.data_version();
        agent.insert_from(&state("http://x", BodyMode::None), Some(200), Some(1), Source::Cli).unwrap();
        assert_ne!(window.data_version(), before);
        assert_eq!(window.list_recent(5).unwrap().len(), 1);
    }

    #[test]
    fn body_mode_string_round_trip() {
        for m in [BodyMode::None, BodyMode::Json, BodyMode::UrlEncoded, BodyMode::Raw] {
            assert!(body_mode_from_str(body_mode_to_str(m)) == m);
        }
        assert!(body_mode_from_str("garbage") == BodyMode::None);
    }
}
