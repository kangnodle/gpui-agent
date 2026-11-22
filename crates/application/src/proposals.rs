use crate::{ApplicationError,
            ApplicationResult,
            ErrorKind,
            host_defaults};
use gpui_agent_contracts::ProposalStatus;
use gpui_agent_dci::proposal_governance::{LedgerOutcome,
                                          ProposalLedgerContext,
                                          ProposalRecord};
use gpui_agent_domain::{McpScope,
                        ProposalAuditEvent,
                        ProposalLedgerError,
                        ProposalLedgerState};
use gpui_agent_ports::Clock;
use std::sync::{Arc,
                Mutex};

#[derive(Clone, Debug, Default)]
pub struct ProposalOrigin {
    pub conversation_id: Option<i64>,
    pub turn: Option<u64>,
    pub request: Option<String>,
}

pub trait ProposalAudit: Send + Sync {
    fn record(&self, event: &serde_json::Value);
}

struct Inner<T> {
    clock: Arc<dyn Clock>,
    audit: Arc<dyn ProposalAudit>,
    state: ProposalLedgerState<T>,
}

pub(crate) struct Ledger<T>(Arc<Mutex<Inner<T>>>);

impl<T> Clone for Ledger<T> {
    fn clone(&self) -> Self { Self(self.0.clone()) }
}

fn map_error(error: ProposalLedgerError) -> ApplicationError {
    match error {
        | ProposalLedgerError::NotFound => ApplicationError(ErrorKind::NotFound, "Proposal unavailable".into()),
        | ProposalLedgerError::NotPending => ApplicationError(ErrorKind::Busy, "Proposal is no longer pending".into()),
        | ProposalLedgerError::TooManyUndecidedInScope =>
            ApplicationError(ErrorKind::Busy, "Too many undecided proposals; wait for the operator to decide".into()),
        | ProposalLedgerError::TooManyInTurn => ApplicationError(
            ErrorKind::Busy,
            "This run has made too many proposals; finish your task with what is available or ask the user".into(),
        ),
        | ProposalLedgerError::LedgerFull => ApplicationError(ErrorKind::Busy, "Too many proposals".into()),
        | ProposalLedgerError::InvalidTransition => ApplicationError(ErrorKind::Invalid, "Invalid proposal transition".into()),
    }
}

fn emit_audits(audit: &dyn ProposalAudit, events: &[ProposalAuditEvent]) {
    for event in events {
        audit.record(&serde_json::json!({
            "event": "agent_proposal",
            "kind": event.kind,
            "proposal": event.proposal_id,
            "outcome": event.outcome_label(),
            "provider": event.scope.provider,
            "account": event.scope.account_id,
            "conversation_id": event.conversation_id,
        }));
    }
}

impl<T: ProposalRecord> Ledger<T> {
    pub(crate) fn new(prefix: &'static str, kind: &'static str) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            clock: host_defaults::default_clock(),
            audit: host_defaults::default_audit(),
            state: ProposalLedgerState::new(prefix, kind),
        })))
    }

    pub(crate) fn with_audit(self, audit: Arc<dyn ProposalAudit>) -> Self {
        if let Ok(mut inner) = self.0.lock() {
            inner.audit = audit;
        }
        self
    }

    pub(crate) fn with_clock(self, clock: Arc<dyn Clock>) -> Self {
        if let Ok(mut inner) = self.0.lock() {
            inner.clock = clock;
        }
        self
    }

    fn apply<R>(&self, step: impl FnOnce(ProposalLedgerState<T>, u64) -> LedgerOutcome<T, R>) -> ApplicationResult<R> {
        let mut inner = self
            .0
            .lock()
            .map_err(|_| ApplicationError(ErrorKind::Failure, "Proposal registry unavailable".into()))?;
        let now = inner.clock.now_unix_secs();
        let audit = inner.audit.clone();
        let placeholder = ProposalLedgerState::new(inner.state.prefix, inner.state.kind);
        let current = std::mem::replace(&mut inner.state, placeholder);
        let outcome = step(current, now);
        inner.state = outcome.state;
        emit_audits(&*audit, &outcome.audits);
        outcome.result.map_err(map_error)
    }

    pub(crate) fn insert(&self, scope: &McpScope, origin: &ProposalOrigin, make: impl FnOnce(String, u64) -> T) -> ApplicationResult<T> {
        self.apply(|state, now| ProposalLedgerContext::submit(state, scope, origin.conversation_id, origin.turn, now, make))
    }

    pub(crate) fn get(&self, caller: Option<&McpScope>, id: &str) -> ApplicationResult<T> {
        self.apply(|state, now| ProposalLedgerContext::get(state, caller, id, now))
    }

    pub(crate) fn pending(&self) -> ApplicationResult<Vec<T>> { self.apply(ProposalLedgerContext::pending) }

    pub(crate) fn origin(&self, id: &str) -> ApplicationResult<(McpScope, Option<i64>)> {
        self.apply(|state, now| ProposalLedgerContext::origin(state, id, now))
    }

    pub(crate) fn claim(&self, id: &str, status: ProposalStatus) -> ApplicationResult<T> {
        self.apply(|state, now| ProposalLedgerContext::claim(state, id, status, now))
    }

    pub(crate) fn finish(&self, id: &str, outcome: ApplicationResult<()>) -> ApplicationResult<T> {
        let failure_message = outcome.as_ref().err().map(|error| error.1.clone());
        let settled = self.apply(|state, now| ProposalLedgerContext::finish(state, id, failure_message, now))?;
        outcome.map(|()| settled)
    }
}
