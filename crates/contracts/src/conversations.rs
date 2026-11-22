use crate::{ConversationArchiveFilter,
            ConversationCursor,
            Job,
            Provider};
use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, Deserialize)]
pub struct ConversationTarget {
    pub provider: Provider,
    pub account: Option<String>,
    pub workspace: Option<i64>,
}

fn default_conversation_limit() -> usize { 20 }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationListRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub query: Option<String>,
    #[serde(default)]
    pub archived: ConversationArchiveFilter,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_conversation_limit")]
    pub limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationReadRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub id: i64,
    #[serde(default)]
    pub cursor: ConversationCursor,
    #[serde(default = "default_conversation_limit")]
    pub limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationRenameRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub id: i64,
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationOrganizeRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub id: i64,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationSendRequest {
    #[serde(default)]
    pub skills: Vec<crate::SkillReference>,
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub id: Option<i64>,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<crate::Attachment>,
    #[serde(default)]
    pub attachment_ids: Vec<i64>,
    pub model: Option<String>,
    pub command_id: String,
}

fn default_wait_timeout() -> u64 { 1000 }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationWaitRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub id: i64,
    pub job_id: Option<i64>,
    #[serde(default = "default_wait_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationInterruptRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub id: i64,
    pub job_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ConversationWaitResult {
    pub job: Option<Job>,
    pub timed_out: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationForkRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub id: i64,
    pub through_message_id: Option<i64>,
    pub title: Option<String>,
    pub command_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationMergeBackRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub source_id: i64,
    pub target_id: i64,
    pub through_message_id: Option<i64>,
    pub command_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTransfersRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub id: i64,
    #[serde(default)]
    pub after: i64,
}
