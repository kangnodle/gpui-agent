use crate::{Application,
            ApplicationError,
            ApplicationResult,
            ErrorKind,
            Workspaces,
            host_defaults,
            ports::{Clock,
                    DiagnosticSink,
                    JobRepository},
            project_verification::VerificationAttemptOutcome};
use gpui_agent_contracts::{AccountProfile,
                           ChatRequest,
                           InteractionResponse,
                           Job,
                           JobDetail,
                           JobEvent,
                           JobScope,
                           JobState,
                           Provider,
                           StreamEvent};
use gpui_agent_dci::job_execution::{TurnBatchContext,
                                    VerificationLoopContext};
use std::{collections::{HashMap,
                        HashSet},
          sync::{Arc,
                 Mutex,
                 atomic::{AtomicBool,
                          Ordering}}};

struct Active {
    application: Application,
    operation: Option<u64>,
}
pub(crate) struct Inner {
    pub(crate) checks: Mutex<HashMap<i64, tokio::sync::oneshot::Sender<()>>>,
    pub(crate) verifier: Option<Arc<dyn crate::ports::ProjectVerifier>>,
    pub(crate) repository: Arc<dyn JobRepository>,
    pub(crate) workspaces: Workspaces,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) diagnostics: Arc<dyn DiagnosticSink>,
    workers: Mutex<HashSet<String>>,
    active: Mutex<HashMap<i64, Active>>,
    stopping: AtomicBool,
    pub(crate) mutations: tokio::sync::Mutex<()>,
}
#[derive(Clone)]
pub struct Jobs(pub(crate) Arc<Inner>);
pub struct AccountScope {
    pub application: Application,
    pub(crate) _workspace: crate::workspaces::WorkspaceScope,
}

impl Jobs {
    pub fn new(repository: Arc<dyn JobRepository>, workspaces: Workspaces) -> Self {
        Self(Arc::new(Inner {
            checks: Mutex::new(HashMap::new()),
            verifier: None,
            repository,
            workspaces,
            clock: host_defaults::default_clock(),
            diagnostics: host_defaults::default_diagnostics(),
            workers: Mutex::new(HashSet::new()),
            active: Mutex::new(HashMap::new()),
            stopping: AtomicBool::new(false),
            mutations: tokio::sync::Mutex::new(()),
        }))
    }

    pub fn with_project_verifier(mut self, verifier: Arc<dyn crate::ports::ProjectVerifier>) -> Self {
        Arc::get_mut(&mut self.0).expect("configure before cloning").verifier = Some(verifier);
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        Arc::get_mut(&mut self.0).expect("configure before cloning").clock = clock;
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Arc<dyn DiagnosticSink>) -> Self {
        Arc::get_mut(&mut self.0).expect("configure before cloning").diagnostics = diagnostics;
        self
    }

