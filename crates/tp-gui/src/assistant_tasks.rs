use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use pax_assistant::{
    AssistantTask, AssistantTaskCondition, AssistantTaskEvent, AssistantTaskState,
};
use serde_json::Value;

use crate::voice_tools::{
    VoiceToolCall, VoiceToolCompletion, VoiceToolExecution, VoiceToolResult, TASK_CANCEL_TOOL,
    TASK_STATUS_TOOL, TERMINAL_WAIT_TOOL,
};

const DEFAULT_TIMEOUT_SECONDS: u64 = 300;
const MAX_TIMEOUT_SECONDS: u64 = 600;
const DEFAULT_QUIET_MS: u64 = 900;
const MIN_QUIET_MS: u64 = 250;
const MAX_QUIET_MS: u64 = 10_000;
const DEFAULT_OUTPUT_LINES: usize = 60;
const MAX_OUTPUT_LINES: usize = 200;
const TASK_TICK_MS: u64 = 100;
const DELIVERY_ACK_TIMEOUT_MS: i64 = 10_000;
const MAX_TASK_LABEL_CHARS: usize = 80;

struct RuntimeTask {
    snapshot: AssistantTask,
    output_lines: usize,
    completion: Option<tokio::sync::oneshot::Sender<VoiceToolCompletion>>,
    terminal_listener_id: Option<u64>,
}

type TaskEventListener = Rc<dyn Fn(AssistantTaskEvent)>;

pub(crate) struct AssistantTaskSupervisor {
    workspace: Weak<RefCell<crate::workspace_view::WorkspaceView>>,
    workspace_record_key: String,
    provider: String,
    db_path: std::path::PathBuf,
    tasks: RefCell<HashMap<String, RuntimeTask>>,
    listeners: RefCell<Vec<TaskEventListener>>,
}

