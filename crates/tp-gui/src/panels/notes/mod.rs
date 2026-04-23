//! Note panel: free-form markdown note cards scoped per panel instance.
//!
//! Each panel holds its own notes in the database, keyed by
//! `(record_key, panel_id)`. Closing the panel deletes those notes; the
//! trait hooks `close_confirmation` / `on_permanent_close` glue that to the
//! app-level close flow.

pub mod alert_scheduler;
mod card;
mod editor_dialog;
mod list;

use std::rc::Rc;

use super::PanelBackend;

const PANEL_TYPE_ID: &str = "note";

/// Owns the Note panel's GTK root and the scoping keys used for every DB
/// call. Database connections are opened on demand (same pattern as the
/// editor panel) — cheap enough and avoids Sync headaches.
pub struct NotesPanel {
    widget: gtk4::Widget,
    record_key: Rc<String>,
    panel_id: Rc<String>,
    list: Rc<list::NoteListView>,
}

impl std::fmt::Debug for NotesPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotesPanel")
            .field("record_key", &self.record_key)
            .field("panel_id", &self.panel_id)
            .finish()
    }
}

impl NotesPanel {
    pub fn new(record_key: String, panel_id: String) -> Self {
        let record_key = Rc::new(record_key);
        let panel_id = Rc::new(panel_id);
        let list = list::NoteListView::new(record_key.clone(), panel_id.clone());
        let widget = list.widget().clone();
        Self {
            widget,
            record_key,
            panel_id,
            list,
        }
    }

    fn open_db() -> Option<pax_db::Database> {
        pax_db::Database::open(&pax_db::Database::default_path())
            .map_err(|e| {
                tracing::warn!("notes panel: could not open database: {e}");
                e
            })
            .ok()
    }

    fn note_count(&self) -> i64 {
        let Some(db) = Self::open_db() else {
            return 0;
        };
        db.count_notes_for_panel(&self.record_key, &self.panel_id)
            .unwrap_or(0)
    }
}

impl PanelBackend for NotesPanel {
    fn panel_type(&self) -> &str {
        PANEL_TYPE_ID
    }

    fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    fn on_focus(&self) {
        // Refresh from DB in case another panel mutated the set (e.g. the
        // scheduler marked an alert as fired).
        self.list.reload();
    }

    fn close_confirmation(&self) -> Option<String> {
        let count = self.note_count();
        if count == 0 {
            return None;
        }
        Some(format!(
            "This panel contains {count} note{plural}. Closing it will delete them permanently. Continue?",
            plural = if count == 1 { "" } else { "s" }
        ))
    }

    fn on_permanent_close(&self) {
        let Some(db) = Self::open_db() else {
            return;
        };
        match db.delete_notes_for_panel(&self.record_key, &self.panel_id) {
            Ok(n) => {
                if n > 0 {
                    tracing::info!(
                        "notes panel {}: deleted {} note(s) on close",
                        self.panel_id,
                        n
                    );
                }
            }
            Err(e) => tracing::warn!(
                "notes panel {}: failed to delete notes on close: {e}",
                self.panel_id
            ),
        }
    }
}
