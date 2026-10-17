//! The audit log: a write-ahead log of every change to the history database.
//!
//! The log is `audit.log` beside the database, one JSON object per line. Each line is one
//! [`Event`] with a sequence number, a time, the hash of the line before it and its own hash, so a
//! line that is edited, removed or reordered breaks the chain from there on. The database is the
//! result of applying the events in order: [`apply`] is the one place that turns an event into SQL,
//! and replaying the log into an empty database rebuilds it.
//!
//! What goes in is what the database already holds, in the same redacted form: credentials
//! blanked, `{{placeholders}}` kept, no response bodies, and no secret variable values.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// The format of the events; bumped when an event changes shape.
pub const FORMAT: u32 = 1;

/// The `prev` of the first line of a log.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Oldest history rows beyond this are pruned as new ones are added.
pub const MAX_HISTORY_ROWS: i64 = 1000;

/// How many runs of each workflow are kept.
pub const WORKFLOW_RUNS_KEPT: i64 = 20;

/// A request as the database stores it (credentials already blanked).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct RequestRow {
    pub method: String,
    pub url: String,
    pub headers_text: String,
    pub body_mode: String,
    pub json_body: String,
    pub urlencoded_body: String,
    pub raw_body: String,
    /// The JSON blob of params, multipart fields, options and the values it was sent with.
    pub extra: String,
}

/// One row of the `requests` table, whole: used by a snapshot.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RequestFull {
    pub id: i64,
    pub created_at: String,
    pub source: String,
    pub name: Option<String>,
    pub in_history: bool,
    pub status: Option<i64>,
    pub elapsed_ms: Option<i64>,
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub row: RequestRow,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WorkflowFull {
    pub name: String,
    pub steps: String,
    pub updated_at: String,
    pub group: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RunFull {
    pub id: i64,
    pub workflow: String,
    pub started_at: String,
    pub source: String,
    pub ok: bool,
    pub cancelled: bool,
    pub steps: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VariableFull {
    pub name: String,
    /// Empty for a secret: its value lives in the credential store.
    pub value: String,
    pub secret: bool,
    pub source: String,
    pub updated_at: String,
}

/// The whole state of the database at one moment: the first entry of a log that starts on an existing
/// database, so the chain begins at something verifiable and nothing is lost.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The hash of the last entry of the log file this one continues (after a rotation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continues: Option<String>,
    pub requests: Vec<RequestFull>,
    pub workflows: Vec<WorkflowFull>,
    pub runs: Vec<RunFull>,
    pub variables: Vec<VariableFull>,
}

/// A change to the database.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Event {
    /// A request was sent (`in_history`) or saved under a name.
    RequestAdded {
        id: i64,
        created_at: String,
        source: String,
        name: Option<String>,
        in_history: bool,
        status: Option<i64>,
        elapsed_ms: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        row: RequestRow,
    },
    /// A saved request's form was saved again.
    RequestUpdated { id: i64, row: RequestRow },
    /// A saved request's options (timeout, select, ...) or the values it was sent with changed.
    RequestExtraSet { id: i64, extra: String },
    /// A request was named (saved) or unnamed (removed from Saved).
    RequestNamed { id: i64, name: Option<String> },
    RequestGrouped { id: i64, group: Option<String> },
    /// The history list was cleared (saved requests stay).
    HistoryCleared,
    VariableSet { name: String, value: String, secret: bool, source: String, updated_at: String },
    VariableDeleted { name: String },
    VariablesCleared,
    WorkflowSaved { name: String, steps: String, updated_at: String },
    WorkflowGrouped { name: String, group: Option<String> },
    WorkflowDeleted { name: String },
    WorkflowRunRecorded { id: i64, workflow: String, started_at: String, source: String, ok: bool, cancelled: bool, steps: String },
    Snapshot(Snapshot),
}

impl Event {
    /// A short name for listings.
    pub fn action(&self) -> &'static str {
        match self {
            Event::RequestAdded { in_history: true, .. } => "send",
            Event::RequestAdded { .. } => "request_saved",
            Event::RequestUpdated { .. } => "request_updated",
            Event::RequestExtraSet { .. } => "request_options",
            Event::RequestNamed { name: Some(_), .. } => "request_named",
            Event::RequestNamed { .. } => "request_unsaved",
            Event::RequestGrouped { .. } => "request_grouped",
            Event::HistoryCleared => "history_cleared",
            Event::VariableSet { .. } => "variable_set",
            Event::VariableDeleted { .. } => "variable_deleted",
            Event::VariablesCleared => "variables_cleared",
            Event::WorkflowSaved { .. } => "workflow_saved",
            Event::WorkflowGrouped { .. } => "workflow_grouped",
            Event::WorkflowDeleted { .. } => "workflow_deleted",
            Event::WorkflowRunRecorded { .. } => "workflow_run",
            Event::Snapshot(_) => "snapshot",
        }
    }
}

