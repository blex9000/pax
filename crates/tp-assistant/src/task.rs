use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssistantTaskState {
    Pending,
    Running,
    WaitingForInput,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
}

impl AssistantTaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::WaitingForInput => "waiting_for_input",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::WaitingForInput)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantTaskCondition {
    ShellPrompt {
        command_generation: u64,
    },
    OutputChanged {
        after_revision: u64,
    },
    OutputQuiet {
        after_revision: u64,
        quiet_ms: u64,
    },
    ContainsText {
        text: String,
        case_sensitive: bool,
        after_revision: u64,
    },
}

impl AssistantTaskCondition {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ShellPrompt { .. } => "attesa completamento comando",
            Self::OutputChanged { .. } => "attesa nuovo output",
            Self::OutputQuiet { .. } => "attesa schermata stabile",
            Self::ContainsText { .. } => "attesa testo nel terminale",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantTask {
    pub id: String,
    pub workspace_record_key: String,
    pub provider: String,
    pub provider_session_id: Option<String>,
    pub tool_call_id: String,
    pub tool_name: String,
    pub target_panel_id: Option<String>,
    pub label: String,
    pub state: AssistantTaskState,
    pub condition: AssistantTaskCondition,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deadline_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "task", rename_all = "snake_case")]
pub enum AssistantTaskEvent {
    Created(AssistantTask),
    Updated(AssistantTask),
    Completed(AssistantTask),
    DeliveryRequired(AssistantTask),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolContinuationMode {
    Synchronous,
    NativeNonBlocking,
    HostEvent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub continuation: ToolContinuationMode,
    pub supports_host_events: bool,
    pub persistent_session: bool,
}

impl ProviderCapabilities {
    pub fn for_provider(provider: &str) -> Self {
        match provider {
            // Gemini 3.1 Live currently requires the original tool call to
            // remain pending until Pax sends its matching function response.
            crate::ProviderId::GEMINI_LIVE => Self {
                continuation: ToolContinuationMode::Synchronous,
                supports_host_events: true,
                persistent_session: true,
            },
            crate::ProviderId::CODEX | crate::ProviderId::CLAUDE | crate::ProviderId::LOCAL => {
                Self {
                    continuation: ToolContinuationMode::HostEvent,
                    supports_host_events: true,
                    persistent_session: true,
                }
            }
            _ => Self {
                continuation: ToolContinuationMode::Synchronous,
                supports_host_events: false,
                persistent_session: false,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderTaskAdapter {
    pub provider: String,
    pub capabilities: ProviderCapabilities,
}

impl ProviderTaskAdapter {
    pub fn for_provider(provider: &str) -> Self {
        Self {
            provider: provider.to_string(),
            capabilities: ProviderCapabilities::for_provider(provider),
        }
    }

    pub fn completion_event(&self, task: &AssistantTask) -> Value {
        serde_json::json!({
            "event": "assistant_task_completed",
            "provider": self.provider,
            "continuation": self.capabilities.continuation,
            "task": task,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_task_states_are_distinct_from_terminal_states() {
        assert!(AssistantTaskState::Pending.is_active());
        assert!(AssistantTaskState::Running.is_active());
        assert!(AssistantTaskState::WaitingForInput.is_active());
        assert!(!AssistantTaskState::Succeeded.is_active());
        assert!(!AssistantTaskState::Interrupted.is_active());
    }

    #[test]
    fn provider_capabilities_leave_execution_outside_provider_implementations() {
        let gemini = ProviderCapabilities::for_provider(crate::ProviderId::GEMINI_LIVE);
        let codex = ProviderCapabilities::for_provider(crate::ProviderId::CODEX);
        let claude = ProviderCapabilities::for_provider(crate::ProviderId::CLAUDE);

        assert_eq!(gemini.continuation, ToolContinuationMode::Synchronous);
        assert_eq!(codex.continuation, ToolContinuationMode::HostEvent);
        assert_eq!(claude.continuation, ToolContinuationMode::HostEvent);
        assert!(gemini.supports_host_events);
        assert!(codex.persistent_session);
    }

    #[test]
    fn provider_adapter_builds_a_provider_neutral_completion_event() {
        let adapter = ProviderTaskAdapter::for_provider(crate::ProviderId::CODEX);
        let task = AssistantTask {
            id: "task-1".into(),
            workspace_record_key: "workspace-1".into(),
            provider: crate::ProviderId::CODEX.into(),
            provider_session_id: Some("session-1".into()),
            tool_call_id: "call-1".into(),
            tool_name: "terminal_wait".into(),
            target_panel_id: Some("terminal-1".into()),
            label: "Build".into(),
            state: AssistantTaskState::Succeeded,
            condition: AssistantTaskCondition::ShellPrompt {
                command_generation: 1,
            },
            created_at_ms: 1,
            updated_at_ms: 2,
            deadline_at_ms: 10,
            completed_at_ms: Some(2),
            result: None,
            error: None,
        };

        let event = adapter.completion_event(&task);

        assert_eq!(event["provider"], crate::ProviderId::CODEX);
        assert_eq!(event["continuation"], "host_event");
        assert_eq!(event["task"]["id"], "task-1");
    }
}
