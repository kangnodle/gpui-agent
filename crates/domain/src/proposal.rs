use crate::McpScope;
use serde::{Deserialize,
            Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Applying,
    Applied,
    Rejected,
    Failed,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalLifecycle {
    Pending { expires_at: u64 },
    Applying,
    Applied,
    Rejected,
    Failed { message: Option<String> },
    Expired,
}

impl ProposalLifecycle {
    pub fn from_status(status: ProposalStatus, expires_at: u64, message: Option<String>) -> Self {
        match status {
            | ProposalStatus::Pending => Self::Pending {
                expires_at,
            },
            | ProposalStatus::Applying => Self::Applying,
            | ProposalStatus::Applied => Self::Applied,
            | ProposalStatus::Rejected => Self::Rejected,
            | ProposalStatus::Failed => Self::Failed {
                message,
            },
            | ProposalStatus::Expired => Self::Expired,
        }
    }

    pub fn status(&self) -> ProposalStatus {
        match self {
            | Self::Pending {
                ..
            } => ProposalStatus::Pending,
            | Self::Applying => ProposalStatus::Applying,
            | Self::Applied => ProposalStatus::Applied,
            | Self::Rejected => ProposalStatus::Rejected,
            | Self::Failed {
                ..
            } => ProposalStatus::Failed,
            | Self::Expired => ProposalStatus::Expired,
        }
    }

    pub fn message(&self) -> Option<String> {
        match self {
            | Self::Failed {
                message,
            } => message.clone(),
            | _ => None,
        }
    }

    pub fn is_active(&self) -> bool { matches!(self, Self::Pending { .. } | Self::Applying) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalTransitionInput {
    Expire { now: u64 },
    Claim { target: ProposalStatus },
    SettleApplied,
    SettleFailed { message: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalTransitionError {
    NotPending,
    InvalidClaimTarget,
}

pub fn transition_proposal(state: ProposalLifecycle, input: ProposalTransitionInput) -> Result<ProposalLifecycle, ProposalTransitionError> {
    match (state, input) {
        | (
            ProposalLifecycle::Pending {
                expires_at,
            },
            ProposalTransitionInput::Expire {
                now,
            },
        ) =>
            if expires_at <= now {
                Ok(ProposalLifecycle::Expired)
            } else {
                Ok(ProposalLifecycle::Pending {
                    expires_at,
                })
            },
        | (
            other,
            ProposalTransitionInput::Expire {
                ..
            },
        ) => Ok(other),
        | (
            ProposalLifecycle::Pending {
                ..
            },
            ProposalTransitionInput::Claim {
                target: ProposalStatus::Applying,
            },
        ) => Ok(ProposalLifecycle::Applying),
        | (
            ProposalLifecycle::Pending {
                ..
            },
            ProposalTransitionInput::Claim {
                target: ProposalStatus::Rejected,
            },
        ) => Ok(ProposalLifecycle::Rejected),
        | (
            ProposalLifecycle::Pending {
                ..
            },
            ProposalTransitionInput::Claim {
                ..
            },
        ) => Err(ProposalTransitionError::InvalidClaimTarget),
        | (
            _,
            ProposalTransitionInput::Claim {
                ..
            },
        ) => Err(ProposalTransitionError::NotPending),
        | (_, ProposalTransitionInput::SettleApplied) => Ok(ProposalLifecycle::Applied),
        | (
            _,
            ProposalTransitionInput::SettleFailed {
                message,
            },
        ) => Ok(ProposalLifecycle::Failed {
            message: Some(message),
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProposalLedgerLimits {
    pub pending_per_scope: usize,
    pub per_turn: usize,
    pub retained: usize,
    pub lifetime_seconds: u64,
}

impl Default for ProposalLedgerLimits {
    fn default() -> Self {
        Self {
            pending_per_scope: 8,
            per_turn: 4,
            retained: 64,
            lifetime_seconds: 30 * 60,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalAuditEvent {
    pub kind: &'static str,
    pub proposal_id: String,
    pub status: ProposalStatus,
    pub scope: McpScope,
    pub conversation_id: Option<i64>,
}

impl ProposalAuditEvent {
    pub fn outcome_label(&self) -> &'static str {
        match self.status {
            | ProposalStatus::Pending => "proposed",
            | ProposalStatus::Applying => "applying",
            | ProposalStatus::Applied => "applied",
            | ProposalStatus::Rejected => "rejected",
            | ProposalStatus::Failed => "failed",
            | ProposalStatus::Expired => "expired",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalLedgerError {
    NotFound,
    NotPending,
    TooManyUndecidedInScope,
    TooManyInTurn,
    LedgerFull,
    InvalidTransition,
}

#[derive(Clone, Debug)]
pub struct ProposalLedgerEntry<T> {
    pub scope: McpScope,
    pub conversation_id: Option<i64>,
    pub turn: Option<u64>,
    pub proposal: T,
}

#[derive(Clone, Debug)]
pub struct ProposalLedgerState<T> {
    pub prefix: &'static str,
    pub kind: &'static str,
    pub limits: ProposalLedgerLimits,
    pub next_sequence: u64,
    pub entries: Vec<ProposalLedgerEntry<T>>,
}

impl<T> ProposalLedgerState<T> {
    pub fn new(prefix: &'static str, kind: &'static str) -> Self {
        Self {
            prefix,
            kind,
            limits: ProposalLedgerLimits::default(),
            next_sequence: 0,
            entries: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposal_lifecycle_enforces_valid_adt_transitions() {
        let pending = ProposalLifecycle::Pending {
            expires_at: 100,
        };
        assert_eq!(
            transition_proposal(
                pending.clone(),
                ProposalTransitionInput::Expire {
                    now: 99,
                }
            ),
            Ok(pending.clone())
        );
        assert_eq!(
            transition_proposal(
                pending.clone(),
                ProposalTransitionInput::Expire {
                    now: 100,
                }
            ),
            Ok(ProposalLifecycle::Expired)
        );
        let applying = transition_proposal(
            pending.clone(),
            ProposalTransitionInput::Claim {
                target: ProposalStatus::Applying,
            },
        )
        .unwrap();
        assert_eq!(applying, ProposalLifecycle::Applying);
        assert_eq!(
            transition_proposal(
                applying.clone(),
                ProposalTransitionInput::Claim {
                    target: ProposalStatus::Rejected,
                }
            ),
            Err(ProposalTransitionError::NotPending)
        );
        assert_eq!(
            transition_proposal(applying, ProposalTransitionInput::SettleApplied),
            Ok(ProposalLifecycle::Applied)
        );
        let failed = transition_proposal(
            ProposalLifecycle::Applying,
            ProposalTransitionInput::SettleFailed {
                message: "boom".into(),
            },
        )
        .unwrap();
        assert_eq!(failed.status(), ProposalStatus::Failed);
        assert_eq!(failed.message().as_deref(), Some("boom"));
    }
}
