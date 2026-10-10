//! Workflows and the record of their runs.

use super::*;

/// How many runs of each workflow are kept.
pub const WORKFLOW_RUNS_KEPT: i64 = 20;

/// A workflow run to record.
pub struct NewWorkflowRun<'a> {
    pub workflow: &'a str,
    pub started_at: &'a str,
    pub source: Source,
    pub ok: bool,
    pub cancelled: bool,
    /// The per-step results as JSON.
    pub steps: &'a str,
}

/// A recorded workflow run.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRunRow {
    pub id: i64,
    pub workflow: String,
    pub started_at: String,
    pub source: Source,
    pub ok: bool,
    pub cancelled: bool,
    /// The per-step results as JSON.
    pub steps: String,
}

fn workflow_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRunRow> {
    Ok(WorkflowRunRow {
        id: row.get(0)?,
        workflow: row.get(1)?,
        started_at: row.get(2)?,
        source: Source::parse(&row.get::<_, String>(3)?),
        ok: row.get(4)?,
        cancelled: row.get(5)?,
        steps: row.get(6)?,
    })
}

impl History {
    /// Creates or replaces a workflow; `steps` is its JSON.
    pub fn save_workflow(&self, name: &str, steps: &str) -> rusqlite::Result<()> {
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        self.conn.execute(
            "INSERT INTO workflows (name, steps, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET steps = ?2, updated_at = ?3",
            params![name, steps, now],
        )?;
        Ok(())
    }

    /// Records one run of a workflow, keeping the newest `WORKFLOW_RUNS_KEPT` of each workflow.
    pub fn record_workflow_run(&self, run: &NewWorkflowRun) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO workflow_runs (workflow, started_at, source, ok, cancelled, steps) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![run.workflow, run.started_at, run.source.as_str(), run.ok, run.cancelled, run.steps],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute(
            "DELETE FROM workflow_runs WHERE workflow = ?1 AND id NOT IN
               (SELECT id FROM workflow_runs WHERE workflow = ?1 ORDER BY id DESC LIMIT ?2)",
            params![run.workflow, WORKFLOW_RUNS_KEPT],
        )?;
        Ok(id)
    }

    /// The runs of one workflow, newest first.
    pub fn workflow_runs(&self, workflow: &str, limit: i64) -> rusqlite::Result<Vec<WorkflowRunRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workflow, started_at, source, ok, cancelled, steps FROM workflow_runs WHERE workflow = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![workflow, limit], workflow_run_from_row)?;
        rows.collect()
    }

    /// The newest run of every workflow that has one.
    pub fn latest_workflow_runs(&self) -> rusqlite::Result<Vec<WorkflowRunRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workflow, started_at, source, ok, cancelled, steps FROM workflow_runs
             WHERE id IN (SELECT MAX(id) FROM workflow_runs GROUP BY workflow)",
        )?;
        let rows = stmt.query_map([], workflow_run_from_row)?;
        rows.collect()
    }

    /// A workflow's steps (JSON), if there is one by that name.
    pub fn get_workflow(&self, name: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT steps FROM workflows WHERE name = ?1", params![name], |r| r.get(0))
            .optional()
    }

    /// Workflows by name, with their steps (JSON).
    pub fn list_workflows(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT name, steps FROM workflows ORDER BY name COLLATE NOCASE, name")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Removes a workflow and its recorded runs; false when there was none by that name.
    pub fn delete_workflow(&self, name: &str) -> rusqlite::Result<bool> {
        self.conn.execute("DELETE FROM workflow_runs WHERE workflow = ?1", params![name])?;
        Ok(self.conn.execute("DELETE FROM workflows WHERE name = ?1", params![name])? > 0)
    }
}
