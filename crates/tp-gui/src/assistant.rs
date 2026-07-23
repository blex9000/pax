use std::cell::RefCell;
use std::path::{Path, PathBuf};

use pax_assistant::{AssistantRole, ProviderId, WorkspaceSnapshot};

pub(crate) struct AssistantContextStore {
    db_path: PathBuf,
    session_id: RefCell<Option<String>>,
    last_context_json: RefCell<Option<String>>,
}

impl AssistantContextStore {
    pub(crate) fn new() -> Self {
        Self::with_db_path(&pax_db::Database::default_path())
    }

    fn with_db_path(db_path: &Path) -> Self {
        Self {
            db_path: db_path.to_path_buf(),
            session_id: RefCell::new(None),
            last_context_json: RefCell::new(None),
        }
    }

    pub(crate) fn refresh(&self, snapshot: &WorkspaceSnapshot) {
        let Ok(context_json) = serde_json::to_string(&snapshot.provider_context()) else {
            tracing::warn!("failed to serialize assistant workspace context");
            return;
        };
        if self.last_context_json.borrow().as_deref() == Some(context_json.as_str()) {
            return;
        }

        let session_id = self.session_id.borrow().clone();
        let result = pax_db::Database::open(&self.db_path).and_then(|db| {
            if let Some(session_id) = session_id.as_deref() {
                db.update_assistant_session_context(session_id, snapshot)
            } else {
                let session = db.open_or_create_assistant_session(
                    &snapshot.record_key,
                    ProviderId::GEMINI_LIVE,
                    snapshot,
                )?;
                self.session_id.borrow_mut().replace(session.id);
                Ok(())
            }
        });

        match result {
            Ok(()) => {
                self.last_context_json.replace(Some(context_json));
            }
            Err(error) => {
                tracing::warn!(%error, "failed to persist assistant context");
            }
        }
    }

    pub(crate) fn append_message(
        &self,
        role: AssistantRole,
        content: &str,
        metadata: &serde_json::Value,
    ) {
        let content = content.trim();
        if content.is_empty() {
            return;
        }
        let Some(session_id) = self.session_id.borrow().clone() else {
            tracing::warn!("assistant message ignored because no session is active");
            return;
        };
        if let Err(error) = pax_db::Database::open(&self.db_path).and_then(|db| {
            db.append_assistant_message(&session_id, None, role, content, metadata)
                .map(|_| ())
        }) {
            tracing::warn!(%error, "failed to persist assistant message");
        }
    }

    pub(crate) fn messages(&self) -> Vec<pax_db::AssistantMessageRecord> {
        let Some(session_id) = self.session_id.borrow().clone() else {
            return Vec::new();
        };
        match pax_db::Database::open(&self.db_path)
            .and_then(|db| db.assistant_messages(&session_id))
        {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(%error, "failed to load assistant messages");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pax_assistant::{LayoutSnapshot, WorkspaceSnapshot, WORKSPACE_SNAPSHOT_VERSION};
    use uuid::Uuid;

    #[test]
    fn first_refresh_creates_and_then_reuses_session() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("assistant.db");
        let store = AssistantContextStore::with_db_path(&db_path);
        let snapshot = WorkspaceSnapshot {
            version: WORKSPACE_SNAPSHOT_VERSION,
            workspace_id: Uuid::new_v4(),
            record_key: "test-workspace".into(),
            name: "Test".into(),
            config_path: None,
            dirty: false,
            focused_panel_id: None,
            zoomed_panel_id: None,
            active_tabs: Vec::new(),
            layout: LayoutSnapshot::Panel {
                panel_id: "p1".into(),
            },
            panels: Vec::new(),
        };

        store.refresh(&snapshot);
        store.refresh(&snapshot);
        store.append_message(
            AssistantRole::User,
            "hello",
            &serde_json::json!({"channel": "text"}),
        );

        let db = pax_db::Database::open(&db_path).unwrap();
        let sessions: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM assistant_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(sessions, 1);
        assert_eq!(store.messages()[0].content, "hello");
    }
}