impl Event {
    /// Who made the change (`gui`, `cli`, `mcp`), for the events that say.
    pub fn source(&self) -> Option<&str> {
        match self {
            Event::RequestAdded { source, .. } | Event::VariableSet { source, .. } | Event::WorkflowRunRecorded { source, .. } => Some(source),
            _ => None,
        }
    }

    /// One line saying what happened, for a listing. No bodies, no values.
    pub fn summary(&self) -> String {
        fn cut(text: &str) -> String {
            let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
            if flat.chars().count() > 90 {
                format!("{}...", flat.chars().take(90).collect::<String>())
            } else {
                flat
            }
        }
        match self {
            Event::RequestAdded { id, name, status, row, .. } => {
                let outcome = status.map(|s| format!(" -> {s}")).unwrap_or_default();
                let saved = name.as_ref().map(|n| format!(" saved as \"{n}\"")).unwrap_or_default();
                format!("#{id} {} {}{outcome}{saved}", row.method, cut(&row.url))
            }
            Event::RequestUpdated { id, row } => format!("#{id} now {} {}", row.method, cut(&row.url)),
            Event::RequestExtraSet { id, .. } => format!("#{id} options changed"),
            Event::RequestNamed { id, name: Some(name) } => format!("#{id} named \"{name}\""),
            Event::RequestNamed { id, name: None } => format!("#{id} removed from Saved"),
            Event::RequestGrouped { id, group } => format!("#{id} group {}", group.as_deref().unwrap_or("(none)")),
            Event::HistoryCleared => "history cleared".to_string(),
            Event::VariableSet { name, secret, .. } => format!("{name}{}", if *secret { " (secret)" } else { "" }),
            Event::VariableDeleted { name } => name.clone(),
            Event::VariablesCleared => "all agent variables removed".to_string(),
            Event::WorkflowSaved { name, .. } => name.clone(),
            Event::WorkflowGrouped { name, group } => format!("{name} group {}", group.as_deref().unwrap_or("(none)")),
            Event::WorkflowDeleted { name } => name.clone(),
            Event::WorkflowRunRecorded { workflow, ok, cancelled, .. } => {
                format!("{workflow}: {}", if *cancelled { "cancelled" } else if *ok { "passed" } else { "failed" })
            }
            Event::Snapshot(s) => format!(
                "{} requests, {} workflows, {} runs, {} variables{}",
                s.requests.len(),
                s.workflows.len(),
                s.runs.len(),
                s.variables.len(),
                s.note.as_ref().map(|n| format!(" ({n})")).unwrap_or_default()
            ),
        }
    }
}

/// One line of the log.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Entry {
    pub seq: u64,
    pub time: String,
    pub v: u32,
    pub prev: String,
    pub event: Event,
    pub hash: String,
}

/// The hash of an entry: over the previous hash and the entry's own content without its `hash`.
fn entry_hash(seq: u64, time: &str, v: u32, prev: &str, event: &Event) -> String {
    #[derive(Serialize)]
    struct Body<'a> {
        seq: u64,
        time: &'a str,
        v: u32,
        prev: &'a str,
        event: &'a Event,
    }
    let body = serde_json::to_string(&Body { seq, time, v, prev, event }).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    hasher.finalize().iter().fold(String::with_capacity(64), |mut out, b| {
        out.push_str(&format!("{b:02x}"));
        out
    })
}

impl Entry {
    fn new(seq: u64, prev: &str, event: Event) -> Entry {
        let time = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let hash = entry_hash(seq, &time, FORMAT, prev, &event);
        Entry { seq, time, v: FORMAT, prev: prev.to_string(), event, hash }
    }

    /// Whether the stored hash is the hash of the content.
    pub fn is_sound(&self) -> bool {
        entry_hash(self.seq, &self.time, self.v, &self.prev, &self.event) == self.hash
    }
}

/// Where the log stands: the last entry written, how many bytes of the file are applied, and the last
/// ids handed out (so ids are never reused).
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub seq: u64,
    pub hash: String,
    pub offset: u64,
    pub last_request_id: i64,
    pub last_run_id: i64,
}

