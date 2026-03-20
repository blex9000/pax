use anyhow::Result;

use crate::Database;

#[derive(Debug)]
pub struct OutputRecord {
    pub id: i64,
    pub workspace_name: Option<String>,
    pub panel_id: String,
    pub content: String,
    pub saved_at: String,
}

impl Database {
    /// Save terminal output for a panel.
    pub fn save_output(
        &self,
        workspace_name: Option<&str>,
        panel_id: &str,
        content: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO saved_output (workspace_name, panel_id, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![workspace_name, panel_id, content],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Search saved output using FTS5.
    pub fn search_output(&self, query: &str, limit: usize) -> Result<Vec<OutputRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT so.id, so.workspace_name, so.panel_id, so.content, so.saved_at
             FROM saved_output so
             JOIN saved_output_fts fts ON so.id = fts.rowid
             WHERE saved_output_fts MATCH ?1
             ORDER BY so.saved_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![query, limit as i64], |row| {
            Ok(OutputRecord {
                id: row.get(0)?,
                workspace_name: row.get(1)?,
                panel_id: row.get(2)?,
                content: row.get(3)?,
                saved_at: row.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Purge output older than N days.
    pub fn purge_old_output(&self, days: u32) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM saved_output WHERE saved_at < datetime('now', ?1)",
            [format!("-{} days", days)],
        )?;
        Ok(deleted)
    }
}
