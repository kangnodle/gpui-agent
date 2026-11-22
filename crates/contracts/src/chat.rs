use gpui_agent_domain::{AuthStatus,
                        Event};
use serde::{Deserialize,
            Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct RenameRequest {
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PreparedRevision {
    pub conversation: gpui_agent_domain::SavedConversation,
    pub request: gpui_agent_domain::ChatRequest,
    pub versions: Vec<gpui_agent_domain::ConversationVersion>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LoginRequest {
    #[serde(default)]
    pub device_code: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    Interaction {
        operation_id: u64,
        request_id: u64,
        event: Event,
    },
    Started {
        operation_id: u64,
    },
    Conversation {
        conversation_id: i64,
    },
    Authenticated {
        status: AuthStatus,
    },
    Completed {
        session_id: String,
        text: String,
    },
    Error {
        message: String,
    },
    Cancelled,
    #[serde(untagged)]
    Agent(Event),
}

impl StreamEvent {
    pub fn is_terminal(&self) -> bool { matches!(self, Self::Authenticated { .. } | Self::Completed { .. } | Self::Error { .. } | Self::Cancelled) }
}
