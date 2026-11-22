use crate::{ApplicationError,
            ApplicationResult,
            ErrorKind,
            Guidance,
            proposals::{Ledger,
                        ProposalAudit,
                        ProposalOrigin}};
use gpui_agent_contracts::{ProposalStatus,
                           SkillProposal,
                           SkillProposalInput};
use gpui_agent_dci::proposal_governance::SkillProposalContext;
use gpui_agent_domain::McpScope;
use gpui_agent_ports::Clock;
use std::sync::Arc;

#[derive(Clone)]
pub struct SkillProposals(Ledger<SkillProposal>);

impl Default for SkillProposals {
    fn default() -> Self { Self(Ledger::new("sp", "skill")) }
}

fn invalid(message: impl Into<String>) -> ApplicationError { ApplicationError(ErrorKind::Invalid, message.into()) }

impl SkillProposals {
    pub fn with_audit(self, audit: Arc<dyn ProposalAudit>) -> Self { Self(self.0.with_audit(audit)) }

    pub fn with_clock(self, clock: Arc<dyn Clock>) -> Self { Self(self.0.with_clock(clock)) }

    pub fn propose(&self, scope: &McpScope, origin: ProposalOrigin, input: SkillProposalInput) -> ApplicationResult<SkillProposal> {
        let (skill_id, _) = SkillProposalContext::plan_import(&input).map_err(invalid)?;
        self.0.insert(scope, &origin, |id, expires_at| SkillProposal {
            id,
            skill_id,
            name: input.name,
            content: input.content,
            resources: input.resources,
            requested_during: origin.request.clone(),
            status: ProposalStatus::Pending,
            message: None,
            expires_at,
        })
    }

    pub fn status(&self, caller: Option<&McpScope>, id: &str) -> ApplicationResult<SkillProposal> { self.0.get(caller, id) }

    pub fn pending(&self) -> ApplicationResult<Vec<SkillProposal>> { self.0.pending() }

    pub fn origin(&self, id: &str) -> ApplicationResult<(McpScope, Option<i64>)> { self.0.origin(id) }

    pub fn reject(&self, id: &str) -> ApplicationResult<SkillProposal> { self.0.claim(id, ProposalStatus::Rejected) }

    pub async fn approve(&self, id: &str, guidance: &Guidance) -> ApplicationResult<SkillProposal> {
        let claimed = self.0.claim(id, ProposalStatus::Applying)?;
        let outcome = match SkillProposalContext::plan_import(&SkillProposalInput {
            name: claimed.name,
            content: claimed.content,
            resources: claimed.resources,
        }) {
            | Ok((_, request)) => guidance.import_skill(request).await.map(|_| ()),
            | Err(error) => Err(invalid(error)),
        };
        self.0.finish(id, outcome)
    }
}

pub fn skill_proposal_continuation(proposal: &SkillProposal) -> String { SkillProposalContext::continuation_prompt(proposal) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposals::{ProposalAudit,
                           ProposalOrigin};
    use std::sync::{Arc,
                    Mutex};

    #[derive(Default)]
    struct Audit(Mutex<Vec<serde_json::Value>>);
    impl ProposalAudit for Audit {
        fn record(&self, event: &serde_json::Value) { self.0.lock().unwrap().push(event.clone()); }
    }

    fn scope(workspace: &str) -> McpScope {
        McpScope {
            provider: gpui_agent_domain::Provider::Codex,
            account_id: "default".into(),
            workspace_path: workspace.into(),
        }
    }

    fn input(name: &str) -> SkillProposalInput {
        SkillProposalInput {
            name: name.into(),
            content: format!("---\nname: {name}\ndescription: Helper\n---\nInstructions"),
            resources: Default::default(),
        }
    }

    #[test]
    fn given_proposals_then_one_run_cannot_flood_the_operator_and_audit_omits_content() {
        let audit = Arc::new(Audit::default());
        let proposals = SkillProposals::default().with_audit(audit.clone());
        let origin = ProposalOrigin {
            conversation_id: Some(7),
            turn: Some(1),
            request: Some("secret user text".into()),
        };
        for i in 0 .. 4 {
            let created = proposals.propose(&scope("/work"), origin.clone(), input(&format!("skill-{i}"))).unwrap();
            proposals.reject(&created.id).unwrap();
        }
        assert!(proposals.propose(&scope("/work"), origin, input("skill-4")).is_err());
        let records = audit.0.lock().unwrap();
        assert_eq!(records.len(), 8);
        let serialized = serde_json::to_string(&*records).unwrap();
        assert!(!serialized.contains("Instructions"));
        assert!(!serialized.contains("secret user text"));
    }

    #[test]
    fn given_rejected_proposals_from_one_scope_then_pending_proposals_are_never_evicted() {
        let proposals = SkillProposals::default();
        let kept = proposals.propose(&scope("/work-a"), ProposalOrigin::default(), input("kept")).unwrap();
        for i in 0 .. 80 {
            let created = proposals
                .propose(&scope("/work-b"), ProposalOrigin::default(), input(&format!("drop-{i}")))
                .unwrap();
            proposals.reject(&created.id).unwrap();
        }
        assert_eq!(proposals.pending().unwrap().iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), [kept.id]);
    }
}
