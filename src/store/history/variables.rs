//! Variables an agent set from the command line or over MCP.

use super::*;

/// A variable set from the command line or over MCP.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentVariable {
    pub name: String,
    /// Empty for a secret: its value is in the credential store.
    pub value: String,
    pub secret: bool,
    /// Who set it: `cli` or `mcp`.
    pub source: String,
    pub updated_at: String,
}

impl History {
    /// Creates or updates a variable set by an agent. A secret's value must not be passed here
    /// (it goes to the credential store); only the fact that it exists is recorded.
    pub fn set_agent_variable(&self, name: &str, value: &str, secret: bool, source: Source) -> rusqlite::Result<()> {
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        self.conn.execute(
            "INSERT INTO agent_variables (name, value, secret, source, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(name) DO UPDATE SET value = ?2, secret = ?3, source = ?4, updated_at = ?5",
            params![name, value, secret, source.as_str(), now],
        )?;
        Ok(())
    }

    /// Variables set by agents, by name.
    pub fn list_agent_variables(&self) -> rusqlite::Result<Vec<AgentVariable>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, value, secret, source, updated_at FROM agent_variables ORDER BY name COLLATE NOCASE, name")?;
        let rows = stmt.query_map([], |r| {
            Ok(AgentVariable { name: r.get(0)?, value: r.get(1)?, secret: r.get(2)?, source: r.get(3)?, updated_at: r.get(4)? })
        })?;
        rows.collect()
    }

    /// Removes one agent variable; false when there was none by that name.
    pub fn delete_agent_variable(&self, name: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.execute("DELETE FROM agent_variables WHERE name = ?1", params![name])? > 0)
    }

    /// Removes every agent variable and returns their names (so the caller can forget secrets).
    pub fn clear_agent_variables(&self) -> rusqlite::Result<Vec<String>> {
        let names: Vec<String> = self.list_agent_variables()?.into_iter().map(|v| v.name).collect();
        self.conn.execute("DELETE FROM agent_variables", [])?;
        Ok(names)
    }
}
