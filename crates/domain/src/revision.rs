//! Pure preparation of a revised question; persistence rechecks the source
//! atomically.
use crate::{Attachment,
            ChatRequest,
            ClaudePermissionMode,
            SavedConversation,
            SnapshotError,
            conversation_snapshot,
            validate_attachments};
use serde::{Deserialize,
            Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviseRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<crate::ConversationExtensions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<crate::SkillReference>>,
    pub message_id: i64,
    pub text: String,
    pub model: Option<String>,
    pub claude_permission_mode: Option<ClaudePermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_execution_policy: Option<crate::CodexExecutionPolicy>,
}

#[derive(Debug)]
pub struct RevisionPlan {
    pub request: ChatRequest,
    pub context: Vec<Attachment>,
}

#[derive(Debug)]
pub enum RevisionError {
    Busy,
    Invalid(&'static str),
    Snapshot(SnapshotError),
}

impl std::fmt::Display for RevisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            | Self::Busy => f.write_str("Only the last completed question can be edited"),
            | Self::Invalid(message) => f.write_str(message),
            | Self::Snapshot(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for RevisionError {}
impl From<&'static str> for RevisionError {
    fn from(message: &'static str) -> Self { Self::Invalid(message) }
}
impl From<SnapshotError> for RevisionError {
    fn from(error: SnapshotError) -> Self { Self::Snapshot(error) }
}

impl SavedConversation {
    pub fn prepare_revision(mut self, revision: ReviseRequest) -> Result<RevisionPlan, RevisionError> {
        let index = self.messages.iter().rposition(|message| message.user).ok_or("No question to edit")?;
        if self.messages[index].id != Some(revision.message_id) || self.messages.len() <= index + 1 || self.messages.iter().any(|message| message.incomplete) {
            return Err(RevisionError::Busy);
        }
        let request = ChatRequest {
            extensions: revision
                .extensions
                .or_else(|| self.messages[index].guidance.as_ref().and_then(|receipt| receipt.extensions.clone())),
            skills: revision.skills.unwrap_or_else(|| self.messages[index].skills.clone()),
            computer_use: false,
            text: revision.text,
            attachments: self.messages[index].attachments.clone(),
            model: revision.model,
            claude_permission_mode: revision.claude_permission_mode,
            codex_execution_policy: revision.codex_execution_policy,
            session_id: None,
            conversation_id: None,
        };
        request.validate()?;
        self.messages.truncate(index);
        let context = if index == 0 { vec![] } else { conversation_snapshot(&self, None)? };
        let mut combined = request.attachments.clone();
        combined.extend(context.iter().cloned());
        validate_attachments(&combined)?;
        Ok(RevisionPlan {
            request,
            context,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SavedMessage;

    fn saved() -> SavedConversation {
        SavedConversation {
            id: 9,
            model: None,
            session_id: Some("old-session".into()),
            timeline: vec![],
            messages: ["earlier question", "earlier answer", "old question", "old answer"]
                .into_iter()
                .enumerate()
                .map(|(index, text)| SavedMessage {
                    skills: vec![],
                    guidance: None,
                    id: Some(index as i64 + 1),
                    user: index % 2 == 0,
                    text: text.into(),
                    incomplete: false,
                    attachments: vec![],
                })
                .collect(),
        }
    }

    fn request() -> ReviseRequest {
        ReviseRequest {
            extensions: None,
            skills: None,
            message_id: 3,
            text: "edited question".into(),
            model: Some("new-model".into()),
            claude_permission_mode: Some(ClaudePermissionMode::Plan),
            codex_execution_policy: Some(crate::CodexExecutionPolicy {
                approval_policy: crate::CodexApprovalPolicy::OnRequest,
                sandbox: crate::CodexSandbox::WorkspaceWrite,
            }),
        }
    }

    #[test]
    fn revision_replays_only_the_prefix_and_preserves_question_attachments() {
        let mut source = saved();
        let attachment = Attachment::Text {
            name: "question.txt".into(),
            text: "question file".into(),
            source: None,
        };
        source.messages[2].attachments.push(attachment.clone());
        let plan = source.prepare_revision(request()).unwrap();
        assert_eq!(plan.request.attachments, vec![attachment]);
        assert_eq!(plan.request.text, "edited question");
        assert_eq!(plan.request.model.as_deref(), Some("new-model"));
        assert_eq!(plan.request.codex_execution_policy, request().codex_execution_policy);
        assert_eq!(plan.request.claude_permission_mode, Some(ClaudePermissionMode::Plan));
        assert!(plan.request.session_id.is_none() && plan.request.conversation_id.is_none());
        let context = plan.context[0].text_context().unwrap();
        assert!(context.contains("earlier question") && context.contains("earlier answer"));
        assert!(!context.contains("old question") && !context.contains("old answer") && !context.contains("old-session"));

        let mut source = saved();
        source.messages.truncate(2);
        let plan = source
            .prepare_revision(ReviseRequest {
                extensions: None,
                message_id: 1,
                ..request()
            })
            .unwrap();
        assert!(plan.context.is_empty());
    }

    #[test]
    fn revision_rejects_stale_unfinished_invalid_and_oversized_inputs() {
        assert!(matches!(
            saved().prepare_revision(ReviseRequest {
                extensions: None,
                message_id: 1,
                ..request()
            }),
            Err(RevisionError::Busy)
        ));
        let mut source = saved();
        source.messages[1].incomplete = true;
        assert!(matches!(source.prepare_revision(request()), Err(RevisionError::Busy)));
        let mut source = saved();
        source.messages.pop();
        assert!(matches!(source.prepare_revision(request()), Err(RevisionError::Busy)));
        let mut source = saved();
        source.messages.clear();
        assert!(matches!(source.prepare_revision(request()), Err(RevisionError::Invalid("No question to edit"))));
        assert!(matches!(
            saved().prepare_revision(ReviseRequest {
                extensions: None,
                text: "".into(),
                ..request()
            }),
            Err(RevisionError::Invalid(_))
        ));
        let mut source = saved();
        source.messages[2].attachments = vec![
            Attachment::Text {
                name: "file.txt".into(),
                text: "text".into(),
                source: None
            };
            8
        ];
        assert!(matches!(source.prepare_revision(request()), Err(RevisionError::Invalid(_))));
    }
}
