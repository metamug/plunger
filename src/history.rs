use crate::model::{BodyMode, PersistedState};
use rusqlite::{params, Connection};

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
    pub status: Option<i64>,
    pub elapsed_ms: Option<i64>,
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
        }
    }
}

fn body_mode_to_str(mode: BodyMode) -> &'static str {
    match mode {
        BodyMode::None => "None",
        BodyMode::Json => "Json",
        BodyMode::UrlEncoded => "UrlEncoded",
        BodyMode::Raw => "Raw",
    }
}

fn body_mode_from_str(s: &str) -> BodyMode {
    match s {
        "Json" => BodyMode::Json,
        "UrlEncoded" => BodyMode::UrlEncoded,
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
        let path = directories::ProjectDirs::from("", "", "Metamug API Tester")
            .map(|dirs| dirs.data_dir().join("history.sqlite3"))
            .unwrap_or_else(|| std::path::PathBuf::from("history.sqlite3"));

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        Self::with_connection(Connection::open(path)?)
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
        self.conn.execute(
            "INSERT INTO requests
                (created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body, status, elapsed_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                created_at,
                state.method,
                state.url,
                state.headers_text,
                body_mode_to_str(state.body_mode),
                state.json_body,
                state.urlencoded_body,
                state.raw_body,
                status.map(|s| s as i64),
                elapsed_ms.map(|e| e as i64),
            ],
        )?;
        Ok(())
    }

    pub fn list_recent(&self, limit: i64) -> rusqlite::Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body, status, elapsed_ms
             FROM requests ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(HistoryEntry {
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
    fn body_mode_string_round_trip() {
        for m in [BodyMode::None, BodyMode::Json, BodyMode::UrlEncoded, BodyMode::Raw] {
            assert!(body_mode_from_str(body_mode_to_str(m)) == m);
        }
        assert!(body_mode_from_str("garbage") == BodyMode::None);
    }
}
