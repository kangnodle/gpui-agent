use gpui_agent_contracts::{McpEnvironmentCheck,
                           McpProposal,
                           McpProposalInput,
                           McpScope,
                           McpServer,
                           PluginImportRequest,
                           ProposalAuditEvent,
                           ProposalLedgerEntry,
                           ProposalLedgerError,
                           ProposalLedgerState,
                           ProposalLifecycle,
                           ProposalStatus,
                           ProposalTransitionError,
                           ProposalTransitionInput,
                           SavedConversation,
                           SkillImportRequest,
                           SkillProposal,
                           SkillProposalInput,
                           transition_proposal,
                           validate_mcp,
                           validate_plugin_id};
use std::collections::BTreeMap;

const REQUEST_CONTEXT_CHARS: usize = 500;

pub trait ProposalRecord: Clone {
    fn id(&self) -> &str;
    fn status(&self) -> ProposalStatus;
    fn expires_at(&self) -> u64;
    fn message(&self) -> Option<&str>;
    fn with_lifecycle(&self, lifecycle: &ProposalLifecycle) -> Self;
}

impl ProposalRecord for SkillProposal {
    fn id(&self) -> &str { &self.id }

    fn status(&self) -> ProposalStatus { self.status }

    fn expires_at(&self) -> u64 { self.expires_at }

    fn message(&self) -> Option<&str> { self.message.as_deref() }

    fn with_lifecycle(&self, lifecycle: &ProposalLifecycle) -> Self {
        let mut next = self.clone();
        next.status = lifecycle.status();
        next.message = lifecycle.message();
        next
    }
}

impl ProposalRecord for McpProposal {
    fn id(&self) -> &str { &self.id }

    fn status(&self) -> ProposalStatus { self.status }

    fn expires_at(&self) -> u64 { self.expires_at }

    fn message(&self) -> Option<&str> { self.message.as_deref() }

    fn with_lifecycle(&self, lifecycle: &ProposalLifecycle) -> Self {
        let mut next = self.clone();
        next.status = lifecycle.status();
        next.message = lifecycle.message();
        next
    }
}

#[derive(Clone, Debug)]
pub struct LedgerOutcome<T, R> {
    pub state: ProposalLedgerState<T>,
    pub audits: Vec<ProposalAuditEvent>,
    pub result: Result<R, ProposalLedgerError>,
}

pub struct ProposalLedgerContext;

impl ProposalLedgerContext {
    fn audit_event<T: ProposalRecord>(kind: &'static str, entry: &ProposalLedgerEntry<T>) -> ProposalAuditEvent {
        ProposalAuditEvent {
            kind,
            proposal_id: entry.proposal.id().to_owned(),
            status: entry.proposal.status(),
            scope: entry.scope.clone(),
            conversation_id: entry.conversation_id,
        }
    }

    fn sweep_expired<T: ProposalRecord>(mut state: ProposalLedgerState<T>, now: u64) -> (ProposalLedgerState<T>, Vec<ProposalAuditEvent>) {
        let mut audits = Vec::new();
        let kind = state.kind;
        for entry in &mut state.entries {
            let current = ProposalLifecycle::from_status(
                entry.proposal.status(),
                entry.proposal.expires_at(),
                entry.proposal.message().map(str::to_owned),
            );
            if let Ok(next) = transition_proposal(
                current,
                ProposalTransitionInput::Expire {
                    now,
                },
            ) && next.status() != entry.proposal.status()
            {
                entry.proposal = entry.proposal.with_lifecycle(&next);
                audits.push(Self::audit_event(kind, entry));
            }
        }
        (state, audits)
    }

