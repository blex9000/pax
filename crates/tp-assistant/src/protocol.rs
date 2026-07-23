use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    pub const GEMINI_LIVE: &'static str = "gemini_live";
    pub const CODEX: &'static str = "codex";
    pub const CLAUDE: &'static str = "claude";
    pub const LOCAL: &'static str = "local";

    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let value = value.trim();
        (!value.is_empty()).then(|| Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssistantSessionState {
    Idle,
    Listening,
    Thinking,
    Acting,
    Speaking,
    Interrupted,
    Failed,
    Closed,
}

impl AssistantSessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Listening => "listening",
            Self::Thinking => "thinking",
            Self::Acting => "acting",
            Self::Speaking => "speaking",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssistantRole {
    System,
    User,
    Assistant,
    Tool,
}

impl AssistantRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessage {
    pub role: AssistantRole,
    pub content: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallResult {
    pub call_id: String,
    pub result: Value,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum AssistantEvent {
    StateChanged(AssistantSessionState),
    UserTranscript { text: String, is_final: bool },
    AssistantText { text: String, is_final: bool },
    AudioLevel(f64),
    AudioChunk(Vec<u8>),
    ToolCall(ToolCall),
    ToolResult(ToolCallResult),
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_are_trimmed_and_extensible() {
        assert_eq!(
            ProviderId::new(" custom-provider ").unwrap().as_str(),
            "custom-provider"
        );
        assert!(ProviderId::new("   ").is_none());
    }
}
