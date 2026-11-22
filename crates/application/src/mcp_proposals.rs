use crate::{ApplicationError,
            ApplicationResult,
            ErrorKind,
            Plugins,
            proposals::{Ledger,
                        ProposalAudit,
                        ProposalOrigin}};
use gpui_agent_contracts::{McpProposal,
                           McpProposalInput,
                           ProposalStatus};
use gpui_agent_dci::proposal_governance::{McpHostFacts,
                                          McpProposalContext};
use gpui_agent_domain::{McpScope,
                        McpServer};
use gpui_agent_ports::Clock;
use std::{collections::BTreeMap,
          sync::Arc};

#[derive(Clone)]
pub struct McpProposals(Ledger<McpProposal>);

impl Default for McpProposals {
    fn default() -> Self { Self(Ledger::new("mp", "mcp")) }
}

fn invalid(message: impl Into<String>) -> ApplicationError { ApplicationError(ErrorKind::Invalid, message.into()) }

pub trait McpHost: Sync {
    fn executable_exists(&self, command: &str) -> bool;

    fn environment_ready(&self, variable: &str) -> bool;
}

fn collect_host_facts(server: &McpServer, host: &dyn McpHost) -> McpHostFacts {
    let executable_exists = match server {
        | McpServer::Stdio {
            command, ..
        } => host.executable_exists(command),
        | McpServer::Http {
            ..
        } => true,
    };
    let environment_ready: BTreeMap<String, bool> = McpProposalContext::environment_variables(server)
        .into_iter()
        .map(|variable| (variable.to_owned(), host.environment_ready(variable)))
        .collect();
    McpHostFacts {
        executable_exists,
        environment_ready,
    }
}

impl McpProposals {
    pub fn with_audit(self, audit: Arc<dyn ProposalAudit>) -> Self { Self(self.0.with_audit(audit)) }

    pub fn with_clock(self, clock: Arc<dyn Clock>) -> Self { Self(self.0.with_clock(clock)) }

    pub fn propose(&self, scope: &McpScope, origin: ProposalOrigin, input: McpProposalInput, host: &dyn McpHost) -> ApplicationResult<McpProposal> {
        let facts = collect_host_facts(&input.server, host);
        let (plugin_id, _) = McpProposalContext::plan_import(&input, &facts).map_err(invalid)?;
        self.0.insert(scope, &origin, |id, expires_at| McpProposal {
            id,
            plugin_id,
            server: input.server,
            rationale: input.rationale,
            environment: vec![],
            requested_during: origin.request.clone(),
            status: ProposalStatus::Pending,
            message: None,
            expires_at,
        })
    }

    pub fn status(&self, caller: Option<&McpScope>, id: &str) -> ApplicationResult<McpProposal> { self.0.get(caller, id) }

    pub fn pending(&self, host: &dyn McpHost) -> ApplicationResult<Vec<McpProposal>> {
        let mut proposals = self.0.pending()?;
        for proposal in &mut proposals {
            let facts = collect_host_facts(&proposal.server, host);
            proposal.environment = McpProposalContext::annotate_environment(&proposal.server, &facts.environment_ready);
        }
        Ok(proposals)
    }

    pub fn origin(&self, id: &str) -> ApplicationResult<(McpScope, Option<i64>)> { self.0.origin(id) }

    pub fn reject(&self, id: &str) -> ApplicationResult<McpProposal> { self.0.claim(id, ProposalStatus::Rejected) }

    pub async fn approve(&self, id: &str, plugins: &Plugins, host: &dyn McpHost) -> ApplicationResult<McpProposal> {
        let claimed = self.0.claim(id, ProposalStatus::Applying)?;
        let outcome = async {
            let input = McpProposalInput {
                server: claimed.server,
                rationale: claimed.rationale,
            };
            let facts = collect_host_facts(&input.server, host);
            let (plugin_id, request) = McpProposalContext::plan_import(&input, &facts).map_err(invalid)?;
            plugins.import(request).await?;
            if let Err(error) = plugins.set_enabled_reviewed(&plugin_id, true, Some(vec![input.server])).await {
                let _ = plugins.remove(&plugin_id).await;
                return Err(error);
            }
            Ok(())
        }
        .await;
        self.0.finish(id, outcome)
    }
}

pub fn mcp_proposal_continuation(proposal: &McpProposal) -> String { McpProposalContext::continuation_prompt(proposal) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::PluginSource;
    use gpui_agent_domain::{PluginCatalog,
                            PluginImportRequest,
                            PluginSyncResult};
    use std::sync::{Arc,
                    atomic::{AtomicBool,
                             Ordering}};

    struct Host {
        environment: bool,
    }
    impl McpHost for Host {
        fn executable_exists(&self, _: &str) -> bool { true }

        fn environment_ready(&self, _: &str) -> bool { self.environment }
    }

    struct Untouched(Arc<AtomicBool>);
    impl PluginSource for Untouched {
        fn catalog(&self) -> anyhow::Result<PluginCatalog> {
            self.0.store(true, Ordering::SeqCst);
            anyhow::bail!("must not be called")
        }

        fn import(&self, _: &PluginImportRequest) -> anyhow::Result<PluginCatalog> { self.catalog() }

        fn set_enabled_reviewed(&self, _: &str, _: bool, _: Option<&[McpServer]>) -> anyhow::Result<PluginCatalog> { self.catalog() }

        fn remove(&self, _: &str) -> anyhow::Result<PluginCatalog> { self.catalog() }

        fn sync_claude(&self, _: &str, _: bool) -> anyhow::Result<PluginSyncResult> { self.catalog().map(|_| PluginSyncResult::default()) }
    }

    #[tokio::test]
    async fn given_a_ready_proposal_when_the_host_environment_changes_before_approval_then_nothing_is_installed() {
        let scope = McpScope {
            provider: gpui_agent_domain::Provider::Codex,
            account_id: "default".into(),
            workspace_path: "/work".into(),
        };
        let proposals = McpProposals::default();
        let input = || McpProposalInput {
            server: McpServer::Http {
                name: "remote".into(),
                url: "https://example.com/mcp".into(),
                bearer_token_env_var: Some("REMOTE_TOKEN".into()),
                env_http_headers: Default::default(),
                oauth: None,
            },
            rationale: "Needs the API".into(),
        };
        let proposal = proposals
            .propose(
                &scope,
                Default::default(),
                input(),
                &Host {
                    environment: true,
                },
            )
            .unwrap();
        let touched = Arc::new(AtomicBool::new(false));
        let plugins = Plugins::new(Arc::new(Untouched(touched.clone())));
        let error = proposals
            .approve(
                &proposal.id,
                &plugins,
                &Host {
                    environment: false,
                },
            )
            .await
            .unwrap_err();
        assert!(error.1.contains("REMOTE_TOKEN"), "{}", error.1);
        assert!(!touched.load(Ordering::SeqCst), "A refused approval must not touch installed plugins");
        let status = proposals.status(Some(&scope), &proposal.id).unwrap();
        assert_eq!(status.status, ProposalStatus::Failed);
        assert!(
            proposals
                .propose(
                    &scope,
                    Default::default(),
                    input(),
                    &Host {
                        environment: false
                    }
                )
                .is_err()
        );
    }
}
