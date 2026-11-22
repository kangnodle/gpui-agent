use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversationSummary {
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub execution_state: Option<crate::JobState>,
    pub id: i64,
    pub title: String,
    pub updated_at: i64,
    #[serde(default)]
    pub matched_message: Option<MatchedMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatchedMessage {
    pub message_id: i64,
    pub user: bool,
    pub preview: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedMessage {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<crate::SkillReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<crate::GuidanceReceipt>,
    #[serde(default)]
    pub attachments: Vec<crate::Attachment>,

    #[serde(default)]
    pub id: Option<i64>,
    pub user: bool,
    pub text: String,
    pub incomplete: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub message_id: i64,
    pub event: crate::Event,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedConversation {
    #[serde(default)]
    pub timeline: Vec<TimelineEntry>,
    pub id: i64,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub messages: Vec<SavedMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversationVersion {
    pub conversation_id: i64,
    pub question_index: usize,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationState {
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
}