impl State {
    pub fn fresh() -> State {
        State { seq: 0, hash: GENESIS.to_string(), offset: 0, last_request_id: 0, last_run_id: 0 }
    }
}

pub fn ensure_state_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS log_state (
            id              INTEGER PRIMARY KEY CHECK (id = 1),
            seq             INTEGER NOT NULL,
            hash            TEXT NOT NULL,
            offset          INTEGER NOT NULL,
            last_request_id INTEGER NOT NULL,
            last_run_id     INTEGER NOT NULL
        )",
        [],
    )?;
    Ok(())
}

pub fn read_state(conn: &Connection) -> rusqlite::Result<Option<State>> {
    conn.query_row("SELECT seq, hash, offset, last_request_id, last_run_id FROM log_state WHERE id = 1", [], |r| {
        Ok(State { seq: r.get::<_, i64>(0)? as u64, hash: r.get(1)?, offset: r.get::<_, i64>(2)? as u64, last_request_id: r.get(3)?, last_run_id: r.get(4)? })
    })
    .optional()
}

pub fn write_state(conn: &Connection, s: &State) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO log_state (id, seq, hash, offset, last_request_id, last_run_id) VALUES (1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET seq = ?1, hash = ?2, offset = ?3, last_request_id = ?4, last_run_id = ?5",
        params![s.seq as i64, s.hash, s.offset as i64, s.last_request_id, s.last_run_id],
    )?;
    Ok(())
}

/// Appends `event` as the next line. The line is written in one `write` and synced before this returns.
pub fn append(path: &Path, state: &State, event: Event) -> std::io::Result<(Entry, u64)> {
    let entry = Entry::new(state.seq + 1, &state.hash, event);
    let mut line = serde_json::to_string(&entry).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.sync_data()?;
    let offset = file.metadata()?.len();
    Ok((entry, offset))
}

/// The entries from byte `offset` on, with the offset after each. A line that does not parse stops the read.
pub fn read_from(path: &Path, offset: u64) -> std::io::Result<Vec<(Entry, u64)>> {
    use std::io::Seek;
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    file.seek(std::io::SeekFrom::Start(offset))?;
    let mut reader = std::io::BufReader::new(file);
    let mut position = offset;
    let mut out = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        if !line.ends_with('\n') {
            break; // a line still being written
        }
        let Ok(entry) = serde_json::from_str::<Entry>(line.trim_end()) else { break };
        position += n as u64;
        out.push((entry, position));
    }
    Ok(out)
}

/// What is wrong with a log.
#[derive(Debug, PartialEq)]
pub struct Broken {
    /// The sequence number where the chain first fails (the line after the last good one).
    pub seq: u64,
    pub reason: String,
}

/// Walks the log and returns the number of entries and the head hash, or where the chain breaks.
pub fn verify(path: &Path) -> Result<(u64, String), Broken> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok((0, GENESIS.to_string())),
    };
    let reader = std::io::BufReader::new(file);
    let mut prev = GENESIS.to_string();
    let mut expected: Option<u64> = None;
    let mut count = 0u64;
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| Broken { seq: count + 1, reason: format!("unreadable: {e}") })?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: Entry = serde_json::from_str(&line)
            .map_err(|e| Broken { seq: expected.unwrap_or(count + 1), reason: format!("line {} is not an entry: {e}", index + 1) })?;
        if let Some(want) = expected {
            if entry.seq != want {
                return Err(Broken { seq: want, reason: format!("expected entry {want}, found {}: an entry was removed or reordered", entry.seq) });
            }
        }
        // the first line follows nothing, or is the snapshot that continues an archived file
        let continues_archive = count == 0 && matches!(&entry.event, Event::Snapshot(s) if s.continues.as_deref() == Some(entry.prev.as_str()));
        if entry.prev != prev && !continues_archive {
            return Err(Broken { seq: entry.seq, reason: "it does not follow the entry before it".to_string() });
        }
        if !entry.is_sound() {
            return Err(Broken { seq: entry.seq, reason: "its content does not match its hash: it was edited".to_string() });
        }
        prev = entry.hash.clone();
        expected = Some(entry.seq + 1);
        count += 1;
    }
    Ok((count, prev))
}

/// What applying an event changed.
#[derive(Debug, Clone, Default)]
pub struct Applied {
    /// Rows the event changed (0 when it named something that is not there).
    pub changed: usize,
}