    pub(crate) async fn io<T: Send + 'static>(&self, work: impl FnOnce(&dyn JobRepository) -> ApplicationResult<T> + Send + 'static) -> ApplicationResult<T> {
        let repository = self.0.repository.clone();
        tokio::task::spawn_blocking(move || work(repository.as_ref()))
            .await
            .map_err(|error| ApplicationError(ErrorKind::Failure, error.to_string()))?
    }

    pub async fn start(&self) -> ApplicationResult<()> {
        for job in self.list().await? {
            if job.state == JobState::Queued {
                self.wake(job.scope);
            }
        }
        let pr_weak = Arc::downgrade(&self.0);
        tokio::spawn(async move {
            loop {
                let Some(inner) = pr_weak.upgrade() else {
                    break;
                };
                let jobs = Jobs(inner);
                if jobs.is_stopping() {
                    break;
                }
                let now = jobs.0.clock.now_unix_secs() as i64;
                if jobs.poll_pull_requests_at(now).await.is_err() {
                    jobs.0
                        .diagnostics
                        .warn("PR watch poll failed; retrying without advancing uncommitted baselines");
                }
                drop(jobs);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
        Ok(())
    }

    pub async fn accounts(&self, provider: Provider) -> ApplicationResult<Vec<AccountProfile>> { self.io(move |repo| repo.accounts(provider)).await }

    pub async fn create_account(&self, provider: Provider, label: String) -> ApplicationResult<AccountProfile> {
        let label = gpui_agent_domain::normalize_title(&label)?.to_owned();
        self.io(move |repo| repo.create_account(provider, &label)).await
    }

    pub(crate) async fn resolve(
        &self,
        provider: Provider,
        account_id: String,
        workspace_id: Option<i64>,
    ) -> ApplicationResult<(JobScope, crate::workspaces::WorkspaceScope)> {
        gpui_agent_domain::validate_account_id(&account_id)?;
        let lease = self.0.workspaces.scope(workspace_id).await?;
        let catalog = self.0.workspaces.catalog_in_scope(&lease).await?;
        let id = workspace_id.or(catalog.default_workspace_id);
        let workspace = catalog
            .projects
            .iter()
            .flat_map(|project| &project.workspaces)
            .find(|workspace| Some(workspace.id) == id && !workspace.missing)
            .ok_or(ApplicationError(ErrorKind::NotFound, "Workspace not found".into()))?;
        let scope = JobScope {
            provider,
            account_id,
            workspace_id: Some(workspace.id),
            workspace_path: workspace.path.clone(),
        };
        Ok((scope, lease))
    }

    pub async fn account_scope(&self, provider: Provider, account: String, workspace: Option<i64>) -> ApplicationResult<AccountScope> {
        let (scope, lease) = self.resolve(provider, account, workspace).await?;
        let application = self.io(move |repo| repo.application(&scope)).await?;
        Ok(AccountScope {
            application,
            _workspace: lease,
        })
    }

    pub async fn enqueue(&self, provider: Provider, account: String, workspace: Option<i64>, request: ChatRequest) -> ApplicationResult<Job> {
        self.enqueue_command(provider, account, workspace, request, None).await
    }

    pub async fn enqueue_once(
        &self,
        provider: Provider,
        account: String,
        workspace: Option<i64>,
        request: ChatRequest,
        command_id: String,
    ) -> ApplicationResult<Job> {
        gpui_agent_domain::validate_command_id(&command_id)?;
        self.enqueue_command(provider, account, workspace, request, Some((command_id, Vec::new())))
            .await
    }

    pub(crate) async fn enqueue_uploaded_once(&self, scope: JobScope, request: ChatRequest, command_id: String, ids: Vec<i64>) -> ApplicationResult<Job> {
        gpui_agent_domain::validate_command_id(&command_id)?;
        self.enqueue_command(scope.provider, scope.account_id, scope.workspace_id, request, Some((command_id, ids)))
            .await
    }

    pub(crate) async fn session_workspace(&self, scope: &gpui_agent_domain::McpScope, workspace: Option<i64>) -> ApplicationResult<i64> {
        let catalog = self.0.workspaces.catalog().await?;
        let selected = catalog
            .projects
            .iter()
            .flat_map(|project| &project.workspaces)
            .find(|candidate| !candidate.missing && candidate.path == scope.workspace_path)
            .ok_or(ApplicationError(ErrorKind::NotFound, "MCP session workspace is unavailable".into()))?;
        if workspace.is_some_and(|id| id != selected.id) {
            return Err("Workspace is outside this MCP session's scope".into());
        }
        Ok(selected.id)
    }

    async fn enqueue_command(
        &self,
        provider: Provider,
        account: String,
        workspace: Option<i64>,
        request: ChatRequest,
        command_id: Option<(String, Vec<i64>)>,
    ) -> ApplicationResult<Job> {
        request.validate_provider(provider)?;
        request.validate_with_uploads(command_id.as_ref().map(|(_, ids)| ids.as_slice()).unwrap_or_default())?;
        if self.0.stopping.load(Ordering::SeqCst) {
            return Err(ApplicationError(ErrorKind::Unavailable, "Queue is stopping".into()));
        }
        let (scope, lease) = self.resolve(provider, account, workspace).await?;
        let application = self
            .io({
                let scope = scope.clone();
                move |repo| repo.application(&scope)
            })
            .await?;
        let mutation = self.0.mutations.lock().await;
        if let Some(id) = request.conversation_id {
            application.conversation(provider, id).await?;
        }
        let job = self
            .io({
                let scope = scope.clone();
                move |repo| match command_id {
                    | Some((command_id, ids)) => repo.enqueue_uploaded_once(&scope, &request, &ids, &command_id),
                    | None => repo.enqueue(&scope, &request),
                }
            })
            .await?;
        drop(mutation);
        drop(lease);
        self.wake(scope);
        Ok(job)
    }

    pub async fn list(&self) -> ApplicationResult<Vec<Job>> { self.io(|repo| repo.list()).await }

    pub fn is_stopping(&self) -> bool { self.0.stopping.load(Ordering::SeqCst) }

    pub async fn job(&self, id: i64) -> ApplicationResult<Job> { self.io(move |repo| repo.job(id)).await }

    pub async fn get(&self, id: i64) -> ApplicationResult<JobDetail> { self.io(move |repo| repo.get(id)).await }

    pub async fn events(&self, id: i64, after: i64) -> ApplicationResult<Vec<JobEvent>> { self.io(move |repo| repo.events(id, after)).await }

    pub(crate) async fn scoped_job(&self, caller: Option<&gpui_agent_domain::McpScope>, id: i64) -> ApplicationResult<Job> {
        if id <= 0 {
            return Err("Job ID must be positive".into());
        }
        let job = self.job(id).await?;
        if caller.is_some_and(|caller| !caller.allows(&job.scope)) {
            return Err(ApplicationError(ErrorKind::NotFound, "Job not found in this MCP session".into()));
        }
        Ok(job)
    }

    pub async fn cancel(&self, id: i64) -> ApplicationResult<()> {
        if let Ok(mut checks) = self.0.checks.lock() {
            checks.remove(&id);
        }
        if !self.io(move |repo| repo.cancel_queued(id)).await? {
            let active = match self.0.active.lock() {
                | Ok(guard) => guard,
                | Err(error) => {
                    self.0.diagnostics.warn(&format!(
                        "Active job registry lock poisoned; recovering registered entries for cleanup: {error}"
                    ));
                    error.into_inner()
                },
            };
            if let Some(active) = active.get(&id)
                && let Some(operation) = active.operation
            {
                active.application.cancel(operation);
            }
        }
        Ok(())
    }

    pub fn respond(&self, id: i64, request_id: u64, response: InteractionResponse) -> ApplicationResult<()> {
        self.respond_interaction(id, request_id, response, false)
    }

    pub async fn get_pending_questions(
        &self,
        caller: Option<&gpui_agent_contracts::McpScope>,
        id: i64,
    ) -> ApplicationResult<Option<gpui_agent_contracts::PendingQuestions>> {
        self.scoped_job(caller, id).await?;
        let active = self
            .0
            .active
            .lock()
            .map_err(|error| ApplicationError(ErrorKind::Failure, format!("Active job registry lock poisoned: {error}")))?;
        match active.get(&id) {
            | Some(active) => match active.operation {
                | Some(operation) => active.application.pending_questions(operation),
                | None => Ok(None),
            },
            | None => Ok(None),
        }
    }

    pub(crate) fn respond_interaction(&self, id: i64, request_id: u64, response: InteractionResponse, questions_only: bool) -> ApplicationResult<()> {
        let active = match self.0.active.lock() {
            | Ok(active) => active,
            | Err(error) => {
                return Err(ApplicationError(ErrorKind::Failure, format!("Active job registry lock poisoned: {error}")));
            },
        };
        let active = active
            .get(&id)
            .ok_or(ApplicationError(ErrorKind::NotFound, "Job is no longer running".into()))?;
        active.application.respond_interaction(
            active
                .operation
                .ok_or(ApplicationError(ErrorKind::NotFound, "Operation has not started".into()))?,
            request_id,
            response,
            questions_only,
        )
    }

    pub async fn mutation_guard(&self) -> tokio::sync::MutexGuard<'_, ()> { self.0.mutations.lock().await }

    pub async fn purge_conversation(&self, provider: Provider, account: String, workspace: Option<i64>, conversation: i64) -> ApplicationResult<()> {
        let workspace = workspace
            .or(self.0.workspaces.catalog().await?.default_workspace_id)
            .ok_or(ApplicationError(ErrorKind::NotFound, "Workspace not found".into()))?;
        self.io(move |repo| repo.purge_conversation(provider, &account, workspace, conversation)).await
    }

    pub fn maintenance_busy(&self) -> ApplicationResult<bool> { Ok(self.0.repository.list()?.iter().any(|job| !job.state.terminal())) }

    pub fn workspace_busy(&self, id: i64) -> ApplicationResult<bool> { self.0.repository.workspace_busy(id) }

    pub fn workspace_has_uploads(&self, id: i64) -> ApplicationResult<bool> { self.0.repository.workspace_has_uploads(id) }

    pub fn cancel_all(&self) {
        if let Ok(mut checks) = self.0.checks.lock() {
            checks.clear();
        }
        self.0.repository.cancel_mcp_oauth();
        self.0.stopping.store(true, Ordering::SeqCst);
        self.0.repository.cancel_all();
    }

    pub(crate) fn wake(&self, scope: JobScope) {
        if self.0.stopping.load(Ordering::SeqCst) {
            return;
        }
        let key = format!("{}:{}", gpui_agent_domain::provider_name(scope.provider), scope.account_id);
        match self.0.workers.lock() {
            | Ok(mut workers) =>
                if !workers.insert(key.clone()) {
                    return;
                },
            | Err(error) => {
                self.0.stopping.store(true, Ordering::SeqCst);
                self.0.diagnostics.warn(&format!("Worker registry lock poisoned; queue stopped: {error}"));
                return;
            },
        }
        let service = self.clone();
        tokio::spawn(async move {
            let mut storage_failed = false;
            while !service.0.stopping.load(Ordering::SeqCst) {
                let next = service
                    .io({
                        let scope = scope.clone();
                        move |repo| repo.claim(&scope)
                    })
                    .await;
                let detail = match next {
                    | Ok(Some(detail)) => detail,
                    | Ok(None) => break,
                    | Err(error) => {
                        service.0.diagnostics.warn(&format!("Cannot claim queued job: {error}"));
                        storage_failed = true;
                        break;
                    },
                };
                let id = detail.job.id;
                if let Err(error) = service.run(detail).await {
                    let message = error.to_string();
                    let recorded = service
                        .io(move |repo| {
                            repo.append(
                                id,
                                &StreamEvent::Error {
                                    message,
                                },
                            )
                        })
                        .await;
                    if let Err(error) = recorded {
                        service.0.diagnostics.warn(&format!("Cannot persist failed job {id}: {error}"));
                    }
                }
                (match service.0.active.lock() {
                    | Ok(guard) => guard,
                    | Err(error) => {
                        service.0.diagnostics.warn(&format!(
                            "Active job registry lock poisoned; recovering registered entries for cleanup: {error}"
                        ));
                        service.0.stopping.store(true, Ordering::SeqCst);
                        error.into_inner()
                    },
                })
                .remove(&id);
            }
            (match service.0.workers.lock() {
                | Ok(guard) => guard,
                | Err(error) => {
                    service
                        .0
                        .diagnostics
                        .warn(&format!("Worker registry lock poisoned; recovering registered entries for cleanup: {error}"));
                    service.0.stopping.store(true, Ordering::SeqCst);
                    error.into_inner()
                },
            })
            .remove(&key);
            if !storage_failed
                && !service.0.stopping.load(Ordering::SeqCst)
                && let Ok(jobs) = service.list().await
                && jobs
                    .iter()
                    .any(|job| job.state == JobState::Queued && job.scope.provider == scope.provider && job.scope.account_id == scope.account_id)
                && !jobs.iter().any(|job| {
                    matches!(job.state, JobState::Running | JobState::Waiting)
                        && job.scope.provider == scope.provider
                        && job.scope.account_id == scope.account_id
                })
            {
                service.wake(scope);
            }
        });
    }

    async fn run(&self, detail: JobDetail) -> ApplicationResult<()> {
        let scope = &detail.job.scope;
        let lease = self.0.workspaces.scope(scope.workspace_id).await?;
        let application = self
            .io({
                let scope = scope.clone();
                move |repo| repo.application(&scope)
            })
            .await?;
        let id = detail.job.id;
        {
            let mut active = match self.0.active.lock() {
                | Ok(active) => active,
                | Err(error) => {
                    self.0.stopping.store(true, Ordering::SeqCst);
                    return Err(ApplicationError(ErrorKind::Failure, format!("Active job registry lock poisoned: {error}")));
                },
            };
            active.insert(
                id,
                Active {
                    application: application.clone(),
                    operation: None,
                },
            );
        }
        if self.get(id).await?.job.cancel_requested {
            return self.io(move |repo| repo.append(id, &StreamEvent::Cancelled)).await;
        }
        let catalog = self.0.workspaces.catalog_in_scope(&lease).await?;
        let plan = catalog
            .projects
            .iter()
            .find(|project| project.workspaces.iter().any(|workspace| Some(workspace.id) == scope.workspace_id))
            .map(|project| project.verification.clone())
            .unwrap_or_default();
        plan.validate()?;
        let mut request = detail.request;
        drop(lease);
        let mut verification_loop = VerificationLoopContext::new(plan.max_attempts);
        for attempt in 1 ..= plan.max_attempts {
            if self.job(id).await?.cancel_requested || self.is_stopping() {
                return self.io(move |repo| repo.append(id, &StreamEvent::Cancelled)).await;
            }
            let (events, turn) = if attempt == 1 {
                application.chat_in_job(scope.provider, request.clone(), Some(id)).await?
            } else {
                application.repair_in_job(scope.provider, request.clone(), id).await?
            };
            let completion = self.run_turn(id, &application, events, !plan.commands.is_empty()).await?;
            if plan.commands.is_empty() || completion.is_none() {
                return Ok(());
            }
            if let Ok(mut active) = self.0.active.lock()
                && let Some(active) = active.get_mut(&id)
            {
                active.operation = None;
            }
            let (next_loop, outcome) = self.run_verification_attempt(id, scope, &application, turn, &plan, verification_loop).await?;
            verification_loop = next_loop;
            if self.job(id).await?.cancel_requested || self.is_stopping() {
                return self.io(move |repo| repo.append(id, &StreamEvent::Cancelled)).await;
            }
            match outcome {
                | VerificationAttemptOutcome::Cancelled => {
                    return self.io(move |repo| repo.append(id, &StreamEvent::Cancelled)).await;
                },
                | VerificationAttemptOutcome::Passed => {
                    let event = completion.expect("successful provider turn");
                    return self.io(move |repo| repo.append(id, &event)).await;
                },
                | VerificationAttemptOutcome::Repair {
                    instruction,
                } => {
                    request.text = instruction;
                    request.conversation_id = self.job(id).await?.conversation_id;
                    request.session_id = None;
                    request.attachments.clear();
                },
                | VerificationAttemptOutcome::Exhausted {
                    max_attempts,
                } => {
                    return Err(ApplicationError(
                        ErrorKind::Failure,
                        format!(
                            "Required project verification did not pass after {max_attempts} attempts. See the activity panel for command results and code versions."
                        ),
                    ));
                },
            }
        }
        Err(ApplicationError(
            ErrorKind::Failure,
            format!(
                "Required project verification did not pass after {} attempts. See the activity panel for command results and code versions.",
                plan.max_attempts
            ),
        ))
    }

    pub(crate) async fn run_turn(
        &self,
        id: i64,
        application: &Application,
        mut events: tokio::sync::mpsc::Receiver<StreamEvent>,
        verified: bool,
    ) -> ApplicationResult<Option<StreamEvent>> {
        let mut completion = None;
        let mut terminal = false;
        let batch_context = TurnBatchContext::new(verified);
        while let Some(event) = events.recv().await {
            let mut bytes = serde_json::to_vec(&event).map_err(anyhow::Error::from)?.len();
            let mut batch = vec![event];
            while batch.len() < 64 && bytes < 1024 * 1024 {
                let Ok(event) = events.try_recv() else {
                    break;
                };
                bytes += serde_json::to_vec(&event).map_err(anyhow::Error::from)?.len();
                batch.push(event);
            }
            let processed = batch_context.process(batch, terminal);
            for operation_id in processed.started_operations {
                {
                    let mut active = match self.0.active.lock() {
                        | Ok(active) => active,
                        | Err(error) => {
                            self.0.stopping.store(true, Ordering::SeqCst);
                            return Err(ApplicationError(ErrorKind::Failure, format!("Active job registry lock poisoned: {error}")));
                        },
                    };
                    match active.get_mut(&id) {
                        | Some(active) => active.operation = Some(operation_id),
                        | None => return Err(ApplicationError(ErrorKind::Failure, "Running job is missing from the active registry".into())),
                    }
                }
                if self.job(id).await?.cancel_requested {
                    application.cancel(operation_id);
                }
            }
            terminal = processed.saw_terminal;
            if processed.withheld_completion.is_some() {
                completion = processed.withheld_completion;
            }
            let events_to_persist = processed.events;
            self.io(move |repo| repo.append_batch(id, &events_to_persist)).await?;
        }
        if !terminal {
            return Err(ApplicationError(ErrorKind::Failure, "Job event stream ended without a terminal result".into()));
        }
        Ok(completion)
    }
}
