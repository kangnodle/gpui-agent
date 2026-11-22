use serde::{Deserialize,
            Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}
impl JobState {
    pub fn terminal(self) -> bool { matches!(self, Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobMachine {
    Queued {
        operation_id: Option<u64>,
        conversation_id: Option<i64>,
    },
    Running {
        operation_id: Option<u64>,
        conversation_id: Option<i64>,
    },
    WaitingForInteraction {
        operation_id: Option<u64>,
        conversation_id: Option<i64>,
        request_id: u64,
    },
    Completed {
        operation_id: Option<u64>,
        conversation_id: Option<i64>,
    },
    Failed {
        operation_id: Option<u64>,
        conversation_id: Option<i64>,
        error: String,
    },
    Cancelled {
        operation_id: Option<u64>,
        conversation_id: Option<i64>,
    },
    Interrupted {
        operation_id: Option<u64>,
        conversation_id: Option<i64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobTransitionInput {
    OperationStarted { operation_id: u64 },
    ConversationBound { conversation_id: i64 },
    InteractionRequested { request_id: u64 },
    InteractionResolved,
    Completed,
    Failed { message: String },
    Cancelled,
    Passthrough,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobTransitionOutcome {
    Accepted(JobMachine),
    IgnoredTerminal(JobMachine),
}

impl JobMachine {
    pub fn from_job(job: &Job) -> Self {
        match job.state {
            | JobState::Queued => Self::Queued {
                operation_id: job.operation_id,
                conversation_id: job.conversation_id,
            },
            | JobState::Running => Self::Running {
                operation_id: job.operation_id,
                conversation_id: job.conversation_id,
            },
            | JobState::Waiting => match job.pending_request_id {
                | Some(request_id) => Self::WaitingForInteraction {
                    operation_id: job.operation_id,
                    conversation_id: job.conversation_id,
                    request_id,
                },
                | None => Self::Running {
                    operation_id: job.operation_id,
                    conversation_id: job.conversation_id,
                },
            },
            | JobState::Completed => Self::Completed {
                operation_id: job.operation_id,
                conversation_id: job.conversation_id,
            },
            | JobState::Failed => Self::Failed {
                operation_id: job.operation_id,
                conversation_id: job.conversation_id,
                error: job.error.clone().unwrap_or_default(),
            },
            | JobState::Cancelled => Self::Cancelled {
                operation_id: job.operation_id,
                conversation_id: job.conversation_id,
            },
            | JobState::Interrupted => Self::Interrupted {
                operation_id: job.operation_id,
                conversation_id: job.conversation_id,
            },
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed { .. } | Self::Cancelled { .. } | Self::Interrupted { .. }
        )
    }

    pub fn state(&self) -> JobState {
        match self {
            | Self::Queued {
                ..
            } => JobState::Queued,
            | Self::Running {
                ..
            } => JobState::Running,
            | Self::WaitingForInteraction {
                ..
            } => JobState::Waiting,
            | Self::Completed {
                ..
            } => JobState::Completed,
            | Self::Failed {
                ..
            } => JobState::Failed,
            | Self::Cancelled {
                ..
            } => JobState::Cancelled,
            | Self::Interrupted {
                ..
            } => JobState::Interrupted,
        }
    }

    pub fn operation_id(&self) -> Option<u64> {
        match self {
            | Self::Queued {
                operation_id, ..
            }
            | Self::Running {
                operation_id, ..
            }
            | Self::WaitingForInteraction {
                operation_id, ..
            }
            | Self::Completed {
                operation_id, ..
            }
            | Self::Failed {
                operation_id, ..
            }
            | Self::Cancelled {
                operation_id, ..
            }
            | Self::Interrupted {
                operation_id, ..
            } => *operation_id,
        }
    }

    pub fn conversation_id(&self) -> Option<i64> {
        match self {
            | Self::Queued {
                conversation_id, ..
            }
            | Self::Running {
                conversation_id, ..
            }
            | Self::WaitingForInteraction {
                conversation_id, ..
            }
            | Self::Completed {
                conversation_id, ..
            }
            | Self::Failed {
                conversation_id, ..
            }
            | Self::Cancelled {
                conversation_id, ..
            }
            | Self::Interrupted {
                conversation_id, ..
            } => *conversation_id,
        }
    }

    pub fn pending_request_id(&self) -> Option<u64> {
        match self {
            | Self::WaitingForInteraction {
                request_id, ..
            } => Some(*request_id),
            | _ => None,
        }
    }

    pub fn error(&self) -> Option<String> {
        match self {
            | Self::Failed {
                error, ..
            } => Some(error.clone()),
            | _ => None,
        }
    }

    pub fn project_onto(&self, job: &Job) -> Job {
        let mut next = job.clone();
        next.state = self.state();
        next.operation_id = self.operation_id();
        next.conversation_id = self.conversation_id();
        next.pending_request_id = self.pending_request_id();
        next.error = self.error().or_else(|| job.error.clone());
        next
    }
}

pub fn transition_job(state: JobMachine, input: JobTransitionInput) -> JobTransitionOutcome {
    let operation_id = state.operation_id();
    let conversation_id = state.conversation_id();
    match (state, input) {
        | (
            terminal @ (JobMachine::Completed {
                ..
            }
            | JobMachine::Failed {
                ..
            }
            | JobMachine::Cancelled {
                ..
            }
            | JobMachine::Interrupted {
                ..
            }),
            _,
        ) => JobTransitionOutcome::IgnoredTerminal(terminal),
        | (
            JobMachine::WaitingForInteraction {
                conversation_id,
                request_id,
                ..
            },
            JobTransitionInput::OperationStarted {
                operation_id,
            },
        ) => JobTransitionOutcome::Accepted(JobMachine::WaitingForInteraction {
            operation_id: Some(operation_id),
            conversation_id,
            request_id,
        }),
        | (
            JobMachine::Queued {
                conversation_id, ..
            },
            JobTransitionInput::OperationStarted {
                operation_id,
            },
        ) => JobTransitionOutcome::Accepted(JobMachine::Queued {
            operation_id: Some(operation_id),
            conversation_id,
        }),
        | (
            JobMachine::Running {
                ..
            },
            JobTransitionInput::OperationStarted {
                operation_id,
            },
        ) => JobTransitionOutcome::Accepted(JobMachine::Running {
            operation_id: Some(operation_id),
            conversation_id,
        }),
        | (
            JobMachine::WaitingForInteraction {
                operation_id,
                request_id,
                ..
            },
            JobTransitionInput::ConversationBound {
                conversation_id,
            },
        ) => JobTransitionOutcome::Accepted(JobMachine::WaitingForInteraction {
            operation_id,
            conversation_id: Some(conversation_id),
            request_id,
        }),
        | (
            JobMachine::Queued {
                operation_id, ..
            },
            JobTransitionInput::ConversationBound {
                conversation_id,
            },
        ) => JobTransitionOutcome::Accepted(JobMachine::Queued {
            operation_id,
            conversation_id: Some(conversation_id),
        }),
        | (
            JobMachine::Running {
                ..
            },
            JobTransitionInput::ConversationBound {
                conversation_id,
            },
        ) => JobTransitionOutcome::Accepted(JobMachine::Running {
            operation_id,
            conversation_id: Some(conversation_id),
        }),
        | (
            _,
            JobTransitionInput::InteractionRequested {
                request_id,
            },
        ) => JobTransitionOutcome::Accepted(JobMachine::WaitingForInteraction {
            operation_id,
            conversation_id,
            request_id,
        }),
        | (_, JobTransitionInput::InteractionResolved) => JobTransitionOutcome::Accepted(JobMachine::Running {
            operation_id,
            conversation_id,
        }),
        | (_, JobTransitionInput::Completed) => JobTransitionOutcome::Accepted(JobMachine::Completed {
            operation_id,
            conversation_id,
        }),
        | (
            _,
            JobTransitionInput::Failed {
                message,
            },
        ) => JobTransitionOutcome::Accepted(JobMachine::Failed {
            operation_id,
            conversation_id,
            error: message,
        }),
        | (_, JobTransitionInput::Cancelled) => JobTransitionOutcome::Accepted(JobMachine::Cancelled {
            operation_id,
            conversation_id,
        }),
        | (current, JobTransitionInput::Passthrough) => JobTransitionOutcome::Accepted(current),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProfile {
    pub id: String,
    pub provider: crate::Provider,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobScope {
    pub provider: crate::Provider,
    pub account_id: String,
    pub workspace_id: Option<i64>,
    pub workspace_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub scope: JobScope,
    pub state: JobState,
    pub conversation_id: Option<i64>,
    pub operation_id: Option<u64>,
    pub pending_request_id: Option<u64>,
    pub last_sequence: i64,
    pub preview: String,
    pub cancel_requested: bool,
    pub error: Option<String>,
    pub created_at: i64,
}

pub fn validate_account_id(id: &str) -> Result<(), &'static str> {
    if id == "default" || (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())) {
        Ok(())
    } else {
        Err("Invalid account profile ID")
    }
}

pub fn validate_command_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty() || id.len() > 128 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')) {
        Err("Command ID must contain 1–128 ASCII letters, digits, hyphens, underscores, colons or periods")
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<crate::ConversationExtensions>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<crate::SkillReference>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub computer_use: bool,
    #[serde(default)]
    pub attachments: Vec<crate::Attachment>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_permission_mode: Option<crate::runtime::ClaudePermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_execution_policy: Option<crate::CodexExecutionPolicy>,

    pub text: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub conversation_id: Option<i64>,
}

fn is_false(value: &bool) -> bool { !value }

impl ChatRequest {
    pub fn validate_provider(&self, provider: crate::Provider) -> Result<(), &'static str> {
        if let Some(policy) = self.codex_execution_policy {
            policy.validate_provider(provider)?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(selection) = &self.extensions {
            selection.validate()?;
        }
        crate::validate_skill_references(&self.skills)?;
        crate::validate_message(&self.text, self.session_id.as_deref(), &self.attachments)
    }

    pub fn validate_with_uploads(&self, ids: &[i64]) -> Result<(), &'static str> {
        if let Some(selection) = &self.extensions {
            selection.validate()?;
        }
        crate::validate_skill_references(&self.skills)?;
        if ids.len() + self.attachments.len() > crate::attachment::MAX_ATTACHMENTS || ids.iter().any(|id| *id <= 0) {
            return Err("At most 8 inline and uploaded attachments with positive IDs are allowed");
        }
        if !ids.is_empty() && self.text.trim().is_empty() {
            if self.text.len() > crate::attachment::MAX_TEXT_BYTES {
                return Err("Message exceeds 256 KiB");
            }
            crate::validate_message("attachment", self.session_id.as_deref(), &self.attachments)
        } else {
            self.validate()
        }
    }
}
