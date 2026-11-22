use crate::{Attachment,
            SavedConversation,
            SnapshotError,
            conversation_snapshot,
            normalize_title,
            validate_command_id};
use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextTransferCommand {
    pub source_id: i64,
    pub target_id: Option<i64>,
    pub through_message_id: Option<i64>,
    pub title: Option<String>,
    pub command_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextTransferKind {
    Fork,
    MergeBack,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextTransferState {
    Pending,
    Attached,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContextTransfer {
    pub id: i64,
    pub kind: ContextTransferKind,
    pub source_id: i64,
    pub target_id: i64,
    pub through_message_id: i64,
    pub state: ContextTransferState,
    pub attached_message_id: Option<i64>,
    pub created_at: i64,
}

pub struct ContextTransferPlan {
    pub source: SavedConversation,
    pub context: Vec<Attachment>,
    pub through_message_id: i64,
    pub prefix_count: usize,
}

#[derive(Debug)]
pub enum ContextTransferError {
    NotFound,
    Conflict,
    Unrelated,
    Invalid(&'static str),
    Snapshot(SnapshotError),
}
impl std::fmt::Display for ContextTransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            | Self::NotFound => f.write_str("Conversation not found in this scope"),
            | Self::Conflict => f.write_str("Command ID was already used with different input"),
            | Self::Unrelated => f.write_str("Merge target must be an ancestor of the source branch"),
            | Self::Invalid(message) => f.write_str(message),
            | Self::Snapshot(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for ContextTransferError {}
impl From<SnapshotError> for ContextTransferError {
    fn from(error: SnapshotError) -> Self { Self::Snapshot(error) }
}
impl From<&'static str> for ContextTransferError {
    fn from(message: &'static str) -> Self { Self::Invalid(message) }
}

impl ContextTransferCommand {
    pub fn validate(&self) -> Result<(), ContextTransferError> {
        validate_command_id(&self.command_id)?;
        if self.source_id <= 0 || self.target_id.is_some_and(|id| id <= 0 || id == self.source_id) || self.through_message_id.is_some_and(|id| id <= 0) {
            return Err("Positive, distinct conversation IDs and a positive message boundary are required".into());
        }
        if let Some(title) = &self.title {
            normalize_title(title)?;
        }
        if self.target_id.is_some() && self.title.is_some() {
            return Err("Only a fork can set a title".into());
        }
        Ok(())
    }

    pub fn prepare(&self, mut source: SavedConversation, ancestor_prefix: Option<usize>) -> Result<ContextTransferPlan, ContextTransferError> {
        self.validate()?;
        let end = match self.through_message_id {
            | Some(id) => source
                .messages
                .iter()
                .position(|message| message.id == Some(id))
                .map(|index| index + 1)
                .ok_or(SnapshotError::MessageNotFound)?,
            | None => source.messages.len(),
        };
        let boundary = end
            .checked_sub(1)
            .and_then(|index| source.messages.get(index))
            .ok_or("Source conversation is empty")?;
        if boundary.user {
            return Err("Choose a completed assistant message as the boundary".into());
        }
        if boundary.incomplete {
            return Err(SnapshotError::Incomplete.into());
        }
        let through_message_id = boundary.id.ok_or("Source message has no durable ID")?;
        let skip = if self.target_id.is_some() {
            ancestor_prefix.ok_or(ContextTransferError::Unrelated)?
        } else {
            0
        };
        if skip >= end {
            return Err("No branch messages exist after the ancestor's fork boundary".into());
        }
        source.messages.truncate(end);
        source.messages.drain(.. skip);
        let context = conversation_snapshot(&source, None)?;
        source.session_id = None;
        source
            .timeline
            .retain(|entry| matches!(entry.event, crate::Event::Tool { .. }) && source.messages.iter().any(|message| message.id == Some(entry.message_id)));
        Ok(ContextTransferPlan {
            source,
            context,
            through_message_id,
            prefix_count: end,
        })
    }
}
