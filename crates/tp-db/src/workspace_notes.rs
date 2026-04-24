//! Free-form workspace notes, scoped per `(record_key, panel_id)`.
//!
//! Each `Note` panel instance owns its own set of notes; closing the panel
//! deletes them. Notes carry markdown text, a list of tags, a severity
//! (`info` | `warning` | `important`), and an optional scheduled alert.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::Database;

pub const SEVERITY_INFO: &str = "info";
pub const SEVERITY_WARNING: &str = "warning";
pub const SEVERITY_IMPORTANT: &str = "important";
pub const NOTE_SEVERITIES: &[&str] = &[SEVERITY_INFO, SEVERITY_WARNING, SEVERITY_IMPORTANT];

/// A single workspace note.
#[derive(Debug, Clone)]
pub struct WorkspaceNote {
    pub id: i64,
    pub record_key: String,
    pub panel_id: String,
    pub title: String,
    pub text: String,
    pub tags: Vec<String>,
    pub severity: String,
    pub alert_at: Option<i64>,
    pub alert_fired_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn normalize_severity(s: &str) -> &'static str {
    match s {
        SEVERITY_WARNING => SEVERITY_WARNING,
        SEVERITY_IMPORTANT => SEVERITY_IMPORTANT,
        _ => SEVERITY_INFO,
    }
}

fn tags_to_json(tags: &[String]) -> String {
    serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string())
}

fn tags_from_json(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceNote> {
    let tags_json: String = row.get(5)?;
    Ok(WorkspaceNote {
        id: row.get(0)?,
        record_key: row.get(1)?,
        panel_id: row.get(2)?,
        title: row.get(3)?,
        text: row.get(4)?,
        tags: tags_from_json(&tags_json),
        severity: row.get(6)?,
        alert_at: row.get(7)?,
        alert_fired_at: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

const SELECT_COLUMNS: &str =
    "id, record_key, panel_id, title, text, tags, severity, alert_at, alert_fired_at, created_at, updated_at";

/// Sort clause used by both list and search results so the ordering stays
/// consistent: `important` notes on top, then most recently created first.
const ORDER_CLAUSE: &str =
    "ORDER BY (CASE WHEN severity = 'important' THEN 0 ELSE 1 END) ASC, created_at DESC, id DESC";

impl Database {
    pub fn add_workspace_note(
        &self,
        record_key: &str,
        panel_id: &str,
        title: &str,
        text: &str,
        tags: &[String],
        severity: &str,
        alert_at: Option<i64>,
    ) -> Result<WorkspaceNote> {
        let now = now_secs();
        let severity = normalize_severity(severity);
        let tags_json = tags_to_json(tags);
        self.conn.execute(
            "INSERT INTO workspace_notes
                (record_key, panel_id, title, text, tags, severity, alert_at, alert_fired_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?8)",
            params![record_key, panel_id, title, text, tags_json, severity, alert_at, now],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_workspace_note(id)?
            .ok_or_else(|| anyhow::anyhow!("inserted note {} not found", id))
    }

    pub fn update_workspace_note(
        &self,
        id: i64,
        title: &str,
        text: &str,
        tags: &[String],
        severity: &str,
        alert_at: Option<i64>,
    ) -> Result<()> {
        let severity = normalize_severity(severity);
        let tags_json = tags_to_json(tags);
        // Re-setting alert_at to a new value (or clearing it) also clears
        // alert_fired_at so the scheduler can refire.
        self.conn.execute(
            "UPDATE workspace_notes
             SET title = ?2, text = ?3, tags = ?4, severity = ?5, alert_at = ?6, alert_fired_at = NULL, updated_at = ?7
             WHERE id = ?1",
            params![id, title, text, tags_json, severity, alert_at, now_secs()],
        )?;
        Ok(())
    }

    pub fn delete_workspace_note(&self, id: i64) -> Result<usize> {
        let n = self
            .conn
            .execute("DELETE FROM workspace_notes WHERE id = ?1", [id])?;
        Ok(n)
    }

    pub fn get_workspace_note(&self, id: i64) -> Result<Option<WorkspaceNote>> {
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM workspace_notes WHERE id = ?1"
        );
        self.conn
            .query_row(&sql, [id], row_to_note)
            .optional()
            .map_err(Into::into)
    }

    pub fn list_notes_for_panel(
        &self,
        record_key: &str,
        panel_id: &str,
    ) -> Result<Vec<WorkspaceNote>> {
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM workspace_notes
             WHERE record_key = ?1 AND panel_id = ?2
             {ORDER_CLAUSE}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![record_key, panel_id], row_to_note)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Full-text search over text + tags, constrained to one panel. Empty
    /// query falls back to the full list so callers can wire a single path.
    pub fn search_notes_for_panel(
        &self,
        record_key: &str,
        panel_id: &str,
        query: &str,
    ) -> Result<Vec<WorkspaceNote>> {
        if query.trim().is_empty() {
            return self.list_notes_for_panel(record_key, panel_id);
        }
        let mut stmt = self.conn.prepare(
            "SELECT wn.id, wn.record_key, wn.panel_id, wn.text, wn.tags, wn.severity,
                    wn.alert_at, wn.alert_fired_at, wn.created_at, wn.updated_at
             FROM workspace_notes wn
             JOIN workspace_notes_fts fts ON wn.id = fts.rowid
             WHERE workspace_notes_fts MATCH ?1
               AND wn.record_key = ?2
               AND wn.panel_id = ?3
             ORDER BY (CASE WHEN wn.severity = 'important' THEN 0 ELSE 1 END) ASC,
                      wn.created_at DESC, wn.id DESC",
        )?;
        let rows = stmt.query_map(params![query, record_key, panel_id], row_to_note)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Distinct tags observed on the notes of a panel (used to populate the
    /// tag filter dropdown).
    pub fn list_tags_for_panel(
        &self,
        record_key: &str,
        panel_id: &str,
    ) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT tags FROM workspace_notes WHERE record_key = ?1 AND panel_id = ?2",
        )?;
        let rows = stmt.query_map(params![record_key, panel_id], |row| {
            let s: String = row.get(0)?;
            Ok(s)
        })?;
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        for row in rows.flatten() {
            for tag in tags_from_json(&row) {
                if !tag.is_empty() {
                    seen.insert(tag);
                }
            }
        }
        Ok(seen.into_iter().collect())
    }

    pub fn count_notes_for_panel(&self, record_key: &str, panel_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM workspace_notes WHERE record_key = ?1 AND panel_id = ?2",
            params![record_key, panel_id],
            |row| row.get(0),
        )?;
        Ok(n)
    }

    pub fn delete_notes_for_panel(&self, record_key: &str, panel_id: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM workspace_notes WHERE record_key = ?1 AND panel_id = ?2",
            params![record_key, panel_id],
        )?;
        Ok(n)
    }

    /// Notes whose scheduled alert has come due and has not been fired yet.
    /// Returned sorted oldest-first so the scheduler fires them in order.
    pub fn due_workspace_notes(&self, now: i64) -> Result<Vec<WorkspaceNote>> {
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM workspace_notes
             WHERE alert_at IS NOT NULL
               AND alert_fired_at IS NULL
               AND alert_at <= ?1
             ORDER BY alert_at ASC, id ASC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([now], row_to_note)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn mark_note_alert_fired(&self, id: i64, fired_at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_notes SET alert_fired_at = ?2 WHERE id = ?1",
            params![id, fired_at],
        )?;
        Ok(())
    }
}