    pub fn submit<T: ProposalRecord>(
        state: ProposalLedgerState<T>,
        scope: &McpScope,
        conversation_id: Option<i64>,
        turn: Option<u64>,
        now: u64,
        make: impl FnOnce(String, u64) -> T,
    ) -> LedgerOutcome<T, T> {
        let (mut state, mut audits) = Self::sweep_expired(state, now);
        let pending = state
            .entries
            .iter()
            .filter(|entry| entry.scope == *scope && entry.proposal.status() == ProposalStatus::Pending)
            .count();
        if pending >= state.limits.pending_per_scope {
            return LedgerOutcome {
                state,
                audits,
                result: Err(ProposalLedgerError::TooManyUndecidedInScope),
            };
        }
        if turn.is_some() && state.entries.iter().filter(|entry| entry.scope == *scope && entry.turn == turn).count() >= state.limits.per_turn {
            return LedgerOutcome {
                state,
                audits,
                result: Err(ProposalLedgerError::TooManyInTurn),
            };
        }
        if state.entries.len() >= state.limits.retained {
            let Some(oldest) = state.entries.iter().position(|entry| {
                !ProposalLifecycle::from_status(
                    entry.proposal.status(),
                    entry.proposal.expires_at(),
                    entry.proposal.message().map(str::to_owned),
                )
                .is_active()
            }) else {
                return LedgerOutcome {
                    state,
                    audits,
                    result: Err(ProposalLedgerError::LedgerFull),
                };
            };
            state.entries.remove(oldest);
        }
        state.next_sequence += 1;
        let id = format!("{}{}", state.prefix, state.next_sequence);
        let proposal = make(id, now.saturating_add(state.limits.lifetime_seconds));
        let entry = ProposalLedgerEntry {
            scope: scope.clone(),
            conversation_id,
            turn,
            proposal: proposal.clone(),
        };
        audits.push(Self::audit_event(state.kind, &entry));
        state.entries.push(entry);
        LedgerOutcome {
            state,
            audits,
            result: Ok(proposal),
        }
    }

    pub fn get<T: ProposalRecord>(state: ProposalLedgerState<T>, caller: Option<&McpScope>, id: &str, now: u64) -> LedgerOutcome<T, T> {
        let (state, audits) = Self::sweep_expired(state, now);
        let result = state
            .entries
            .iter()
            .find(|entry| entry.proposal.id() == id && caller.is_none_or(|scope| *scope == entry.scope))
            .map(|entry| entry.proposal.clone())
            .ok_or(ProposalLedgerError::NotFound);
        LedgerOutcome {
            state,
            audits,
            result,
        }
    }

    pub fn pending<T: ProposalRecord>(state: ProposalLedgerState<T>, now: u64) -> LedgerOutcome<T, Vec<T>> {
        let (state, audits) = Self::sweep_expired(state, now);
        let list = state
            .entries
            .iter()
            .filter(|entry| entry.proposal.status() == ProposalStatus::Pending)
            .map(|entry| entry.proposal.clone())
            .collect();
        LedgerOutcome {
            state,
            audits,
            result: Ok(list),
        }
    }

    pub fn origin<T: ProposalRecord>(state: ProposalLedgerState<T>, id: &str, now: u64) -> LedgerOutcome<T, (McpScope, Option<i64>)> {
        let (state, audits) = Self::sweep_expired(state, now);
        let result = state
            .entries
            .iter()
            .find(|entry| entry.proposal.id() == id)
            .map(|entry| (entry.scope.clone(), entry.conversation_id))
            .ok_or(ProposalLedgerError::NotFound);
        LedgerOutcome {
            state,
            audits,
            result,
        }
    }

    pub fn claim<T: ProposalRecord>(state: ProposalLedgerState<T>, id: &str, target: ProposalStatus, now: u64) -> LedgerOutcome<T, T> {
        let (mut state, mut audits) = Self::sweep_expired(state, now);
        let kind = state.kind;
        let Some(entry) = state.entries.iter_mut().find(|entry| entry.proposal.id() == id) else {
            return LedgerOutcome {
                state,
                audits,
                result: Err(ProposalLedgerError::NotFound),
            };
        };
        let current = ProposalLifecycle::from_status(
            entry.proposal.status(),
            entry.proposal.expires_at(),
            entry.proposal.message().map(str::to_owned),
        );
        match transition_proposal(
            current,
            ProposalTransitionInput::Claim {
                target,
            },
        ) {
            | Ok(next) => {
                entry.proposal = entry.proposal.with_lifecycle(&next);
                let proposal = entry.proposal.clone();
                audits.push(Self::audit_event(kind, entry));
                LedgerOutcome {
                    state,
                    audits,
                    result: Ok(proposal),
                }
            },
            | Err(ProposalTransitionError::NotPending) => LedgerOutcome {
                state,
                audits,
                result: Err(ProposalLedgerError::NotPending),
            },
            | Err(ProposalTransitionError::InvalidClaimTarget) => LedgerOutcome {
                state,
                audits,
                result: Err(ProposalLedgerError::InvalidTransition),
            },
        }
    }

