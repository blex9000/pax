use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::Database;

/// Derive the DB `record_key` for a workspace without touching the DB. This
/// is the public, side-effect-free counterpart to the logic embedded in
/// `record_workspace_open`. GUI code uses it to find a workspace's metadata.
pub fn compute_record_key(name: &str, config_path: Option<&str>) -> String {
    config_path
        .filter(|path| !path.trim().is_empty())
        .map(|path| format!("path:{}", path))
        .unwrap_or_else(|| format!("name:{}", name))
}

#[derive(Debug, Clone)]
pub struct WorkspaceRecord {
    pub id: i64,
    pub name: String,
    pub config_path: Option<String>,
    pub last_opened: String,
    pub open_count: i64,
    /// True when the user has explicitly pinned this workspace so it
    /// stays at the top of recent lists regardless of `last_opened`.
    pub pinned: bool,
}

impl Database {
    /// Record workspace open (upsert).
    pub fn record_workspace_open(&self, name: &str, config_path: Option<&str>) -> Result<()> {
        let record_key = config_path
            .filter(|path| !path.trim().is_empty())
            .map(|path| format!("path:{}", path))
            .unwrap_or_else(|| format!("name:{}", name));
        self.conn.execute(
            "INSERT INTO workspace_metadata (name, config_path, record_key)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(record_key) DO UPDATE SET
                name = excluded.name,
                last_opened = datetime('now'),
                open_count = open_count + 1,
                config_path = COALESCE(excluded.config_path, workspace_metadata.config_path)",
            rusqlite::params![name, config_path, record_key],
        )?;
        Ok(())
    }

