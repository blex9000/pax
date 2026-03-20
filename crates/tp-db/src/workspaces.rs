use anyhow::Result;

use crate::Database;

#[derive(Debug)]
pub struct WorkspaceRecord {
    pub id: i64,
    pub name: String,
    pub config_path: Option<String>,
    pub last_opened: String,
    pub open_count: i64,
}

impl Database {
    /// Record workspace open (upsert).
    pub fn record_workspace_open(&self, name: &str, config_path: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO workspace_metadata (name, config_path)
             VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET
                last_opened = datetime('now'),
                open_count = open_count + 1,
                config_path = COALESCE(?2, config_path)",
            rusqlite::params![name, config_path],
        )?;
        Ok(())
    }

    /// List all workspaces, most recently opened first.
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>> {
        self.list_workspaces_limit(100)
    }

    /// List recent workspaces with a limit.
    pub fn list_workspaces_limit(&self, limit: usize) -> Result<Vec<WorkspaceRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, config_path, last_opened, open_count
             FROM workspace_metadata ORDER BY last_opened DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(WorkspaceRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                config_path: row.get(2)?,
                last_opened: row.get(3)?,
                open_count: row.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Remove a workspace record.
    pub fn remove_workspace(&self, name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM workspace_metadata WHERE name = ?1",
            [name],
        )?;
        Ok(())
    }
}
