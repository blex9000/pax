//! Note-typed accessors on top of `metadata_entries`.
//!
//! The database treats every entry as `(record_key, entry_type, file_path,
//! line_number, line_anchor, payload JSON)`. This module is the only place
//! that knows `entry_type = "note"` and how to serialize/deserialize the
//! `{"text": "..."}` payload shape.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::metadata_entries::MetadataEntry;
use crate::Database;

pub const NOTE_ENTRY_TYPE: &str = "note";

#[derive(Debug, Clone)]
pub struct FileNote {
    pub id: i64,
    pub record_key: String,
    pub file_path: String,
    pub line_number: i32,
    pub line_anchor: Option<String>,
    pub text: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct NotePayload {
    text: String,
}

fn payload_from_text(text: &str) -> String {
    serde_json::to_string(&NotePayload {
        text: text.to_string(),
    })
    .expect("note payload serialization cannot fail")
}

fn text_from_payload(payload: &str) -> String {
    serde_json::from_str::<NotePayload>(payload)
        .map(|p| p.text)
        .unwrap_or_default()
}

fn entry_to_note(e: MetadataEntry) -> FileNote {
    FileNote {
        id: e.id,
        record_key: e.record_key,
        file_path: e.file_path,
        line_number: e.line_number,
        line_anchor: e.line_anchor,
        text: text_from_payload(&e.payload),
        created_at: e.created_at,
        updated_at: e.updated_at,
    }
}

impl Database {
    /// Insert a new note. Returns the persisted FileNote (id populated).
    pub fn add_note(
        &self,
        record_key: &str,
        file_path: &str,
        line_number: i32,
        line_anchor: Option<&str>,
        text: &str,
    ) -> Result<FileNote> {
        let payload = payload_from_text(text);
        let id = self.insert_metadata_entry(
            record_key,
            NOTE_ENTRY_TYPE,
            file_path,
            line_number,
            line_anchor,
            &payload,
        )?;
        let entry = self
            .get_metadata_entry(id)?
            .expect("row we just inserted must exist");
        Ok(entry_to_note(entry))
    }

    /// Replace a note's text.
    pub fn update_note_text(&self, id: i64, text: &str) -> Result<()> {
        self.update_metadata_payload(id, &payload_from_text(text))
    }

    /// List all notes attached to a file in a workspace.
    pub fn list_notes_for_file(
        &self,
        record_key: &str,
        file_path: &str,
    ) -> Result<Vec<FileNote>> {
        Ok(self
            .list_metadata_by_file(record_key, NOTE_ENTRY_TYPE, file_path)?
            .into_iter()
            .map(entry_to_note)
            .collect())
    }

    /// List every note in a workspace.
    pub fn list_notes_for_workspace(&self, record_key: &str) -> Result<Vec<FileNote>> {
        Ok(self
            .list_metadata_for_workspace(record_key, Some(NOTE_ENTRY_TYPE))?
            .into_iter()
            .map(entry_to_note)
            .collect())
    }
}
