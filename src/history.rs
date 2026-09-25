use crate::model::{BodyMode, FormField, KeyValue, PersistedState};
use crate::redact::{redact_headers_text, redact_url};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Oldest rows beyond this are pruned on insert so the database can't grow forever.
const MAX_ROWS: i64 = 1000;

/// Per-user application data directory (history database, crash log).
pub fn app_data_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "Metamug API Tester")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[derive(Clone)]
pub struct HistoryEntry {
    pub id: i64,
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

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::with_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn with_connection(conn: Connection) -> rusqlite::Result<Self> {
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
        let has_extra = conn
            .prepare("PRAGMA table_info(requests)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .any(|name| name.is_ok_and(|n| n == "extra_json"));
        if !has_extra {
            conn.execute("ALTER TABLE requests ADD COLUMN extra_json TEXT NOT NULL DEFAULT ''", [])?;
        }
        Ok(Self { conn })
    }

    pub fn insert(
        &self,
        state: &PersistedState,
        status: Option<u16>,
        elapsed_ms: Option<u128>,
    ) -> rusqlite::Result<()> {
        let created_at = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        // Same credential rules as the saved form state: blank secrets before they hit disk.
        let safe = state.redacted();
        let extra = serde_json::to_string(&Extra {
            params: safe.params,
            multipart: safe.multipart_fields,
        })
        .unwrap_or_default();
        self.conn.execute(
            "INSERT INTO requests
                (created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body, status, elapsed_ms, extra_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                created_at,
                state.method,
                redact_url(&state.url),
                safe.headers_text,
                body_mode_to_str(state.body_mode),
                state.json_body,
                state.urlencoded_body,
                state.raw_body,
                status.map(|s| s as i64),
                elapsed_ms.map(|e| e as i64),
                extra,
            ],
        )?;
        self.conn.execute(
            "DELETE FROM requests WHERE id NOT IN (SELECT id FROM requests ORDER BY id DESC LIMIT ?1)",
            params![MAX_ROWS],
        )?;
        Ok(())
    }

    pub fn list_recent(&self, limit: i64) -> rusqlite::Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body, status, elapsed_ms, extra_json
             FROM requests ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
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
            })
        })?;
        rows.collect()
    }

    pub fn clear(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM requests", [])?;
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
    fn body_mode_string_round_trip() {
        for m in [BodyMode::None, BodyMode::Json, BodyMode::UrlEncoded, BodyMode::Raw] {
            assert!(body_mode_from_str(body_mode_to_str(m)) == m);
        }
        assert!(body_mode_from_str("garbage") == BodyMode::None);
    }
}
