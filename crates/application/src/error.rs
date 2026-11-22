//! Application error classification, independent of transport status codes.
use gpui_agent_domain::{ContextTransferError,
                        QueueError,
                        RevisionError,
                        SnapshotError};

#[derive(Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Invalid,
    NotFound,
    Busy,
    Unavailable,
    Failure,
}

#[derive(Debug)]
pub struct ApplicationError(pub ErrorKind, pub String);
pub type ApplicationResult<T> = Result<T, ApplicationError>;

impl std::fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.1) }
}
impl std::error::Error for ApplicationError {}
impl From<anyhow::Error> for ApplicationError {
    fn from(error: anyhow::Error) -> Self {
        match error.downcast::<ContextTransferError>() {
            | Ok(error) => error.into(),
            | Err(error) => Self(ErrorKind::Failure, error.to_string()),
        }
    }
}
impl From<ContextTransferError> for ApplicationError {
    fn from(error: ContextTransferError) -> Self {
        if let ContextTransferError::Snapshot(error) = error {
            return error.into();
        }
        let kind = match error {
            | ContextTransferError::NotFound => ErrorKind::NotFound,
            | ContextTransferError::Conflict => ErrorKind::Busy,
            | _ => ErrorKind::Invalid,
        };
        Self(kind, error.to_string())
    }
}
impl From<&'static str> for ApplicationError {
    fn from(message: &'static str) -> Self { Self(ErrorKind::Invalid, message.into()) }
}

impl From<SnapshotError> for ApplicationError {
    fn from(error: SnapshotError) -> Self {
        let kind = match error {
            | SnapshotError::MessageNotFound => ErrorKind::NotFound,
            | SnapshotError::Incomplete => ErrorKind::Busy,
            | SnapshotError::Invalid(_) => ErrorKind::Invalid,
            | SnapshotError::Encoding(_) => ErrorKind::Failure,
        };
        Self(kind, error.to_string())
    }
}

impl From<RevisionError> for ApplicationError {
    fn from(error: RevisionError) -> Self {
        match error {
            | RevisionError::Busy => Self(ErrorKind::Busy, error.to_string()),
            | RevisionError::Invalid(message) => message.into(),
            | RevisionError::Snapshot(error) => error.into(),
        }
    }
}

impl From<QueueError> for ApplicationError {
    fn from(error: QueueError) -> Self { Self(ErrorKind::Busy, error.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_errors_preserve_application_error_kinds() {
        for (error, kind) in [(QueueError::Full, ErrorKind::Busy), (QueueError::AccountFull, ErrorKind::Busy)] {
            let message = error.to_string();
            let error = ApplicationError::from(error);
            assert_eq!(error.0, kind);
            assert_eq!(error.1, message);
        }
    }

    #[test]
    fn snapshot_errors_preserve_application_error_kinds() {
        for (error, kind) in [
            (SnapshotError::MessageNotFound, ErrorKind::NotFound),
            (SnapshotError::Incomplete, ErrorKind::Busy),
            (SnapshotError::Invalid("invalid context"), ErrorKind::Invalid),
            (
                SnapshotError::Encoding(serde_json::from_str::<serde_json::Value>("{").unwrap_err()),
                ErrorKind::Failure,
            ),
        ] {
            let message = error.to_string();
            let error = ApplicationError::from(error);
            assert_eq!(error.0, kind);
            assert_eq!(error.1, message);
        }
    }

    #[test]
    fn revision_errors_preserve_application_error_kinds() {
        for (error, kind) in [
            (RevisionError::Busy, ErrorKind::Busy),
            (RevisionError::Invalid("No question to edit"), ErrorKind::Invalid),
            (RevisionError::Snapshot(SnapshotError::Incomplete), ErrorKind::Busy),
        ] {
            let message = error.to_string();
            let error = ApplicationError::from(error);
            assert_eq!(error.0, kind);
            assert_eq!(error.1, message);
        }
    }
}
