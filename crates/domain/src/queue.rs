//! Queue policy over supplied state; the repository enforces it while locked.
use crate::ChatRequest;

#[derive(Debug, PartialEq, Eq)]
pub enum QueueError {
    Full,
    AccountFull,
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            | Self::Full => "Queue has reached 256 pending jobs",
            | Self::AccountFull => "Account queue has reached 32 pending jobs",
        })
    }
}
impl std::error::Error for QueueError {}

pub fn validate_queue_capacity(pending: i64, account_pending: i64) -> Result<(), QueueError> {
    if pending >= 256 {
        return Err(QueueError::Full);
    }
    if account_pending >= 32 {
        return Err(QueueError::AccountFull);
    }
    Ok(())
}

impl ChatRequest {
    pub fn preview(&self) -> String {
        crate::conversation_title(if self.text.trim().is_empty() {
            self.attachments.first().map_or("Attachment", |attachment| attachment.name())
        } else {
            &self.text
        })
    }
}