impl AssistantTaskSupervisor {
    pub(crate) fn new(
        workspace: &Rc<RefCell<crate::workspace_view::WorkspaceView>>,
        provider: &str,
    ) -> Rc<Self> {
        let workspace_record_key = workspace.borrow().assistant_snapshot().record_key;
        let db_path = pax_db::Database::default_path();
        let persisted = pax_db::Database::open(&db_path)
            .and_then(|db| {
                db.interrupt_active_assistant_tasks(
                    &workspace_record_key,
                    "Pax e' stato riavviato mentre il task era attivo.",
                )?;
                db.assistant_tasks(&workspace_record_key)
            })
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "failed to restore assistant tasks");
                Vec::new()
            });
        let tasks = persisted
            .into_iter()
            .map(|snapshot| {
                (
                    snapshot.id.clone(),
                    RuntimeTask {
                        snapshot,
                        output_lines: DEFAULT_OUTPUT_LINES,
                        completion: None,
                        terminal_listener_id: None,
                    },
                )
            })
            .collect();

        Rc::new(Self {
            workspace: Rc::downgrade(workspace),
            workspace_record_key,
            provider: provider.to_string(),
            db_path,
            tasks: RefCell::new(tasks),
            listeners: RefCell::new(Vec::new()),
        })
    }

    pub(crate) fn execute(self: &Rc<Self>, call: VoiceToolCall) -> Option<VoiceToolExecution> {
        match call.name.as_str() {
            TERMINAL_WAIT_TOOL => Some(self.start_terminal_wait(call)),
            TASK_STATUS_TOOL => Some(VoiceToolExecution::immediate(self.task_status(&call))),
            TASK_CANCEL_TOOL => Some(VoiceToolExecution::immediate(self.cancel_from_tool(&call))),
            _ => None,
        }
    }

    pub(crate) fn subscribe(&self, listener: TaskEventListener) {
        self.listeners.borrow_mut().push(listener);
    }

    pub(crate) fn tasks(&self) -> Vec<AssistantTask> {
        let mut tasks = self
            .tasks
            .borrow()
            .values()
            .map(|runtime| runtime.snapshot.clone())
            .collect::<Vec<_>>();
        tasks.sort_by_key(|task| std::cmp::Reverse(task.created_at_ms));
        tasks
    }

    pub(crate) fn cancel(self: &Rc<Self>, task_id: &str) -> bool {
        let Some(task) = self
            .tasks
            .borrow()
            .get(task_id)
            .map(|runtime| runtime.snapshot.clone())
        else {
            return false;
        };
        if !task.state.is_active() {
            return false;
        }
        let result = VoiceToolResult {
            call_id: task.tool_call_id.clone(),
            name: task.tool_name.clone(),
            response: serde_json::json!({
                "status": "cancelled",
                "task_id": task.id,
                "panel_id": task.target_panel_id,
                "message": "Monitoraggio annullato. Il processo nel terminale non e' stato interrotto."
            }),
        };
        self.finish_task(
            task_id,
            AssistantTaskState::Cancelled,
            result,
            Some("Monitoraggio annullato dall'utente.".to_string()),
        );
        true
    }

    fn start_terminal_wait(self: &Rc<Self>, call: VoiceToolCall) -> VoiceToolExecution {
        match self.prepare_terminal_wait(&call) {
            Ok((task, output_lines)) => {
                let task_id = task.id.clone();
                let panel_id = task
                    .target_panel_id
                    .clone()
                    .expect("terminal wait task has panel");
                let (completion, receiver) = tokio::sync::oneshot::channel();
                self.tasks.borrow_mut().insert(
                    task_id.clone(),
                    RuntimeTask {
                        snapshot: task.clone(),
                        output_lines,
                        completion: Some(completion),
                        terminal_listener_id: None,
                    },
                );
                self.persist(&task);
                self.notify(AssistantTaskEvent::Created(task));
                self.install_terminal_observers(&panel_id, &task_id);
                self.install_task_tick(&task_id);
                self.schedule_evaluate(&task_id);
                VoiceToolExecution::Pending { task_id, receiver }
            }
            Err(error) => VoiceToolExecution::immediate(VoiceToolResult::error(&call, error)),
        }
    }

    fn prepare_terminal_wait(
        &self,
        call: &VoiceToolCall,
    ) -> Result<(AssistantTask, usize), String> {
        let workspace = self
            .workspace
            .upgrade()
            .ok_or_else(|| "Workspace non piu' disponibile.".to_string())?;
        let view = workspace
            .try_borrow()
            .map_err(|_| "Workspace occupato; riprova il monitoraggio.".to_string())?;
        let panel_id = crate::voice_tools::terminal_panel_id(&view, &call.arguments)?;
        let host = view
            .host(&panel_id)
            .ok_or_else(|| format!("Terminale '{panel_id}' non disponibile."))?;
        let runtime = host.terminal_runtime_snapshot();
        let condition_name = required_string(&call.arguments, "condition")?;
        let condition = match condition_name.as_str() {
            "shell_prompt" => {
                let generation =
                    optional_u64(&call.arguments, "watch_token")?.unwrap_or_else(|| {
                        if runtime.busy {
                            runtime.command_generation
                        } else {
                            runtime.command_generation.wrapping_add(1)
                        }
                    });
                AssistantTaskCondition::ShellPrompt {
                    command_generation: generation,
                }
            }
            "output_changed" => AssistantTaskCondition::OutputChanged {
                after_revision: optional_u64(&call.arguments, "after_revision")?
                    .unwrap_or(runtime.output_revision),
            },
            "output_quiet" => AssistantTaskCondition::OutputQuiet {
                after_revision: optional_u64(&call.arguments, "after_revision")?
                    .unwrap_or(runtime.output_revision),
                quiet_ms: optional_u64(&call.arguments, "quiet_ms")?
                    .unwrap_or(DEFAULT_QUIET_MS)
                    .clamp(MIN_QUIET_MS, MAX_QUIET_MS),
            },
            "contains_text" => AssistantTaskCondition::ContainsText {
                text: required_string(&call.arguments, "text")?,
                case_sensitive: call
                    .arguments
                    .get("case_sensitive")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                after_revision: optional_u64(&call.arguments, "after_revision")?
                    .unwrap_or(runtime.output_revision),
            },
            _ => {
                return Err(format!(
                    "Condizione terminal_wait non supportata: {condition_name}."
                ));
            }
        };

        let timeout_seconds = optional_u64(&call.arguments, "timeout_seconds")?
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
            .clamp(1, MAX_TIMEOUT_SECONDS);
        let output_lines = optional_u64(&call.arguments, "output_lines")?
            .unwrap_or(DEFAULT_OUTPUT_LINES as u64)
            .clamp(1, MAX_OUTPUT_LINES as u64) as usize;
        let label = call
            .arguments
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(|label| label.chars().take(MAX_TASK_LABEL_CHARS).collect())
            .unwrap_or_else(|| condition.label().to_string());
        let now = now_millis();
        Ok((
            AssistantTask {
                id: format!("task-{}", uuid::Uuid::new_v4().simple()),
                workspace_record_key: self.workspace_record_key.clone(),
                provider: self.provider.clone(),
                provider_session_id: None,
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                target_panel_id: Some(panel_id),
                label,
                state: AssistantTaskState::Running,
                condition,
                created_at_ms: now,
                updated_at_ms: now,
                deadline_at_ms: now + timeout_seconds as i64 * 1_000,
                completed_at_ms: None,
                result: None,
                error: None,
            },
            output_lines,
        ))
    }

    fn install_terminal_observers(self: &Rc<Self>, panel_id: &str, task_id: &str) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let Ok(view) = workspace.try_borrow() else {
            return;
        };
        let Some(host) = view.host(panel_id) else {
            return;
        };

        let weak = Rc::downgrade(self);
        let task_id_for_status = task_id.to_string();
        let weak_for_output = weak.clone();
        let task_id_for_output = task_id.to_string();
        let listener_id = host.add_terminal_runtime_listener(
            Box::new(move |_| {
                if let Some(supervisor) = weak.upgrade() {
                    supervisor.schedule_evaluate(&task_id_for_status);
                }
            }),
            Box::new(move || {
                if let Some(supervisor) = weak_for_output.upgrade() {
                    supervisor.schedule_evaluate(&task_id_for_output);
                }
            }),
        );
        drop(view);
        if let Some(runtime) = self.tasks.borrow_mut().get_mut(task_id) {
            runtime.terminal_listener_id = Some(listener_id);
        }
    }

    fn install_task_tick(self: &Rc<Self>, task_id: &str) {
        let weak = Rc::downgrade(self);
        let task_id = task_id.to_string();
        gtk4::glib::timeout_add_local(Duration::from_millis(TASK_TICK_MS), move || {
            let Some(supervisor) = weak.upgrade() else {
                return gtk4::glib::ControlFlow::Break;
            };
            supervisor.evaluate(&task_id);
            if supervisor.is_active(&task_id) {
                gtk4::glib::ControlFlow::Continue
            } else {
                gtk4::glib::ControlFlow::Break
            }
        });
    }

    fn schedule_evaluate(self: &Rc<Self>, task_id: &str) {
        let weak = Rc::downgrade(self);
        let task_id = task_id.to_string();
        gtk4::glib::idle_add_local_once(move || {
            if let Some(supervisor) = weak.upgrade() {
                supervisor.evaluate(&task_id);
            }
        });
    }

    fn evaluate(self: &Rc<Self>, task_id: &str) {
        let Some(task) = self
            .tasks
            .borrow()
            .get(task_id)
            .map(|runtime| runtime.snapshot.clone())
        else {
            return;
        };
        if !task.state.is_active() {
            return;
        }

        let now = now_millis();
        if now >= task.deadline_at_ms {
            let result = VoiceToolResult {
                call_id: task.tool_call_id.clone(),
                name: task.tool_name.clone(),
                response: serde_json::json!({
                    "status": "timeout",
                    "task_id": task.id,
                    "panel_id": task.target_panel_id,
                    "message": "La condizione attesa non si e' verificata entro il timeout."
                }),
            };
            self.finish_task(
                task_id,
                AssistantTaskState::TimedOut,
                result,
                Some("Timeout del monitoraggio terminale.".to_string()),
            );
            return;
        }

        let Some(workspace) = self.workspace.upgrade() else {
            self.fail_unavailable(task_id, &task, "Workspace non piu' disponibile.");
            return;
        };
        let Ok(view) = workspace.try_borrow() else {
            return;
        };
        let Some(panel_id) = task.target_panel_id.as_deref() else {
            self.fail_unavailable(task_id, &task, "Task senza terminale target.");
            return;
        };
        let Some(host) = view.host(panel_id) else {
            self.fail_unavailable(task_id, &task, "Il terminale monitorato non esiste piu'.");
            return;
        };
        let runtime = host.terminal_runtime_snapshot();
        let content = matches!(task.condition, AssistantTaskCondition::ContainsText { .. })
            .then(|| host.text_content())
            .flatten();
        let condition_met =
            terminal_condition_met(&task.condition, runtime, now, content.as_deref());
        if !condition_met {
            return;
        }

        let output_lines = self
            .tasks
            .borrow()
            .get(task_id)
            .map(|runtime| runtime.output_lines)
            .unwrap_or(DEFAULT_OUTPUT_LINES);
        let content = host.text_content().unwrap_or_default();
        let output = crate::voice_tools::recent_terminal_output(panel_id, &content, output_lines);
        let result = VoiceToolResult {
            call_id: task.tool_call_id.clone(),
            name: task.tool_name.clone(),
            response: serde_json::json!({
                "status": "ok",
                "task_id": task.id,
                "panel_id": panel_id,
                "condition": task.condition,
                "duration_ms": now.saturating_sub(task.created_at_ms),
                "output_revision": runtime.output_revision,
                "busy": runtime.busy,
                "command_generation": runtime.command_generation,
                "completed_generation": runtime.completed_generation,
                "output": output
            }),
        };
        drop(view);
        self.finish_task(task_id, AssistantTaskState::Succeeded, result, None);
    }

    fn fail_unavailable(self: &Rc<Self>, task_id: &str, task: &AssistantTask, message: &str) {
        let result = VoiceToolResult {
            call_id: task.tool_call_id.clone(),
            name: task.tool_name.clone(),
            response: serde_json::json!({
                "status": "error",
                "task_id": task.id,
                "error": message
            }),
        };
        self.finish_task(
            task_id,
            AssistantTaskState::Failed,
            result,
            Some(message.to_string()),
        );
    }

    fn finish_task(
        self: &Rc<Self>,
        task_id: &str,
        state: AssistantTaskState,
        result: VoiceToolResult,
        error: Option<String>,
    ) {
        let (snapshot, completion, listener_id) = {
            let mut tasks = self.tasks.borrow_mut();
            let Some(runtime) = tasks.get_mut(task_id) else {
                return;
            };
            if !runtime.snapshot.state.is_active() {
                return;
            }
            let now = now_millis();
            runtime.snapshot.state = state;
            runtime.snapshot.updated_at_ms = now;
            runtime.snapshot.completed_at_ms = Some(now);
            runtime.snapshot.result = Some(result.response.clone());
            runtime.snapshot.error = error;
            (
                runtime.snapshot.clone(),
                runtime.completion.take(),
                runtime.terminal_listener_id.take(),
            )
        };
        if let (Some(listener_id), Some(panel_id), Some(workspace)) = (
            listener_id,
            snapshot.target_panel_id.as_deref(),
            self.workspace.upgrade(),
        ) {
            if let Ok(view) = workspace.try_borrow() {
                if let Some(host) = view.host(panel_id) {
                    host.remove_terminal_runtime_listener(listener_id);
                }
            }
        }
        self.persist(&snapshot);
        self.notify(AssistantTaskEvent::Completed(snapshot.clone()));
        let Some(completion) = completion else {
            self.notify(AssistantTaskEvent::DeliveryRequired(snapshot));
            return;
        };

        let (delivery_ack, ack_receiver) = tokio::sync::oneshot::channel();
        if completion
            .send(VoiceToolCompletion {
                result,
                delivery_ack,
            })
            .is_err()
        {
            self.notify(AssistantTaskEvent::DeliveryRequired(snapshot));
            return;
        }
        self.track_delivery_ack(snapshot, ack_receiver);
    }

    fn track_delivery_ack(
        self: &Rc<Self>,
        task: AssistantTask,
        mut receiver: tokio::sync::oneshot::Receiver<()>,
    ) {
        let weak = Rc::downgrade(self);
        let deadline = now_millis() + DELIVERY_ACK_TIMEOUT_MS;
        gtk4::glib::timeout_add_local(Duration::from_millis(TASK_TICK_MS), move || match receiver
            .try_recv()
        {
            Ok(()) => gtk4::glib::ControlFlow::Break,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                if let Some(supervisor) = weak.upgrade() {
                    supervisor.notify(AssistantTaskEvent::DeliveryRequired(task.clone()));
                }
                gtk4::glib::ControlFlow::Break
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) if now_millis() >= deadline => {
                if let Some(supervisor) = weak.upgrade() {
                    supervisor.notify(AssistantTaskEvent::DeliveryRequired(task.clone()));
                }
                gtk4::glib::ControlFlow::Break
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                gtk4::glib::ControlFlow::Continue
            }
        });
    }

    fn task_status(&self, call: &VoiceToolCall) -> VoiceToolResult {
        let task_id = call
            .arguments
            .get("task_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let response = if let Some(task_id) = task_id {
            match self
                .tasks
                .borrow()
                .get(task_id)
                .map(|runtime| runtime.snapshot.clone())
            {
                Some(task) => serde_json::json!({ "status": "ok", "task": task }),
                None => serde_json::json!({
                    "status": "error",
                    "error": format!("Task '{task_id}' non trovato.")
                }),
            }
        } else {
            let tasks = self
                .tasks()
                .into_iter()
                .map(|mut task| {
                    task.result = None;
                    task
                })
                .collect::<Vec<_>>();
            serde_json::json!({ "status": "ok", "tasks": tasks })
        };
        VoiceToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            response,
        }
    }

    fn cancel_from_tool(self: &Rc<Self>, call: &VoiceToolCall) -> VoiceToolResult {
        let result = required_string(&call.arguments, "task_id").and_then(|task_id| {
            self.cancel(&task_id)
                .then(|| serde_json::json!({ "status": "ok", "task_id": task_id }))
                .ok_or_else(|| format!("Task '{task_id}' non trovato o gia' concluso."))
        });
        match result {
            Ok(response) => VoiceToolResult {
                call_id: call.id.clone(),
                name: call.name.clone(),
                response,
            },
            Err(error) => VoiceToolResult::error(call, error),
        }
    }

    fn is_active(&self, task_id: &str) -> bool {
        self.tasks
            .borrow()
            .get(task_id)
            .is_some_and(|runtime| runtime.snapshot.state.is_active())
    }

    fn persist(&self, task: &AssistantTask) {
        if let Err(error) =
            pax_db::Database::open(&self.db_path).and_then(|db| db.save_assistant_task(task))
        {
            tracing::warn!(%error, task_id = %task.id, "failed to persist assistant task");
        }
    }

    fn notify(&self, event: AssistantTaskEvent) {
        let listeners = self.listeners.borrow().clone();
        for listener in listeners {
            listener(event.clone());
        }
    }
}

