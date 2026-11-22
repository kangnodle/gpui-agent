use crate::{Attachment,
            ConversationTarget};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentUploadRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub attachment: Attachment,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAttachmentRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentListRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentDeleteRequest {
    #[serde(flatten)]
    pub target: ConversationTarget,
    pub id: i64,
}