/// Turns an event into SQL. This is the only code that writes the history tables after they exist, so
/// the log fully determines the database.
pub fn apply(conn: &Connection, event: &Event) -> rusqlite::Result<Applied> {
    let changed = match event {
        Event::RequestAdded { id, created_at, source, name, in_history, status, elapsed_ms, error, row } => {
            let n = conn.execute(
                "INSERT INTO requests
                    (id, created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body,
                     status, elapsed_ms, extra_json, name, in_history, source, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    id, created_at, row.method, row.url, row.headers_text, row.body_mode, row.json_body, row.urlencoded_body,
                    row.raw_body, status, elapsed_ms, row.extra, name, in_history, source, error
                ],
            )?;
            if name.is_none() {
                // only unnamed history is pruned; saved requests are kept however many there are
                conn.execute(
                    "DELETE FROM requests WHERE name IS NULL AND id NOT IN
                        (SELECT id FROM requests WHERE name IS NULL ORDER BY id DESC LIMIT ?1)",
                    params![MAX_HISTORY_ROWS],
                )?;
            }
            n
        }
        Event::RequestUpdated { id, row } => conn.execute(
            "UPDATE requests SET method = ?1, url = ?2, headers_text = ?3, body_mode = ?4, json_body = ?5,
                urlencoded_body = ?6, raw_body = ?7, extra_json = ?8
             WHERE id = ?9",
            params![row.method, row.url, row.headers_text, row.body_mode, row.json_body, row.urlencoded_body, row.raw_body, row.extra, id],
        )?,
        Event::RequestExtraSet { id, extra } => conn.execute("UPDATE requests SET extra_json = ?1 WHERE id = ?2", params![extra, id])?,
        Event::RequestNamed { id, name } => match name {
            Some(name) => conn.execute("UPDATE requests SET name = ?1 WHERE id = ?2", params![name, id])?,
            None => {
                let removed = conn.execute("DELETE FROM requests WHERE id = ?1 AND in_history = 0", params![id])?;
                let cleared = conn.execute("UPDATE requests SET name = NULL, group_name = NULL WHERE id = ?1", params![id])?;
                removed + cleared
            }
        },
        Event::RequestGrouped { id, group } => conn.execute("UPDATE requests SET group_name = ?1 WHERE id = ?2", params![group, id])?,
        Event::HistoryCleared => {
            let removed = conn.execute("DELETE FROM requests WHERE name IS NULL", [])?;
            removed + conn.execute("UPDATE requests SET in_history = 0", [])?
        }
        Event::VariableSet { name, value, secret, source, updated_at } => conn.execute(
            "INSERT INTO agent_variables (name, value, secret, source, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(name) DO UPDATE SET value = ?2, secret = ?3, source = ?4, updated_at = ?5",
            params![name, value, secret, source, updated_at],
        )?,
        Event::VariableDeleted { name } => conn.execute("DELETE FROM agent_variables WHERE name = ?1", params![name])?,
        Event::VariablesCleared => conn.execute("DELETE FROM agent_variables", [])?,
        Event::WorkflowSaved { name, steps, updated_at } => conn.execute(
            "INSERT INTO workflows (name, steps, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET steps = ?2, updated_at = ?3",
            params![name, steps, updated_at],
        )?,
        Event::WorkflowGrouped { name, group } => conn.execute("UPDATE workflows SET group_name = ?1 WHERE name = ?2", params![group, name])?,
        Event::WorkflowDeleted { name } => {
            conn.execute("DELETE FROM workflow_runs WHERE workflow = ?1", params![name])?;
            conn.execute("DELETE FROM workflows WHERE name = ?1", params![name])?
        }
        Event::WorkflowRunRecorded { id, workflow, started_at, source, ok, cancelled, steps } => {
            let n = conn.execute(
                "INSERT INTO workflow_runs (id, workflow, started_at, source, ok, cancelled, steps) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, workflow, started_at, source, ok, cancelled, steps],
            )?;
            conn.execute(
                "DELETE FROM workflow_runs WHERE workflow = ?1 AND id NOT IN
                   (SELECT id FROM workflow_runs WHERE workflow = ?1 ORDER BY id DESC LIMIT ?2)",
                params![workflow, WORKFLOW_RUNS_KEPT],
            )?;
            n
        }
        Event::Snapshot(snapshot) => {
            conn.execute("DELETE FROM requests", [])?;
            conn.execute("DELETE FROM workflows", [])?;
            conn.execute("DELETE FROM workflow_runs", [])?;
            conn.execute("DELETE FROM agent_variables", [])?;
            for r in &snapshot.requests {
                conn.execute(
                    "INSERT INTO requests
                        (id, created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body,
                         status, elapsed_ms, extra_json, name, in_history, source, group_name, error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                    params![
                        r.id, r.created_at, r.row.method, r.row.url, r.row.headers_text, r.row.body_mode, r.row.json_body,
                        r.row.urlencoded_body, r.row.raw_body, r.status, r.elapsed_ms, r.row.extra, r.name, r.in_history, r.source, r.group,
                        r.error
                    ],
                )?;
            }
            for w in &snapshot.workflows {
                conn.execute(
                    "INSERT INTO workflows (name, steps, updated_at, group_name) VALUES (?1, ?2, ?3, ?4)",
                    params![w.name, w.steps, w.updated_at, w.group],
                )?;
            }
            for r in &snapshot.runs {
                conn.execute(
                    "INSERT INTO workflow_runs (id, workflow, started_at, source, ok, cancelled, steps) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![r.id, r.workflow, r.started_at, r.source, r.ok, r.cancelled, r.steps],
                )?;
            }
            for v in &snapshot.variables {
                conn.execute(
                    "INSERT INTO agent_variables (name, value, secret, source, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![v.name, v.value, v.secret, v.source, v.updated_at],
                )?;
            }
            snapshot.requests.len() + snapshot.workflows.len() + snapshot.runs.len() + snapshot.variables.len()
        }
    };
    Ok(Applied { changed })
}

/// The whole current state of the tables, for a snapshot.
pub fn snapshot_of(conn: &Connection, note: Option<String>) -> rusqlite::Result<Snapshot> {
    let mut requests = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, created_at, method, url, headers_text, body_mode, json_body, urlencoded_body, raw_body,
                status, elapsed_ms, extra_json, name, in_history, source, group_name, error FROM requests ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(RequestFull {
            id: r.get(0)?,
            created_at: r.get(1)?,
            row: RequestRow {
                method: r.get(2)?,
                url: r.get(3)?,
                headers_text: r.get(4)?,
                body_mode: r.get(5)?,
                json_body: r.get(6)?,
                urlencoded_body: r.get(7)?,
                raw_body: r.get(8)?,
                extra: r.get(11)?,
            },
            status: r.get(9)?,
            elapsed_ms: r.get(10)?,
            name: r.get(12)?,
            in_history: r.get(13)?,
            source: r.get(14)?,
            group: r.get(15)?,
            error: r.get(16)?,
        })
    })?;
    for row in rows {
        requests.push(row?);
    }
    let workflows = conn
        .prepare("SELECT name, steps, updated_at, group_name FROM workflows ORDER BY name")?
        .query_map([], |r| Ok(WorkflowFull { name: r.get(0)?, steps: r.get(1)?, updated_at: r.get(2)?, group: r.get(3)? }))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let runs = conn
        .prepare("SELECT id, workflow, started_at, source, ok, cancelled, steps FROM workflow_runs ORDER BY id")?
        .query_map([], |r| {
            Ok(RunFull { id: r.get(0)?, workflow: r.get(1)?, started_at: r.get(2)?, source: r.get(3)?, ok: r.get(4)?, cancelled: r.get(5)?, steps: r.get(6)? })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let variables = conn
        .prepare("SELECT name, value, secret, source, updated_at FROM agent_variables ORDER BY name")?
        .query_map([], |r| {
            Ok(VariableFull { name: r.get(0)?, value: r.get(1)?, secret: r.get(2)?, source: r.get(3)?, updated_at: r.get(4)? })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Snapshot { note, continues: None, requests, workflows, runs, variables })
}

/// A log file is moved aside and a new one started when it passes this size.
pub const ROTATE_BYTES: u64 = 20 * 1024 * 1024;

/// Moves the log to `audit.<last seq>.log` beside it. The next entry starts the new file.
pub fn archive(path: &Path, last_seq: u64) -> std::io::Result<PathBuf> {
    let target = path.with_file_name(format!("audit.{last_seq}.log"));
    std::fs::rename(path, &target)?;
    Ok(target)
}

/// The log file that belongs to a database file.
pub fn path_for(database: &Path) -> PathBuf {
    // the real database is `history.sqlite3`, whose log is `audit.log`; any other database file (tests, a copy)
    // gets a log of its own beside it, so two databases never share one
    match database.file_name().and_then(|n| n.to_str()) {
        Some("history.sqlite3") => database.with_file_name("audit.log"),
        Some(name) => database.with_file_name(format!("{name}.audit.log")),
        None => database.with_file_name("audit.log"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{PersistedState, Variable};
    use crate::store::history::{History, NewWorkflowRun, SavedOptions, Source};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plunger-log-{}-{name}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("history.sqlite3")
    }

    fn lines(database: &Path) -> Vec<Entry> {
        read_from(&path_for(database), 0).unwrap().into_iter().map(|(e, _)| e).collect()
    }

    fn state(url: &str) -> PersistedState {
        PersistedState { url: url.into(), ..Default::default() }
    }

    #[test]
    fn a_new_database_starts_its_log_with_a_snapshot_and_every_change_adds_exactly_one_line() {
        let db = scratch("one-line-each");
        let h = History::open_at(&db);
        let count = |h: &History| {
            let _ = h;
            lines(&db).len()
        };
        assert_eq!(count(&h), 1, "the snapshot");
        assert_eq!(lines(&db)[0].event.action(), "snapshot");

        let mut expect = 1;
        let mut step = |label: &str, action: &str, do_it: &dyn Fn()| {
            do_it();
            expect += 1;
            let all = lines(&db);
            assert_eq!(all.len(), expect, "{label} should add one line");
            assert_eq!(all.last().unwrap().event.action(), action, "{label}");
        };
        let sent = std::cell::Cell::new(0);
        step("a send", "send", &|| sent.set(h.insert_from(&state("http://h/a"), Some(200), Some(5), Source::Mcp).unwrap()));
        let saved = std::cell::Cell::new(0);
        step("a save", "request_saved", &|| saved.set(h.save_new_from(&state("http://h/b"), "b", Source::Cli).unwrap()));
        step("an update", "request_updated", &|| assert!(h.update_request(saved.get(), &state("http://h/b2")).unwrap()));
        step("options", "request_options", &|| {
            assert!(h.set_saved_options(saved.get(), &SavedOptions { timeout_secs: Some(9), ..Default::default() }).unwrap())
        });
        step("a group", "request_grouped", &|| assert!(h.set_group(saved.get(), Some("Jira")).unwrap()));
        step("a name", "request_named", &|| h.set_name(sent.get(), Some("named")).unwrap());
        step("an unsave", "request_unsaved", &|| h.set_name(sent.get(), None).unwrap());
        step("a variable", "variable_set", &|| h.set_agent_variable("base", "http://h", false, Source::Mcp).unwrap());
        step("a variable removed", "variable_deleted", &|| assert!(h.delete_agent_variable("base").unwrap()));
        step("a second variable", "variable_set", &|| h.set_agent_variable("a", "1", false, Source::Mcp).unwrap());
        step("variables cleared", "variables_cleared", &|| {
            assert_eq!(h.clear_agent_variables().unwrap(), vec!["a".to_string()]);
        });
        step("a workflow", "workflow_saved", &|| h.save_workflow("flow", "[]").unwrap());
        step("a workflow group", "workflow_grouped", &|| assert!(h.set_workflow_group("flow", Some("Jira")).unwrap()));
        step("a run", "workflow_run", &|| {
            h.record_workflow_run(&NewWorkflowRun { workflow: "flow", started_at: "2026-10-14T10:00:00Z", source: Source::Gui, ok: true, cancelled: false, steps: "[]" }).unwrap();
        });
        step("a workflow deleted", "workflow_deleted", &|| assert!(h.delete_workflow("flow").unwrap()));
        step("history cleared", "history_cleared", &|| h.clear().unwrap());

        // things that change nothing write nothing
        let before = lines(&db).len();
        assert!(!h.set_group(99_999, Some("x")).unwrap());
        assert!(!h.update_request(99_999, &state("http://x")).unwrap());
        assert!(!h.delete_agent_variable("nope").unwrap());
        assert!(!h.set_workflow_group("nope", None).unwrap());
        assert!(h.clear_agent_variables().unwrap().is_empty());
        assert_eq!(lines(&db).len(), before);

        // and the whole chain is sound
        let (count, head) = verify(&path_for(&db)).unwrap();
        assert_eq!(count as usize, lines(&db).len());
        assert_eq!(head, lines(&db).last().unwrap().hash);
    }

    #[test]
    fn a_log_that_is_ahead_of_the_database_is_caught_up_at_open() {
        let db = scratch("recovery");
        {
            let h = History::open_at(&db);
            h.insert_from(&state("http://h/first"), Some(200), Some(1), Source::Gui).unwrap();
        }
        // a process that appended and stopped before the database changed: the line is there, the row is not
        let log = path_for(&db);
        let (last, _) = {
            let all = read_from(&log, 0).unwrap();
            all.last().cloned().unwrap()
        };
        let state_at = State { seq: last.seq, hash: last.hash.clone(), offset: std::fs::metadata(&log).unwrap().len(), last_request_id: 1, last_run_id: 0 };
        let orphan = Event::RequestAdded {
            id: 2,
            created_at: "2026-10-14T10:00:00Z".to_string(),
            source: "mcp".to_string(),
            name: None,
            in_history: true,
            status: Some(201),
            elapsed_ms: Some(7),
            error: None,
            row: RequestRow { method: "POST".into(), url: "http://h/orphan".into(), body_mode: "None".into(), extra: "{}".into(), ..Default::default() },
        };
        append(&log, &state_at, orphan).unwrap();

        let h = History::open_at(&db);
        let urls: Vec<String> = h.search_recent("", 10).unwrap().into_iter().map(|e| e.url).collect();
        assert!(urls.contains(&"http://h/orphan".to_string()), "{urls:?}");
        // the next row does not reuse the id
        let next = h.insert_from(&state("http://h/third"), Some(200), Some(1), Source::Gui).unwrap();
        assert_eq!(next, 3);
        assert!(verify(&log).is_ok());
    }

    #[test]
    fn deleting_the_database_and_opening_again_rebuilds_it_from_the_log() {
        let db = scratch("rebuild");
        let (saved, run) = {
            let h = History::open_at(&db);
            h.insert_from(&state("http://h/a"), Some(200), Some(1), Source::Mcp).unwrap();
            let saved = h.save_new(&state("http://h/{{id}}"), "get one").unwrap();
            h.set_group(saved, Some("Jira/Issues")).unwrap();
            h.set_saved_options(saved, &SavedOptions { timeout_secs: Some(30), select: vec!["$.id".into()], ..Default::default() }).unwrap();
            h.set_agent_variable("base", "http://h", false, Source::Cli).unwrap();
            h.save_workflow("flow", r#"[{"url": "http://h"}]"#).unwrap();
            h.set_workflow_group("flow", Some("Jira")).unwrap();
            let run = h.record_workflow_run(&NewWorkflowRun { workflow: "flow", started_at: "2026-10-14T10:00:00Z", source: Source::Cli, ok: true, cancelled: false, steps: "[]" }).unwrap();
            (saved, run)
        };
        for suffix in ["", "-wal", "-shm"] {
            let mut name = db.as_os_str().to_owned();
            name.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(name));
        }
        let h = History::open_at(&db);
        let entry = h.get(saved).unwrap().expect("the saved request is back, with the same id");
        assert_eq!((entry.name.as_deref(), entry.group.as_deref()), (Some("get one"), Some("Jira/Issues")));
        assert_eq!(entry.options.timeout_secs, Some(30));
        assert_eq!(h.list_saved().unwrap().len(), 1);
        assert_eq!(h.search_recent("", 10).unwrap().len(), 1, "the send is in the history; the saved one is not");
        assert_eq!(h.list_agent_variables().unwrap()[0].name, "base");
        assert_eq!(h.workflow_groups().unwrap().get("flow").map(String::as_str), Some("Jira"));
        assert_eq!(h.workflow_runs("flow", 10).unwrap()[0].id, run);
        // and writing goes on from where it was
        assert!(h.insert_from(&state("http://h/new"), Some(200), Some(1), Source::Gui).unwrap() > saved);
    }

    #[test]
    fn writers_in_different_connections_never_reuse_a_number_or_break_the_chain() {
        let db = scratch("concurrent");
        History::open_at(&db); // starts the log
        let threads: Vec<_> = (0..6)
            .map(|t| {
                let db = db.clone();
                std::thread::spawn(move || {
                    let h = History::open_at(&db);
                    let mut ids = Vec::new();
                    for i in 0..20 {
                        ids.push(h.insert_from(&state(&format!("http://h/{t}/{i}")), Some(200), Some(1), Source::Mcp).unwrap());
                    }
                    ids
                })
            })
            .collect();
        let mut ids: Vec<i64> = threads.into_iter().flat_map(|t| t.join().unwrap()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 120, "every row has its own id");

        let all = lines(&db);
        assert_eq!(all.len(), 1 + 120);
        let seqs: Vec<u64> = all.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, (1..=121).collect::<Vec<u64>>(), "no number twice, none missing");
        assert!(verify(&path_for(&db)).is_ok());
        let h = History::open_at(&db);
        assert_eq!(h.search_recent("", 500).unwrap().len(), 120);
    }

    #[test]
    fn an_edited_removed_or_reordered_line_is_found_and_named() {
        let db = scratch("tamper");
        let h = History::open_at(&db);
        for i in 0..4 {
            h.insert_from(&state(&format!("http://h/{i}")), Some(200), Some(1), Source::Mcp).unwrap();
        }
        let log = path_for(&db);
        let original = std::fs::read_to_string(&log).unwrap();
        let rows: Vec<&str> = original.lines().collect();
        assert_eq!(rows.len(), 5);
        let write = |lines: &[String]| std::fs::write(&log, lines.join("\n") + "\n").unwrap();
        let owned: Vec<String> = rows.iter().map(|l| l.to_string()).collect();

        // an edit: the status of entry 3 changed
        let mut edited = owned.clone();
        edited[2] = edited[2].replace("\"status\":200", "\"status\":500");
        write(&edited);
        let broken = verify(&log).unwrap_err();
        assert_eq!(broken.seq, 3, "{broken:?}");

        // a removal: entry 3 is gone
        let mut removed = owned.clone();
        removed.remove(2);
        write(&removed);
        assert_eq!(verify(&log).unwrap_err().seq, 3);

        // a reorder: entries 3 and 4 swapped
        let mut swapped = owned.clone();
        swapped.swap(2, 3);
        write(&swapped);
        assert_eq!(verify(&log).unwrap_err().seq, 3);

        // truncating the end is not a break in the chain, but the head no longer matches what was there
        write(&owned[..3]);
        let (count, head) = verify(&log).unwrap();
        assert_eq!(count, 3);
        assert_ne!(head, rows_hash(&owned));
        write(&owned);
        assert!(verify(&log).is_ok());
    }

    fn rows_hash(lines: &[String]) -> String {
        serde_json::from_str::<Entry>(lines.last().unwrap()).unwrap().hash
    }

    #[test]
    fn no_secret_value_credential_header_or_response_body_is_written() {
        let db = scratch("secrets");
        let h = History::open_at(&db);
        let state = PersistedState {
            url: "http://h/x?api_key=SUPERSECRETKEY&q=1".into(),
            headers_text: "Authorization: Bearer SUPERSECRETTOKEN\nX-Trace: {{trace}}".into(),
            variables: vec![Variable { name: "api_token".into(), value: "SUPERSECRETVAR".into(), secret: true, remember: false }],
            ..Default::default()
        };
        h.insert_from(&state, Some(200), Some(3), Source::Mcp).unwrap();
        h.save_new_from(&state, "with secrets", Source::Cli).unwrap();
        h.set_agent_variable("api_token", "", true, Source::Mcp).unwrap();
        let text = std::fs::read_to_string(path_for(&db)).unwrap();
        assert!(!text.contains("SUPERSECRET"), "a secret reached the log: {text}");
        assert!(text.contains("{{trace}}"), "placeholders are kept");
        assert!(text.contains("api_token"));
    }

    #[test]
    fn a_large_log_is_kept_aside_and_the_new_one_continues_the_chain() {
        let db = scratch("rotate");
        let h = History::open_at(&db).rotate_at(6_000);
        for i in 0..40 {
            h.insert_from(&state(&format!("http://h/{i}")), Some(200), Some(1), Source::Mcp).unwrap();
        }
        let log = path_for(&db);
        let dir = log.parent().unwrap();
        let archives: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("audit.") && n != "audit.log" && n.ends_with(".log") && !n.contains("sqlite3"))
            .collect();
        assert!(!archives.is_empty(), "the log was rotated: {archives:?}");

        // the current file verifies on its own because its first line says what it continues
        let (_, head) = verify(&log).unwrap();
        let first = lines(&db).remove(0);
        match &first.event {
            Event::Snapshot(s) => {
                assert!(s.continues.is_some() && s.note.as_deref().is_some_and(|n| n.contains("continues")));
            }
            other => panic!("{other:?}"),
        }
        // and so does every archive
        for name in &archives {
            assert!(verify(&dir.join(name)).is_ok(), "{name}");
        }
        // all rows are still there, and the database can be rebuilt from the files that remain
        assert_eq!(h.search_recent("", 100).unwrap().len(), 40);
        assert!(!head.is_empty());
        let before = h.search_recent("", 100).unwrap().len();
        h.rebuild_from_log().unwrap();
        assert_eq!(h.search_recent("", 100).unwrap().len(), before, "a snapshot carries what was before it");
    }
}