    /// Update or promote a workspace record to a persisted config path without
    /// counting it as a new open. This keeps welcome/recent lists clickable
    /// after the first save of an unsaved workspace.
    pub fn sync_workspace_path(&self, name: &str, config_path: &str) -> Result<()> {
        let record_key = format!("path:{config_path}");
        let legacy_key = format!("name:{name}");

        let path_count: Option<i64> = self
            .conn
            .query_row(
                "SELECT open_count FROM workspace_metadata WHERE record_key = ?1",
                [&record_key],
                |row| row.get(0),
            )
            .optional()?;

        let legacy_record: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT id, open_count
                 FROM workspace_metadata
                 WHERE record_key = ?1
                   AND (config_path IS NULL OR trim(config_path) = '')",
                [&legacy_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let open_count = path_count
            .or_else(|| legacy_record.as_ref().map(|(_, count)| *count))
            .unwrap_or(1);

        self.conn.execute(
            "INSERT INTO workspace_metadata (name, config_path, record_key, open_count)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(record_key) DO UPDATE SET
                name = excluded.name,
                config_path = excluded.config_path,
                last_opened = datetime('now'),
                open_count = MAX(workspace_metadata.open_count, excluded.open_count)",
            params![name, config_path, record_key, open_count],
        )?;

        if let Some((legacy_id, _)) = legacy_record {
            self.conn.execute(
                "DELETE FROM workspace_metadata WHERE id = ?1 AND record_key <> ?2",
                params![legacy_id, record_key],
            )?;
        }

        Ok(())
    }

    /// List all workspaces, most recently opened first.
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>> {
        self.list_workspaces_limit(100)
    }

    /// List recent workspaces with a limit. Pinned entries always come
    /// first regardless of recency, so the user's curated favourites
    /// don't get pushed off the visible window after lots of opens.
    pub fn list_workspaces_limit(&self, limit: usize) -> Result<Vec<WorkspaceRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, config_path, last_opened, open_count, pinned
             FROM workspace_metadata
             ORDER BY pinned DESC, last_opened DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let pinned: i64 = row.get(5)?;
            Ok(WorkspaceRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                config_path: row.get(2)?,
                last_opened: row.get(3)?,
                open_count: row.get(4)?,
                pinned: pinned != 0,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Look up a workspace record by its stored record_key
    /// (format: `path:<config>` or `name:<workspace-name>`). Used by
    /// the scheduled-alert toast so a note can display its owning
    /// workspace name when it differs from the currently open one.
    pub fn find_workspace_by_record_key(
        &self,
        record_key: &str,
    ) -> Result<Option<WorkspaceRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, config_path, last_opened, open_count, pinned
             FROM workspace_metadata WHERE record_key = ?1 LIMIT 1",
        )?;
        let row = stmt
            .query_row([record_key], |row| {
                let pinned: i64 = row.get(5)?;
                Ok(WorkspaceRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    config_path: row.get(2)?,
                    last_opened: row.get(3)?,
                    open_count: row.get(4)?,
                    pinned: pinned != 0,
                })
            })
            .optional()?;
        Ok(row)
    }

    /// Toggle the pinned flag for a workspace identified by its
    /// record_key. Returns Ok(()) even if no row matches so callers
    /// can call this freely without checking existence first.
    pub fn set_workspace_pinned(&self, record_key: &str, pinned: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_metadata SET pinned = ?1 WHERE record_key = ?2",
            rusqlite::params![if pinned { 1 } else { 0 }, record_key],
        )?;
        Ok(())
    }

    /// Compute the same record_key the rest of the app uses for a
    /// given (name, config_path) pair, so callers that have a
    /// WorkspaceRecord (not the raw key) can still toggle pin state.
    pub fn record_key_for(record: &WorkspaceRecord) -> String {
        compute_record_key(&record.name, record.config_path.as_deref())
    }

    /// Remove a workspace record.
    pub fn remove_workspace(&self, name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM workspace_metadata WHERE name = ?1",
            [name],
        )?;
        Ok(())
    }

    /// Remove a single workspace record by its `record_key`. Preferred over
    /// `remove_workspace(name)` when the caller has a `WorkspaceRecord` in
    /// hand, because two records can share a name (different config paths)
    /// and a name-based delete would wipe both.
    pub fn remove_workspace_by_key(&self, record_key: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM workspace_metadata WHERE record_key = ?1",
            [record_key],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_workspace_open_keeps_distinct_paths_with_same_name() {
        let db = Database::open_memory().unwrap();

        db.record_workspace_open("dev", Some("/tmp/one.json")).unwrap();
        db.record_workspace_open("dev", Some("/tmp/two.json")).unwrap();

        let rows = db.list_workspaces_limit(10).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn record_workspace_open_increments_same_path_only_once() {
        let db = Database::open_memory().unwrap();

        db.record_workspace_open("dev", Some("/tmp/one.json")).unwrap();
        db.record_workspace_open("dev-renamed", Some("/tmp/one.json")).unwrap();

        let rows = db.list_workspaces_limit(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].open_count, 2);
        assert_eq!(rows[0].name, "dev-renamed");
    }

    #[test]
    fn sync_workspace_path_promotes_unsaved_record_without_increment() {
        let db = Database::open_memory().unwrap();

        db.record_workspace_open("draft", None).unwrap();
        db.sync_workspace_path("draft", "/tmp/draft.json").unwrap();

        let rows = db.list_workspaces_limit(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "draft");
        assert_eq!(rows[0].config_path.as_deref(), Some("/tmp/draft.json"));
        assert_eq!(rows[0].open_count, 1);
    }

    #[test]
    fn sync_workspace_path_updates_existing_path_record_name() {
        let db = Database::open_memory().unwrap();

        db.record_workspace_open("draft", Some("/tmp/draft.json")).unwrap();
        db.sync_workspace_path("renamed", "/tmp/draft.json").unwrap();

        let rows = db.list_workspaces_limit(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "renamed");
        assert_eq!(rows[0].config_path.as_deref(), Some("/tmp/draft.json"));
        assert_eq!(rows[0].open_count, 1);
    }
}