    pub fn finish<T: ProposalRecord>(state: ProposalLedgerState<T>, id: &str, failure_message: Option<String>, now: u64) -> LedgerOutcome<T, T> {
        let (mut state, mut audits) = Self::sweep_expired(state, now);
        let kind = state.kind;
        let Some(entry) = state.entries.iter_mut().find(|entry| entry.proposal.id() == id) else {
            return LedgerOutcome {
                state,
                audits,
                result: Err(ProposalLedgerError::NotFound),
            };
        };
        let current = ProposalLifecycle::from_status(
            entry.proposal.status(),
            entry.proposal.expires_at(),
            entry.proposal.message().map(str::to_owned),
        );
        let input = match failure_message {
            | None => ProposalTransitionInput::SettleApplied,
            | Some(message) => ProposalTransitionInput::SettleFailed {
                message,
            },
        };
        match transition_proposal(current, input) {
            | Ok(next) => {
                entry.proposal = entry.proposal.with_lifecycle(&next);
                let proposal = entry.proposal.clone();
                audits.push(Self::audit_event(kind, entry));
                LedgerOutcome {
                    state,
                    audits,
                    result: Ok(proposal),
                }
            },
            | Err(_) => LedgerOutcome {
                state,
                audits,
                result: Err(ProposalLedgerError::InvalidTransition),
            },
        }
    }
}

pub struct SkillProposalContext;

