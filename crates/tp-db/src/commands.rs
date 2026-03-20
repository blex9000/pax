use anyhow::Result;

use crate::Database;

#[derive(Debug)]
pub struct CommandRecord {
    pub id: i64,
    pub workspace_name: Option<String>,
    pub panel_id: Option<String>,
    pub command: String,
    pub executed_at: String,
    pub exit_code: Option<i32>,
}

impl Database {
    /// Record a command execution.
    pub fn insert_command(
        &self,
        workspace_name: Option<&str>,
        panel_id: Option<&str>,
        command: &str,
        exit_code: Option<i32>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO command_history (workspace_name, panel_id, command, exit_code) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![workspace_name, panel_id, command, exit_code],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Search commands using FTS5.
    pub fn search_commands(&self, query: &str, limit: usize) -> Result<Vec<CommandRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT ch.id, ch.workspace_name, ch.panel_id, ch.command, ch.executed_at, ch.exit_code
             FROM command_history ch
             JOIN command_history_fts fts ON ch.id = fts.rowid
             WHERE command_history_fts MATCH ?1
             ORDER BY ch.executed_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![query, limit as i64], |row| {
            Ok(CommandRecord {
                id: row.get(0)?,
                workspace_name: row.get(1)?,
                panel_id: row.get(2)?,
                command: row.get(3)?,
                executed_at: row.get(4)?,
                exit_code: row.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get recent commands.
    pub fn recent_commands(&self, limit: usize) -> Result<Vec<CommandRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_name, panel_id, command, executed_at, exit_code
             FROM command_history ORDER BY executed_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(CommandRecord {
                id: row.get(0)?,
                workspace_name: row.get(1)?,
                panel_id: row.get(2)?,
                command: row.get(3)?,
                executed_at: row.get(4)?,
                exit_code: row.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
