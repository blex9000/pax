use anyhow::{Context, Result};
use pax_assistant::{
    redact_json, AssistantRole, AssistantSessionState, AssistantTask, AssistantTaskState,
    WorkspaceSnapshot,
};
use rusqlite::{params, OptionalExtension, Row};
use serde_json::Value;
use uuid::Uuid;

use crate::Database;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantSessionRecord {
    pub id: String,
    pub workspace_record_key: String,
    pub provider: String,
    pub provider_session_id: Option<String>,
    pub status: String,
    pub summary: Option<String>,
    pub context_json: String,
    pub tool_schema_version: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_active_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantMessageRecord {
    pub id: i64,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub role: String,
    pub content: String,
    pub metadata_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAssistantToolRun<'a> {
    pub id: &'a str,
    pub session_id: &'a str,
    pub turn_id: Option<&'a str>,
    pub tool_name: &'a str,
    pub risk: &'a str,
    pub arguments: &'a Value,
    pub status: &'a str,
    pub approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantToolRunRecord {
    pub id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub tool_name: String,
    pub risk: String,
    pub arguments_json: String,
    pub result_json: Option<String>,
    pub status: String,
    pub approved: bool,
    pub error: Option<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
}

impl Database {
    pub fn open_or_create_assistant_session(
        &self,
        workspace_record_key: &str,
        provider: &str,
        snapshot: &WorkspaceSnapshot,
    ) -> Result<AssistantSessionRecord> {
        let context_json = serde_json::to_string(&snapshot.provider_context())?;
        let existing = self
            .conn
            .query_row(
                "SELECT id, workspace_record_key, provider, provider_session_id,
                        status, summary, context_json, tool_schema_version,
                        created_at, updated_at, last_active_at
                 FROM assistant_sessions
                 WHERE workspace_record_key = ?1 AND provider = ?2 AND status != 'closed'
                 ORDER BY last_active_at DESC
                 LIMIT 1",
                params![workspace_record_key, provider],
                map_session,
            )
            .optional()?;

        let now = now_secs();
        if let Some(mut session) = existing {
            self.conn.execute(
                "UPDATE assistant_sessions
                 SET context_json = ?2, updated_at = ?3, last_active_at = ?3
                 WHERE id = ?1",
                params![session.id, context_json, now],
            )?;
            session.context_json = context_json;
            session.updated_at = now;
            session.last_active_at = now;
            return Ok(session);
        }

        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO assistant_sessions
                (id, workspace_record_key, provider, status, context_json,
                 tool_schema_version, created_at, updated_at, last_active_at)
             VALUES (?1, ?2, ?3, 'idle', ?4, 1, ?5, ?5, ?5)",
            params![id, workspace_record_key, provider, context_json, now],
        )?;
        self.assistant_session(&id)?
            .context("assistant session disappeared after insert")
    }

    pub fn assistant_session(&self, id: &str) -> Result<Option<AssistantSessionRecord>> {
        self.conn
            .query_row(
                "SELECT id, workspace_record_key, provider, provider_session_id,
                        status, summary, context_json, tool_schema_version,
                        created_at, updated_at, last_active_at
                 FROM assistant_sessions WHERE id = ?1",
                [id],
                map_session,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn update_assistant_session_context(
        &self,
        id: &str,
        snapshot: &WorkspaceSnapshot,
    ) -> Result<()> {
        let now = now_secs();
        let context_json = serde_json::to_string(&snapshot.provider_context())?;
        self.conn.execute(
            "UPDATE assistant_sessions
             SET context_json = ?2, updated_at = ?3, last_active_at = ?3
             WHERE id = ?1",
            params![id, context_json, now],
        )?;
        Ok(())
    }

    pub fn update_assistant_session_state(
        &self,
        id: &str,
        state: AssistantSessionState,
        provider_session_id: Option<&str>,
        summary: Option<&str>,
    ) -> Result<()> {
        let now = now_secs();
        self.conn.execute(
            "UPDATE assistant_sessions
             SET status = ?2,
                 provider_session_id = COALESCE(?3, provider_session_id),
                 summary = COALESCE(?4, summary),
                 updated_at = ?5,
                 last_active_at = ?5
             WHERE id = ?1",
            params![id, state.as_str(), provider_session_id, summary, now],
        )?;
        Ok(())
    }

    pub fn append_assistant_message(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        role: AssistantRole,
        content: &str,
        metadata: &Value,
    ) -> Result<i64> {
        let mut metadata = metadata.clone();
        redact_json(&mut metadata);
        let metadata_json = serde_json::to_string(&metadata)?;
        self.conn.execute(
            "INSERT INTO assistant_messages
                (session_id, turn_id, role, content, metadata_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                turn_id,
                role.as_str(),
                content,
                metadata_json,
                now_secs()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn assistant_messages(&self, session_id: &str) -> Result<Vec<AssistantMessageRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT id, session_id, turn_id, role, content, metadata_json, created_at
             FROM assistant_messages WHERE session_id = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([session_id], |row| {
            Ok(AssistantMessageRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                turn_id: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                metadata_json: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn start_assistant_tool_run(&self, run: NewAssistantToolRun<'_>) -> Result<()> {
        let mut arguments = run.arguments.clone();
        redact_json(&mut arguments);
        self.conn.execute(
            "INSERT INTO assistant_tool_runs
                (id, session_id, turn_id, tool_name, risk, arguments_json,
                 status, approved, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                run.id,
                run.session_id,
                run.turn_id,
                run.tool_name,
                run.risk,
                serde_json::to_string(&arguments)?,
                run.status,
                run.approved,
                now_secs()
            ],
        )?;
        Ok(())
    }

    pub fn finish_assistant_tool_run(
        &self,
        id: &str,
        status: &str,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> Result<()> {
        let result_json = result
            .map(|value| {
                let mut value = value.clone();
                redact_json(&mut value);
                serde_json::to_string(&value)
            })
            .transpose()?;
        self.conn.execute(
            "UPDATE assistant_tool_runs
             SET status = ?2, result_json = ?3, error = ?4, completed_at = ?5
             WHERE id = ?1",
            params![id, status, result_json, error, now_secs()],
        )?;
        Ok(())
    }

    pub fn save_assistant_task(&self, task: &AssistantTask) -> Result<()> {
        // Terminal output remains an on-demand runtime value. Persist task
        // lifecycle metadata, but do not retain terminal contents in SQLite.
        let mut persisted = task.clone();
        persisted.result = None;
        let mut task_json = serde_json::to_value(&persisted)?;
        redact_json(&mut task_json);
        self.conn.execute(
            "INSERT INTO assistant_tasks
                (id, workspace_record_key, provider, provider_session_id,
                 tool_call_id, tool_name, panel_id, state, task_json,
                 created_at_ms, updated_at_ms, deadline_at_ms, completed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET
                provider_session_id = excluded.provider_session_id,
                state = excluded.state,
                task_json = excluded.task_json,
                updated_at_ms = excluded.updated_at_ms,
                deadline_at_ms = excluded.deadline_at_ms,
                completed_at_ms = excluded.completed_at_ms",
            params![
                persisted.id,
                persisted.workspace_record_key,
                persisted.provider,
                persisted.provider_session_id,
                persisted.tool_call_id,
                persisted.tool_name,
                persisted.target_panel_id,
                persisted.state.as_str(),
                serde_json::to_string(&task_json)?,
                persisted.created_at_ms,
                persisted.updated_at_ms,
                persisted.deadline_at_ms,
                persisted.completed_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn assistant_tasks(&self, workspace_record_key: &str) -> Result<Vec<AssistantTask>> {
        let mut statement = self.conn.prepare(
            "SELECT task_json
             FROM assistant_tasks
             WHERE workspace_record_key = ?1
             ORDER BY created_at_ms DESC
             LIMIT 100",
        )?;
        let rows = statement.query_map([workspace_record_key], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            let json = row?;
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
    }

    pub fn interrupt_active_assistant_tasks(
        &self,
        workspace_record_key: &str,
        reason: &str,
    ) -> Result<Vec<AssistantTask>> {
        let now = now_millis();
        let mut interrupted = Vec::new();
        for mut task in self.assistant_tasks(workspace_record_key)? {
            if !task.state.is_active() {
                continue;
            }
            task.state = AssistantTaskState::Interrupted;
            task.updated_at_ms = now;
            task.completed_at_ms = Some(now);
            task.error = Some(reason.to_string());
            self.save_assistant_task(&task)?;
            interrupted.push(task);
        }
        Ok(interrupted)
    }
}

fn map_session(row: &Row<'_>) -> rusqlite::Result<AssistantSessionRecord> {
    Ok(AssistantSessionRecord {
        id: row.get(0)?,
        workspace_record_key: row.get(1)?,
        provider: row.get(2)?,
        provider_session_id: row.get(3)?,
        status: row.get(4)?,
        summary: row.get(5)?,
        context_json: row.get(6)?,
        tool_schema_version: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        last_active_at: row.get(10)?,
    })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pax_assistant::{LayoutSnapshot, WorkspaceSnapshot, WORKSPACE_SNAPSHOT_VERSION};

    fn snapshot() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            version: WORKSPACE_SNAPSHOT_VERSION,
            workspace_id: Uuid::new_v4(),
            record_key: "workspace-key".into(),
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
        }
    }

    #[test]
    fn assistant_session_is_reused_and_context_is_updated() {
        let db = Database::open_memory().unwrap();
        let first = db
            .open_or_create_assistant_session("workspace-key", "unconfigured", &snapshot())
            .unwrap();
        let mut changed = snapshot();
        changed.dirty = true;
        let second = db
            .open_or_create_assistant_session("workspace-key", "unconfigured", &changed)
            .unwrap();

        assert_eq!(first.id, second.id);
        assert!(second.context_json.contains("\"dirty\":true"));
    }

    #[test]
    fn messages_and_tool_audit_redact_sensitive_metadata() {
        let db = Database::open_memory().unwrap();
        let session = db
            .open_or_create_assistant_session("workspace-key", "unconfigured", &snapshot())
            .unwrap();

        db.append_assistant_message(
            &session.id,
            Some("turn-1"),
            AssistantRole::User,
            "hello",
            &serde_json::json!({"api_key": "secret"}),
        )
        .unwrap();
        let messages = db.assistant_messages(&session.id).unwrap();
        assert!(messages[0].metadata_json.contains("[REDACTED]"));

        db.start_assistant_tool_run(NewAssistantToolRun {
            id: "call-1",
            session_id: &session.id,
            turn_id: Some("turn-1"),
            tool_name: "test.read",
            risk: "read",
            arguments: &serde_json::json!({"password": "secret"}),
            status: "running",
            approved: true,
        })
        .unwrap();
        let arguments: String = db
            .conn
            .query_row(
                "SELECT arguments_json FROM assistant_tool_runs WHERE id = 'call-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(arguments.contains("[REDACTED]"));
    }

    #[test]
    fn assistant_tasks_persist_lifecycle_without_terminal_output() {
        let db = Database::open_memory().unwrap();
        let now = now_millis();
        let task = AssistantTask {
            id: "task-1".into(),
            workspace_record_key: "workspace-key".into(),
            provider: "gemini_live".into(),
            provider_session_id: None,
            tool_call_id: "call-1".into(),
            tool_name: "terminal_wait".into(),
            target_panel_id: Some("p1".into()),
            label: "Build".into(),
            state: AssistantTaskState::Running,
            condition: pax_assistant::AssistantTaskCondition::ShellPrompt {
                command_generation: 4,
            },
            created_at_ms: now,
            updated_at_ms: now,
            deadline_at_ms: now + 30_000,
            completed_at_ms: None,
            result: Some(serde_json::json!({
                "lines": [{"text": "SECRET=value"}]
            })),
            error: None,
        };

        db.save_assistant_task(&task).unwrap();
        let loaded = db.assistant_tasks("workspace-key").unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].state, AssistantTaskState::Running);
        assert!(loaded[0].result.is_none());

        let interrupted = db
            .interrupt_active_assistant_tasks("workspace-key", "application restarted")
            .unwrap();
        assert_eq!(interrupted[0].state, AssistantTaskState::Interrupted);
        assert_eq!(
            db.assistant_tasks("workspace-key").unwrap()[0].state,
            AssistantTaskState::Interrupted
        );
    }
}