fn required_string(arguments: &Value, field: &str) -> Result<String, String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Campo obbligatorio mancante: {field}."))
}

fn optional_u64(arguments: &Value, field: &str) -> Result<Option<u64>, String> {
    arguments
        .get(field)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| format!("{field} deve essere un intero non negativo."))
        })
        .transpose()
}

fn terminal_condition_met(
    condition: &AssistantTaskCondition,
    runtime: crate::panel_host::TerminalRuntimeSnapshot,
    now_ms: i64,
    content: Option<&str>,
) -> bool {
    match condition {
        AssistantTaskCondition::ShellPrompt { command_generation } => {
            *command_generation > 0
                && !runtime.busy
                && runtime.completed_generation >= *command_generation
        }
        AssistantTaskCondition::OutputChanged { after_revision } => {
            runtime.output_revision > *after_revision
        }
        AssistantTaskCondition::OutputQuiet {
            after_revision,
            quiet_ms,
        } => {
            runtime.output_revision > *after_revision
                && now_ms.saturating_sub(runtime.last_output_at_ms) >= *quiet_ms as i64
        }
        AssistantTaskCondition::ContainsText {
            text,
            case_sensitive,
            after_revision,
        } => {
            runtime.output_revision > *after_revision
                && content.is_some_and(|content| {
                    if *case_sensitive {
                        content.contains(text)
                    } else {
                        content.to_lowercase().contains(&text.to_lowercase())
                    }
                })
        }
    }
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

    #[test]
    fn task_conditions_have_stable_provider_names() {
        assert_eq!(
            serde_json::to_value(AssistantTaskCondition::OutputQuiet {
                after_revision: 7,
                quiet_ms: 900,
            })
            .unwrap()["type"],
            "output_quiet"
        );
    }

    #[test]
    fn numeric_arguments_reject_negative_values() {
        assert!(optional_u64(&serde_json::json!({"value": -1}), "value").is_err());
        assert_eq!(
            optional_u64(&serde_json::json!({"value": 42}), "value").unwrap(),
            Some(42)
        );
    }

    fn runtime(
        busy: bool,
        completed_generation: u64,
        output_revision: u64,
        last_output_at_ms: i64,
    ) -> crate::panel_host::TerminalRuntimeSnapshot {
        crate::panel_host::TerminalRuntimeSnapshot {
            busy,
            command_generation: completed_generation,
            completed_generation,
            output_revision,
            last_output_at_ms,
        }
    }

    #[test]
    fn shell_prompt_waits_for_the_requested_command_generation() {
        let condition = AssistantTaskCondition::ShellPrompt {
            command_generation: 3,
        };

        assert!(!terminal_condition_met(
            &condition,
            runtime(true, 2, 10, 900),
            1_000,
            None
        ));
        assert!(!terminal_condition_met(
            &condition,
            runtime(false, 2, 10, 900),
            1_000,
            None
        ));
        assert!(terminal_condition_met(
            &condition,
            runtime(false, 3, 10, 900),
            1_000,
            None
        ));
    }

    #[test]
    fn output_conditions_require_a_new_revision_and_their_specific_signal() {
        let changed = AssistantTaskCondition::OutputChanged { after_revision: 4 };
        assert!(!terminal_condition_met(
            &changed,
            runtime(false, 1, 4, 900),
            1_000,
            None
        ));
        assert!(terminal_condition_met(
            &changed,
            runtime(false, 1, 5, 900),
            1_000,
            None
        ));

        let quiet = AssistantTaskCondition::OutputQuiet {
            after_revision: 4,
            quiet_ms: 250,
        };
        assert!(!terminal_condition_met(
            &quiet,
            runtime(false, 1, 5, 900),
            1_000,
            None
        ));
        assert!(terminal_condition_met(
            &quiet,
            runtime(false, 1, 5, 700),
            1_000,
            None
        ));

        let contains = AssistantTaskCondition::ContainsText {
            text: "continue?".into(),
            case_sensitive: false,
            after_revision: 4,
        };
        assert!(terminal_condition_met(
            &contains,
            runtime(true, 1, 5, 900),
            1_000,
            Some("CONTINUE? [y/N]")
        ));
        assert!(!terminal_condition_met(
            &contains,
            runtime(true, 1, 4, 900),
            1_000,
            Some("continue? [y/N]")
        ));
    }
}
