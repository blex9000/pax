//! Markdown panel documents stored in SQLite.
//!
//! These documents are scoped per `(record_key, panel_id)`, matching the
//! Note panel's ownership model: a Markdown panel owns one internal document.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use crate::Database;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Database {
    pub fn get_or_create_markdown_document(
        &self,
        record_key: &str,
        panel_id: &str,
    ) -> Result<String> {
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT content FROM workspace_markdown_documents
                 WHERE record_key = ?1 AND panel_id = ?2",
                params![record_key, panel_id],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(content) = existing {
            return Ok(content);
        }

        let now = now_secs();
        self.conn.execute(
            "INSERT INTO workspace_markdown_documents
                (record_key, panel_id, content, created_at, updated_at)
             VALUES (?1, ?2, '', ?3, ?3)",
            params![record_key, panel_id, now],
        )?;
        Ok(String::new())
    }

    pub fn save_markdown_document(
        &self,
        record_key: &str,
        panel_id: &str,
        content: &str,
    ) -> Result<()> {
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO workspace_markdown_documents
                (record_key, panel_id, content, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(record_key, panel_id) DO UPDATE SET
                content = excluded.content,
                updated_at = excluded.updated_at",
            params![record_key, panel_id, content, now],
        )?;
        Ok(())
    }

    pub fn markdown_document_len(&self, record_key: &str, panel_id: &str) -> Result<i64> {
        let len: Option<i64> = self
            .conn
            .query_row(
                "SELECT COALESCE(LENGTH(content), 0)
                 FROM workspace_markdown_documents
                 WHERE record_key = ?1 AND panel_id = ?2",
                params![record_key, panel_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(len.unwrap_or(0))
    }

    pub fn delete_markdown_document(&self, record_key: &str, panel_id: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM workspace_markdown_documents
             WHERE record_key = ?1 AND panel_id = ?2",
            params![record_key, panel_id],
        )?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_document_round_trip_is_scoped_by_panel() {
        let db = Database::open_memory().unwrap();

        assert_eq!(db.get_or_create_markdown_document("rk", "p1").unwrap(), "");
        db.save_markdown_document("rk", "p1", "# A").unwrap();
        db.save_markdown_document("rk", "p2", "# B").unwrap();

        assert_eq!(
            db.get_or_create_markdown_document("rk", "p1").unwrap(),
            "# A"
        );
        assert_eq!(
            db.get_or_create_markdown_document("rk", "p2").unwrap(),
            "# B"
        );
        assert_eq!(db.markdown_document_len("rk", "p1").unwrap(), 3);

        assert_eq!(db.delete_markdown_document("rk", "p1").unwrap(), 1);
        assert_eq!(db.get_or_create_markdown_document("rk", "p1").unwrap(), "");
        assert_eq!(
            db.get_or_create_markdown_document("rk", "p2").unwrap(),
            "# B"
        );
    }
}
