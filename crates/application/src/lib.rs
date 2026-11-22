#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

mod attachments;
mod browser;
mod computer;
mod context_transfers;
mod conversations;
mod devices;
mod error;
mod guidance;
mod host_defaults;
mod job_events;
mod jobs;
mod mcp;
mod mcp_proposals;
mod operations;
mod plugins;
pub mod ports;
mod project_verification;
mod proposal_continuation;
mod proposals;
mod pull_requests;
mod questions;
mod skill_proposals;
mod use_cases;
mod workspaces;

use anyhow::Result;
pub use browser::automate_browser;
pub use computer::computer_status;
pub use devices::{control_device,
                  grant_device};
pub use error::{ApplicationError,
                ApplicationResult,
                ErrorKind};
use gpui_agent_domain::{Provider,
                        provider_index};
pub use guidance::Guidance;
pub use job_events::{apply_job_event,
                     project_job_event};
pub use jobs::{AccountScope,
               Jobs};
pub use mcp_proposals::{McpHost,
                        McpProposals,
                        mcp_proposal_continuation};
pub use plugins::Plugins;
use ports::{AgentRuntime,
            Clock,
            ConversationRepository,
            DiagnosticSink};
pub use proposals::{ProposalAudit,
                    ProposalOrigin};
pub use skill_proposals::{SkillProposals,
                          skill_proposal_continuation};
use std::{collections::HashMap,
          sync::{Arc,
                 Mutex,
                 atomic::AtomicU64}};
use tokio::sync::oneshot;
pub use workspaces::Workspaces;

struct Slot {
    agent: Option<Arc<dyn AgentRuntime>>,
    issue: Option<String>,
}

struct PendingInteraction {
    operation_id: u64,
    event: gpui_agent_domain::Event,
    sender: oneshot::Sender<gpui_agent_domain::InteractionResponse>,
}

struct Inner {
    automatic_computer: bool,
    inputs: Mutex<HashMap<u64, tokio::sync::mpsc::Sender<gpui_agent_ports::RuntimeInput>>>,
    interactions: Mutex<HashMap<u64, PendingInteraction>>,
    providers: [Slot; gpui_agent_contracts::PROVIDERS.len()],
    operations: Mutex<HashMap<u64, oneshot::Sender<()>>>,
    next_id: AtomicU64,
    history: Arc<dyn ConversationRepository>,
    clock: Arc<dyn Clock>,
    diagnostics: Arc<dyn DiagnosticSink>,
    files: Option<Arc<dyn ports::FileContextReader>>,
    guidance: Option<Guidance>,
    plugins: Option<Plugins>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        for (_, cancel) in (match self.operations.get_mut() {
            | Ok(guard) => guard,
            | Err(error) => {
                self.diagnostics
                    .warn(&format!("Operation registry lock poisoned; recovering entries for cleanup: {error}"));
                error.into_inner()
            },
        })
        .drain()
        {
            let _ = cancel.send(());
        }
        (match self.interactions.get_mut() {
            | Ok(guard) => guard,
            | Err(error) => {
                self.diagnostics
                    .warn(&format!("Interaction registry lock poisoned; recovering entries for cleanup: {error}"));
                error.into_inner()
            },
        })
        .clear();
    }
}

#[derive(Clone)]
pub struct Application(Arc<Inner>);

impl Application {
    pub fn new(agents: [Result<Arc<dyn AgentRuntime>>; gpui_agent_contracts::PROVIDERS.len()], history: Arc<dyn ConversationRepository>) -> Self {
        let clock = host_defaults::default_clock();
        let diagnostics = host_defaults::default_diagnostics();
        let initial_id = clock.now_unix_nanos();
        let providers = agents.map(|agent| match agent {
            | Ok(agent) => Slot {
                agent: Some(agent),
                issue: None,
            },
            | Err(error) => Slot {
                agent: None,
                issue: Some(error.to_string()),
            },
        });
        Self(Arc::new(Inner {
            automatic_computer: false,
            inputs: Mutex::new(HashMap::new()),
            providers,
            operations: Mutex::new(HashMap::new()),
            interactions: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(initial_id),
            history,
            clock,
            diagnostics,
            files: None,
            guidance: None,
            plugins: None,
        }))
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        let inner = Arc::get_mut(&mut self.0).expect("configure before cloning");
        inner.next_id = AtomicU64::new(clock.now_unix_nanos());
        inner.clock = clock;
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Arc<dyn DiagnosticSink>) -> Self {
        Arc::get_mut(&mut self.0).expect("configure before cloning").diagnostics = diagnostics;
        self
    }

    pub fn with_file_context(mut self, files: Arc<dyn ports::FileContextReader>) -> Self {
        Arc::get_mut(&mut self.0).expect("configure before cloning").files = Some(files);
        self
    }

    pub fn with_automatic_computer(mut self, enabled: bool) -> Self {
        Arc::get_mut(&mut self.0).expect("configure before cloning").automatic_computer = enabled;
        self
    }

    fn agent(&self, provider: Provider) -> ApplicationResult<Arc<dyn AgentRuntime>> {
        let slot = &self.0.providers[provider_index(provider)];
        if let Some(issue) = slot.agent.as_ref().and_then(|agent| agent.issue()) {
            return Err(ApplicationError(ErrorKind::Unavailable, issue));
        }
        slot.agent.clone().ok_or_else(|| {
            ApplicationError(
                ErrorKind::Unavailable,
                slot.issue.clone().unwrap_or_else(|| "Native runtime is unavailable".into()),
            )
        })
    }

    pub fn is_busy(&self) -> bool {
        match self.0.operations.lock() {
            | Ok(operations) => !operations.is_empty(),
            | Err(error) => {
                self.0
                    .diagnostics
                    .warn(&format!("Operation registry lock poisoned; treating application as busy: {error}"));
                true
            },
        }
    }

    pub fn cancel_all(&self) {
        if let Ok(mut inputs) = self.0.inputs.lock() {
            inputs.clear();
        }
        for (_, cancel) in (match self.0.operations.lock() {
            | Ok(guard) => guard,
            | Err(error) => {
                self.0
                    .diagnostics
                    .warn(&format!("Operation registry lock poisoned; recovering entries for cleanup: {error}"));
                error.into_inner()
            },
        })
        .drain()
        {
            let _ = cancel.send(());
        }
        (match self.0.interactions.lock() {
            | Ok(guard) => guard,
            | Err(error) => {
                self.0
                    .diagnostics
                    .warn(&format!("Interaction registry lock poisoned; recovering entries for cleanup: {error}"));
                error.into_inner()
            },
        })
        .clear();
    }
}
