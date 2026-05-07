//! Generic storage for `workspace_file_metadata_entries` — the file-scoped
//! per-workspace metadata table. Entries carry an `entry_type` discriminator
//! (e.g. "note") and an opaque JSON payload; type-specific wrappers (see
//! `crate::notes`) translate between strongly-typed structs and the payload.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::Database;

#[derive(Debug, Clone)]
pub struct MetadataEntry {
    pub id: i64,
    pub record_key: String,
    pub entry_type: String,
    pub file_path: String,
    pub line_number: i32,
    pub line_anchor: Option<String>,
    pub payload: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Database {
    /// Insert a new metadata entry. Returns the assigned row id.
    pub fn insert_metadata_entry(
        &self,
        record_key: &str,
        entry_type: &str,
        file_path: &str,
        line_number: i32,
        line_anchor: Option<&str>,
        payload: &str,
    ) -> Result<i64> {
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO workspace_file_metadata_entries
                (record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![record_key, entry_type, file_path, line_number, line_anchor, payload, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Update an entry's anchor position (line + anchor text).
    pub fn update_metadata_position(
        &self,
        id: i64,
        line_number: i32,
        line_anchor: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_file_metadata_entries
             SET line_number = ?2, line_anchor = ?3, updated_at = ?4
             WHERE id = ?1",
            params![id, line_number, line_anchor, now_secs()],
        )?;
        Ok(())
    }

    /// Replace the JSON payload of an entry.
    pub fn update_metadata_payload(&self, id: i64, payload: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_file_metadata_entries
             SET payload = ?2, updated_at = ?3
             WHERE id = ?1",
            params![id, payload, now_secs()],
        )?;
        Ok(())
    }

    /// Delete a single entry by id. Returns how many rows were removed (0 or 1).
    pub fn delete_metadata_entry(&self, id: i64) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM workspace_file_metadata_entries WHERE id = ?1",
            [id],
        )?;
        Ok(n)
    }

    /// Delete every entry belonging to a workspace. Returns the row count.
    pub fn delete_metadata_for_workspace(&self, record_key: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM workspace_file_metadata_entries WHERE record_key = ?1",
            [record_key],
        )?;
        Ok(n)
    }

    /// List entries for a single file in a workspace, filtered by type.
    /// Ordered by `line_number` ascending, then `id` for stable order within
    /// the same line.
    pub fn list_metadata_by_file(
        &self,
        record_key: &str,
        entry_type: &str,
        file_path: &str,
    ) -> Result<Vec<MetadataEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
             FROM workspace_file_metadata_entries
             WHERE record_key = ?1 AND entry_type = ?2 AND file_path = ?3
             ORDER BY line_number ASC, id ASC",
        )?;
        let rows: Vec<MetadataEntry> = stmt
            .query_map(params![record_key, entry_type, file_path], row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// List entries for a whole workspace. `entry_type` filters when Some.
    pub fn list_metadata_for_workspace(
        &self,
        record_key: &str,
        entry_type: Option<&str>,
    ) -> Result<Vec<MetadataEntry>> {
        let results: Vec<MetadataEntry> = match entry_type {
            Some(t) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                     FROM workspace_file_metadata_entries
                     WHERE record_key = ?1 AND entry_type = ?2
                     ORDER BY file_path ASC, line_number ASC, id ASC",
                )?;
                let collected: Vec<MetadataEntry> = stmt
                    .query_map(params![record_key, t], row_to_entry)?
                    .filter_map(|r| r.ok())
                    .collect();
                collected
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                     FROM workspace_file_metadata_entries
                     WHERE record_key = ?1
                     ORDER BY entry_type ASC, file_path ASC, line_number ASC, id ASC",
                )?;
                let collected: Vec<MetadataEntry> = stmt
                    .query_map(params![record_key], row_to_entry)?
                    .filter_map(|r| r.ok())
                    .collect();
                collected
            }
        };
        Ok(results)
    }

    /// List entries across every workspace. Both `search` (substring match
    /// over file_path and payload) and `entry_type` filters are optional.
    pub fn list_metadata_across_workspaces(
        &self,
        search: Option<&str>,
        entry_type: Option<&str>,
    ) -> Result<Vec<MetadataEntry>> {
        let like_pattern: Option<String> =
            search.filter(|s| !s.is_empty()).map(|s| format!("%{}%", s));

        let results: Vec<MetadataEntry> = match (&like_pattern, entry_type) {
            (Some(pat), Some(t)) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                     FROM workspace_file_metadata_entries
                     WHERE (file_path LIKE ?1 OR payload LIKE ?1)
                       AND entry_type = ?2
                     ORDER BY record_key, entry_type, file_path, line_number, id",
                )?;
                let collected: Vec<MetadataEntry> = stmt
                    .query_map(params![pat, t], row_to_entry)?
                    .filter_map(|r| r.ok())
                    .collect();
                collected
            }
            (Some(pat), None) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                     FROM workspace_file_metadata_entries
                     WHERE file_path LIKE ?1 OR payload LIKE ?1
                     ORDER BY record_key, entry_type, file_path, line_number, id",
                )?;
                let collected: Vec<MetadataEntry> = stmt
                    .query_map(params![pat], row_to_entry)?
                    .filter_map(|r| r.ok())
                    .collect();
                collected
            }
            (None, Some(t)) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                     FROM workspace_file_metadata_entries
                     WHERE entry_type = ?1
                     ORDER BY record_key, entry_type, file_path, line_number, id",
                )?;
                let collected: Vec<MetadataEntry> = stmt
                    .query_map(params![t], row_to_entry)?
                    .filter_map(|r| r.ok())
                    .collect();
                collected
            }
            (None, None) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                     FROM workspace_file_metadata_entries
                     ORDER BY record_key, entry_type, file_path, line_number, id",
                )?;
                let collected: Vec<MetadataEntry> = stmt
                    .query_map([], row_to_entry)?
                    .filter_map(|r| r.ok())
                    .collect();
                collected
            }
        };
        Ok(results)
    }

    /// Return every distinct `entry_type` seen in the table, alphabetical.
    pub fn list_metadata_entry_types(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT entry_type FROM workspace_file_metadata_entries ORDER BY entry_type ASC",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Fetch an entry by id.
    pub fn get_metadata_entry(&self, id: i64) -> Result<Option<MetadataEntry>> {
        self.conn
            .query_row(
                "SELECT id, record_key, entry_type, file_path, line_number, line_anchor, payload, created_at, updated_at
                 FROM workspace_file_metadata_entries WHERE id = ?1",
                [id],
                row_to_entry,
            )
            .optional()
            .map_err(Into::into)
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MetadataEntry> {
    Ok(MetadataEntry {
        id: row.get(0)?,
        record_key: row.get(1)?,
        entry_type: row.get(2)?,
        file_path: row.get(3)?,
        line_number: row.get(4)?,
        line_anchor: row.get(5)?,
        payload: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}
