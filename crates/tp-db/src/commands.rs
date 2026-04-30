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

    /// Full command history for a given panel UUID, ordered by most
    /// recent execution. Sibling of `latest_distinct_commands` for
    /// callers that want to see every individual run rather than only
    /// the latest occurrence of each unique command.
    pub fn recent_commands_for_panel(
        &self,
        panel_uuid: &str,
        limit: usize,
    ) -> Result<Vec<CommandRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, workspace_name, panel_id, command, executed_at, exit_code \
             FROM command_history \
             WHERE panel_id = ?1 \
             ORDER BY executed_at DESC \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![panel_uuid, limit as i64], |row| {
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

    /// Last distinct commands for a given panel UUID, deduplicated by
    /// command text and ordered by the most recent execution. Used by
    /// the terminal panel "command history" popup.
    pub fn latest_distinct_commands(
        &self,
        panel_uuid: &str,
        limit: usize,
    ) -> Result<Vec<CommandRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT MIN(id), workspace_name, panel_id, command, \
                    MAX(executed_at) AS last_run, exit_code \
             FROM command_history \
             WHERE panel_id = ?1 \
             GROUP BY command \
             ORDER BY last_run DESC \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![panel_uuid, limit as i64], |row| {
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

    /// Remove all command history rows for a given panel UUID. Called
    /// when the panel is permanently closed to avoid leaving orphan rows.
    pub fn delete_command_history_for_panel(&self, panel_uuid: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM command_history WHERE panel_id = ?1",
            rusqlite::params![panel_uuid],
        )?;
        Ok(n)
    }

    /// Delete a single command_history row by id. Used by the popover's
    /// per-row trash icon.
    pub fn delete_command_history_row(&self, row_id: i64) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM command_history WHERE id = ?1",
            rusqlite::params![row_id],
        )?;
        Ok(n)
    }

    /// Pin a command (add to favourites). No-op when already pinned.
    pub fn pin_command(&self, panel_uuid: &str, command: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO pinned_commands (panel_uuid, command) VALUES (?1, ?2)",
            rusqlite::params![panel_uuid, command],
        )?;
        Ok(())
    }

    /// Remove a command from favourites. Returns the number of rows
    /// removed (0 when the command was not pinned).
    pub fn unpin_command(&self, panel_uuid: &str, command: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM pinned_commands WHERE panel_uuid = ?1 AND command = ?2",
            rusqlite::params![panel_uuid, command],
        )?;
        Ok(n)
    }

    /// Rename an existing pinned command (favourites tab inline edit).
    /// Returns `false` when there is no pinned row with that exact id.
    pub fn update_pinned_command(&self, id: i64, new_command: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE pinned_commands SET command = ?2 WHERE id = ?1",
            rusqlite::params![id, new_command],
        )?;
        Ok(n > 0)
    }

    /// All pinned commands for a panel, newest pin first.
    pub fn pinned_commands_for_panel(
        &self,
        panel_uuid: &str,
        limit: usize,
    ) -> Result<Vec<PinnedCommand>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, panel_uuid, command, created_at \
             FROM pinned_commands \
             WHERE panel_uuid = ?1 \
             ORDER BY created_at DESC \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![panel_uuid, limit as i64], |row| {
            Ok(PinnedCommand {
                id: row.get(0)?,
                panel_uuid: row.get(1)?,
                command: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Set of pinned command strings for a panel — useful for marking
    /// rows in the History tab without N+1 queries.
    pub fn pinned_command_set_for_panel(
        &self,
        panel_uuid: &str,
    ) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT command FROM pinned_commands WHERE panel_uuid = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![panel_uuid], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Drop every pinned command for a panel. Mirrors
    /// `delete_command_history_for_panel` so `on_permanent_close` can
    /// clean up favourites too.
    pub fn delete_pinned_for_panel(&self, panel_uuid: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM pinned_commands WHERE panel_uuid = ?1",
            rusqlite::params![panel_uuid],
        )?;
        Ok(n)
    }
}

#[derive(Debug, Clone)]
pub struct PinnedCommand {
    pub id: i64,
    pub panel_uuid: String,
    pub command: String,
    pub created_at: String,
}