impl SkillProposalContext {
    pub fn plan_import(input: &SkillProposalInput) -> Result<(String, SkillImportRequest), String> {
        let name = &input.name;
        if name.is_empty() || name.len() > 64 || name.starts_with(['.', '-']) || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) {
            return Err("Skill name must be 1-64 ASCII letters, digits, '-' or '_'".into());
        }
        if input.resources.contains_key("SKILL.md") {
            return Err("SKILL.md is the content field, not a resource".into());
        }
        let id = format!("repo:.agents/skills/{name}/SKILL.md");
        let mut files = BTreeMap::new();
        files.insert("SKILL.md".to_owned(), input.content.clone().into_bytes());
        files.extend(input.resources.iter().map(|(path, text)| (path.clone(), text.clone().into_bytes())));
        let request = SkillImportRequest {
            id: id.clone(),
            files,
        };
        request.validate().map_err(str::to_owned)?;
        Ok((id, request))
    }

    pub fn continuation_prompt(proposal: &SkillProposal) -> String {
        format!(
            "[gpui-agent] Your skill proposal {} ({}) was approved by the operator and installed as {}. It is available now: call host_skills_list and host_skills_read to use it, then continue the original task.",
            proposal.id, proposal.name, proposal.skill_id
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpHostFacts {
    pub executable_exists: bool,
    pub environment_ready: BTreeMap<String, bool>,
}

pub struct McpProposalContext;

impl McpProposalContext {
    pub fn environment_variables(server: &McpServer) -> Vec<&str> {
        match server {
            | McpServer::Stdio {
                ..
            } => vec![],
            | McpServer::Http {
                bearer_token_env_var,
                env_http_headers,
                ..
            } => bearer_token_env_var
                .iter()
                .map(String::as_str)
                .chain(env_http_headers.values().map(String::as_str))
                .collect(),
        }
    }

    pub fn plan_import(input: &McpProposalInput, facts: &McpHostFacts) -> Result<(String, PluginImportRequest), String> {
        let rationale = input.rationale.trim();
        if rationale.is_empty() || input.rationale.len() > 1000 {
            return Err("Explain in 1-1000 characters why this server is needed".into());
        }
        match &input.server {
            | McpServer::Http {
                oauth: Some(_), ..
            } => {
                return Err(
                    "OAuth servers need an interactive login, so they cannot be proposed. Ask the operator to register the server in MCP settings and complete the login there"
                        .into(),
                );
            },
            | McpServer::Stdio {
                ..
            } if !facts.executable_exists => {
                return Err("MCP executable does not exist on the host".into());
            },
            | _ => {},
        }
        if let Some(variable) = Self::environment_variables(&input.server)
            .into_iter()
            .find(|variable| !facts.environment_ready.get(*variable).copied().unwrap_or(false))
        {
            return Err(format!(
                "Environment variable {variable} is not set on the host or has an unsupported value. Ask the operator to set it before the host starts; never put a secret value in a proposal"
            ));
        }
        validate_mcp(std::slice::from_ref(&input.server))?;
        let name = format!("mcp-{}", input.server.name().to_ascii_lowercase().replace('_', "-"));
        let plugin_id = format!("repo:{name}");
        validate_plugin_id(&plugin_id).map_err(str::to_owned)?;
        let manifest = serde_json::json!({
            "name": name,
            "description": format!("MCP server {} proposed by an agent: {rationale}", input.server.name()),
            "version": "1",
            "mcp_servers": [&input.server],
        });
        let request = PluginImportRequest {
            scope: "repo".into(),
            files: [(
                "plugin.json".to_owned(),
                serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
            )]
            .into(),
        };
        request.validate()?;
        Ok((plugin_id, request))
    }

    pub fn annotate_environment(server: &McpServer, environment_ready: &BTreeMap<String, bool>) -> Vec<McpEnvironmentCheck> {
        Self::environment_variables(server)
            .into_iter()
            .map(|variable| McpEnvironmentCheck {
                variable: variable.to_owned(),
                ready: environment_ready.get(variable).copied().unwrap_or(false),
            })
            .collect()
    }

    pub fn continuation_prompt(proposal: &McpProposal) -> String {
        format!(
            "[gpui-agent] Your MCP server proposal {} ({}) was approved by the operator and enabled as plugin {}. The server starts with this turn and its tools are available now; continue the original task.",
            proposal.id,
            proposal.server.name(),
            proposal.plugin_id
        )
    }
}

pub struct ProposalOriginContext;

impl ProposalOriginContext {
    pub fn latest_user_request(conversation: &SavedConversation) -> Option<String> {
        let text: String = conversation
            .messages
            .iter()
            .rev()
            .find(|message| message.user)
            .map(|message| {
                message
                    .text
                    .chars()
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .take(REQUEST_CONTEXT_CHARS)
                    .collect()
            })
            .unwrap_or_default();
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!normalized.is_empty()).then_some(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_agent_contracts::Provider;

    fn scope() -> McpScope {
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: "/work".into(),
        }
    }

    #[test]
    fn ledger_context_expires_and_emits_pure_audit_events_deterministically() {
        let state = ProposalLedgerState::<SkillProposal>::new("sp", "skill");
        let submitted = ProposalLedgerContext::submit(state, &scope(), Some(1), Some(1), 100, |id, expires_at| SkillProposal {
            id,
            skill_id: "repo:.agents/skills/demo/SKILL.md".into(),
            name: "demo".into(),
            content: "---\nname: demo\ndescription: Demo\n---\nbody".into(),
            resources: BTreeMap::new(),
            requested_during: None,
            status: ProposalStatus::Pending,
            message: None,
            expires_at,
        });
        let proposal = submitted.result.unwrap();
        assert_eq!(submitted.audits.len(), 1);
        assert_eq!(submitted.audits[0].outcome_label(), "proposed");

        let expired = ProposalLedgerContext::claim(submitted.state, &proposal.id, ProposalStatus::Applying, 100 + 1800);
        assert_eq!(expired.result.unwrap_err(), ProposalLedgerError::NotPending);
        assert_eq!(expired.audits.len(), 1);
        assert_eq!(expired.audits[0].outcome_label(), "expired");
    }
}
