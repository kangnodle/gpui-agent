use crate::{Application,
            ApplicationError,
            ApplicationResult,
            ErrorKind,
            operations::Operation,
            ports::{EditOutcome,
                    ForkOutcome}};
use gpui_agent_contracts::{ChatRequest,
                           LoginRequest,
                           ProviderInfo,
                           StreamEvent};
use gpui_agent_domain::{AuthStatus,
                        ConversationSummary,
                        Model,
                        PROVIDERS,
                        Prompt,
                        Provider,
                        SavedConversation,
                        display_name,
                        normalize_title,
                        provider_index,
                        validate_login,
                        validate_search};
use tokio::sync::mpsc;

fn not_found() -> ApplicationError { ApplicationError(ErrorKind::NotFound, "Conversation not found for this provider and workspace".into()) }

fn edit_result(outcome: EditOutcome) -> ApplicationResult<()> {
    match outcome {
        | EditOutcome::Done => Ok(()),
        | EditOutcome::NotFound => Err(not_found()),
        | EditOutcome::Busy => Err(ApplicationError(
            ErrorKind::Busy,
            "Conversation has an active response; stop it before editing or deleting".into(),
        )),
    }
}

impl Application {
    pub fn providers(&self) -> Vec<ProviderInfo> {
        PROVIDERS
            .into_iter()
            .map(|provider| {
                let slot = &self.0.providers[provider_index(provider)];
                let issue = slot.agent.as_ref().and_then(|agent| agent.issue()).or_else(|| slot.issue.clone());
                ProviderInfo {
                    capabilities: gpui_agent_contracts::ProviderCapabilities {
                        mcp_stdio: provider != Provider::Antigravity,
                        mcp_http: provider != Provider::Antigravity,
                        mcp_capability_check: provider == Provider::Antigravity,
                        usage: matches!(provider, Provider::Claude | Provider::Codex),
                        background_jobs: false,
                        text_attachments: true,
                        images: true,
                        image_capability_check: provider == Provider::Antigravity,
                        tool_timeline: true,
                        approvals: true,
                        user_input: matches!(provider, Provider::Claude | Provider::Codex),
                    },
                    provider,
                    name: display_name(provider).into(),
                    available: slot.agent.is_some() && issue.is_none(),
                    issue,
                }
            })
            .collect()
    }

    pub async fn status(&self, provider: Provider) -> ApplicationResult<AuthStatus> { Ok(self.agent(provider)?.status().await?) }

    pub async fn models(&self, provider: Provider) -> ApplicationResult<Vec<Model>> { Ok(self.agent(provider)?.models().await?) }

    pub async fn logout(&self, provider: Provider) -> ApplicationResult<AuthStatus> {
        self.agent(provider)?.logout().await?;
        Ok(AuthStatus {
            authenticated: false,
            account: None,
            method: None,
        })
    }

    pub async fn login(&self, provider: Provider, request: LoginRequest) -> ApplicationResult<mpsc::Receiver<StreamEvent>> {
        validate_login(provider, request.device_code)?;
        self.stream(self.agent(provider)?, Operation::Login(request.device_code), None)
    }

    pub async fn chat(&self, provider: Provider, request: ChatRequest) -> ApplicationResult<mpsc::Receiver<StreamEvent>> {
        self.chat_in_job(provider, request, None).await.map(|(events, _)| events)
    }

    pub(crate) async fn chat_in_job(
        &self,
        provider: Provider,
        request: ChatRequest,
        job: Option<i64>,
    ) -> ApplicationResult<(mpsc::Receiver<StreamEvent>, crate::ports::Turn)> {
        self.chat_turn(provider, request, job, false).await
    }

    pub(crate) async fn repair_in_job(
        &self,
        provider: Provider,
        request: ChatRequest,
        job: i64,
    ) -> ApplicationResult<(mpsc::Receiver<StreamEvent>, crate::ports::Turn)> {
        self.chat_turn(provider, request, Some(job), true).await
    }

    async fn chat_turn(
        &self,
        provider: Provider,
        mut request: ChatRequest,
        job: Option<i64>,
        repair: bool,
    ) -> ApplicationResult<(mpsc::Receiver<StreamEvent>, crate::ports::Turn)> {
        request.validate_provider(provider)?;
        request.validate()?;
        if request.computer_use && job.is_none() {
            return Err("Computer tasks require a durable job".into());
        }
        let agent = self.agent(provider)?;
        if repair {
            let saved = self
                .conversation(provider, request.conversation_id.ok_or("Repair requires its original conversation")?)
                .await?;
            request.skills = saved
                .messages
                .iter()
                .rev()
                .find(|message| message.user)
                .map(|message| message.skills.clone())
                .unwrap_or_default();
        }
        let mcp_servers = if let Some(selection) = &request.extensions {
            let catalog = if selection.plugins.is_empty() {
                Default::default()
            } else {
                self.plugins()?.catalog().await?
            };
            Some(selection.mcp_names(&catalog)?)
        } else {
            None
        };
        let mut guidance = if let Some(guidance) = &self.0.guidance {
            Some(
                guidance
                    .prepare_for_conversation(&request.text, &request.skills, agent.supports_guidance_loading(), request.extensions.as_ref())
                    .await?,
            )
        } else {
            if !request.skills.is_empty() {
                return Err("Host guidance is unavailable".into());
            }
            None
        };
        if request.extensions.is_some() && guidance.is_none() {
            guidance = Some(gpui_agent_domain::PreparedGuidance {
                extensions: request.extensions.clone(),
                snapshot: Default::default(),
                selected: vec![],
                implicit_available: false,
            });
        }
        let history = self.0.history.clone();
        let mut prompt = Prompt {
            mcp_servers,
            skills: guidance.as_ref().map(|guidance| guidance.references()).unwrap_or(request.skills),
            guidance,
            claude_permission_mode: request.claude_permission_mode,
            codex_execution_policy: request.codex_execution_policy,
            attachments: request.attachments,
            text: request.text,
            model: request.model,
            session_id: request.session_id,
        };
        let (prompt, turn) = tokio::task::spawn_blocking(move || {
            let turn = if repair {
                history.begin_repair(
                    provider,
                    request
                        .conversation_id
                        .ok_or_else(|| anyhow::anyhow!("Repair requires its original conversation"))?,
                    &mut prompt,
                )?
            } else {
                history.begin(provider, request.conversation_id, &mut prompt)?
            };
            Ok::<_, anyhow::Error>((prompt, turn))
        })
        .await
        .map_err(anyhow::Error::from)??;
        Ok((
            self.stream(agent, Operation::Chat(Box::new(prompt), job, request.computer_use), Some(turn))?,
            turn,
        ))
    }

    pub(crate) async fn record_verification(&self, turn: crate::ports::Turn, report: gpui_agent_domain::VerificationReport) -> ApplicationResult<()> {
        let history = self.0.history.clone();
        tokio::task::spawn_blocking(move || {
            history.event(
                turn,
                &gpui_agent_domain::Event::Verification {
                    report,
                },
            )
        })
        .await
        .map_err(anyhow::Error::from)??;
        Ok(())
    }

    pub async fn file_context(&self, request: gpui_agent_domain::FileContextRequest) -> ApplicationResult<gpui_agent_domain::Attachment> {
        self.read_file(request, false).await
    }

    pub async fn file_attachment(&self, request: gpui_agent_domain::FileContextRequest) -> ApplicationResult<gpui_agent_domain::Attachment> {
        self.read_file(request, true).await
    }

    async fn read_file(&self, request: gpui_agent_domain::FileContextRequest, attachment: bool) -> ApplicationResult<gpui_agent_domain::Attachment> {
        request.validate()?;
        let reader = self
            .0
            .files
            .clone()
            .ok_or(ApplicationError(ErrorKind::Unavailable, "File context is unavailable".into()))?;
        tokio::task::spawn_blocking(move || if attachment { reader.attachment(&request) } else { reader.read(&request) })
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn workspace_files(&self, request: gpui_agent_domain::WorkspaceFilesRequest) -> ApplicationResult<gpui_agent_domain::WorkspaceFiles> {
        request.validate()?;
        let reader = self
            .0
            .files
            .clone()
            .ok_or(ApplicationError(ErrorKind::Unavailable, "File context is unavailable".into()))?;
        tokio::task::spawn_blocking(move || reader.list(&request))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn conversations(&self, provider: Provider, query: String) -> ApplicationResult<Vec<ConversationSummary>> {
        validate_search(&query)?;
        let history = self.0.history.clone();
        Ok(tokio::task::spawn_blocking(move || history.list(provider, &query))
            .await
            .map_err(anyhow::Error::from)??)
    }

    pub async fn conversation(&self, provider: Provider, id: i64) -> ApplicationResult<SavedConversation> {
        let history = self.0.history.clone();
        tokio::task::spawn_blocking(move || history.load(provider, id))
            .await
            .map_err(anyhow::Error::from)??
            .ok_or_else(not_found)
    }

    pub async fn conversation_versions(&self, provider: Provider, id: i64) -> ApplicationResult<Vec<gpui_agent_domain::ConversationVersion>> {
        self.conversation(provider, id).await?;
        let history = self.0.history.clone();
        Ok(tokio::task::spawn_blocking(move || history.versions(provider, id))
            .await
            .map_err(anyhow::Error::from)??)
    }

    pub async fn revise_conversation(
        &self,
        provider: Provider,
        id: i64,
        revision: gpui_agent_contracts::ReviseRequest,
    ) -> ApplicationResult<gpui_agent_contracts::PreparedRevision> {
        if let Some(policy) = revision.codex_execution_policy {
            policy.validate_provider(provider)?;
        }
        let message_id = revision.message_id;
        let gpui_agent_domain::RevisionPlan {
            mut request,
            context,
        } = self.conversation(provider, id).await?.prepare_revision(revision)?;
        let history = self.0.history.clone();
        let title = gpui_agent_domain::conversation_title(&request.text);
        let outcome = tokio::task::spawn_blocking(move || history.fork(provider, id, message_id, &title, &context))
            .await
            .map_err(anyhow::Error::from)??;
        let next = match outcome {
            | ForkOutcome::Created(id) => id,
            | ForkOutcome::NotFound => return Err(not_found()),
            | ForkOutcome::Busy => return Err(ApplicationError(ErrorKind::Busy, "The source question changed or is still running".into())),
        };
        request.conversation_id = Some(next);
        Ok(gpui_agent_contracts::PreparedRevision {
            conversation: self.conversation(provider, next).await?,
            request,
            versions: self.conversation_versions(provider, next).await?,
        })
    }

    pub async fn rename_conversation(&self, provider: Provider, id: i64, title: String) -> ApplicationResult<()> {
        let title = normalize_title(&title)?.to_owned();
        let history = self.0.history.clone();
        edit_result(
            tokio::task::spawn_blocking(move || history.rename(provider, id, &title))
                .await
                .map_err(anyhow::Error::from)??,
        )
    }

    pub async fn set_conversation_state(&self, provider: Provider, id: i64, state: gpui_agent_domain::ConversationState) -> ApplicationResult<()> {
        let history = self.0.history.clone();
        edit_result(
            tokio::task::spawn_blocking(move || history.set_state(provider, id, &state))
                .await
                .map_err(anyhow::Error::from)??,
        )
    }

    pub async fn delete_conversation(&self, provider: Provider, id: i64) -> ApplicationResult<()> {
        let history = self.0.history.clone();
        edit_result(
            tokio::task::spawn_blocking(move || history.delete(provider, id))
                .await
                .map_err(anyhow::Error::from)??,
        )
    }

    pub fn respond(&self, operation_id: u64, request_id: u64, response: gpui_agent_domain::InteractionResponse) -> ApplicationResult<()> {
        self.respond_interaction(operation_id, request_id, response, false)
    }

    pub(crate) fn respond_interaction(
        &self,
        operation_id: u64,
        request_id: u64,
        response: gpui_agent_domain::InteractionResponse,
        questions_only: bool,
    ) -> ApplicationResult<()> {
        let operations = match self.0.operations.lock() {
            | Ok(operations) => operations,
            | Err(error) => return Err(ApplicationError(ErrorKind::Failure, format!("Operation registry lock poisoned: {error}"))),
        };
        if !operations.contains_key(&operation_id) {
            return Err(ApplicationError(ErrorKind::NotFound, "Operation is no longer active".into()));
        }
        let mut pending = match self.0.interactions.lock() {
            | Ok(pending) => pending,
            | Err(error) => return Err(ApplicationError(ErrorKind::Failure, format!("Interaction registry lock poisoned: {error}"))),
        };
        let request = pending
            .get(&request_id)
            .filter(|request| request.operation_id == operation_id)
            .ok_or_else(|| ApplicationError(ErrorKind::NotFound, "Interaction is no longer pending".into()))?;
        if questions_only && !matches!(request.event, gpui_agent_domain::Event::UserInput { .. }) {
            return Err("This request is not a user question".into());
        }
        gpui_agent_domain::validate_response(&request.event, &response)?;
        let request = match pending.remove(&request_id) {
            | Some(request) => request,
            | None => return Err(ApplicationError(ErrorKind::NotFound, "Interaction is no longer pending".into())),
        };
        request
            .sender
            .send(response)
            .map_err(|_| ApplicationError(ErrorKind::NotFound, "Interaction has ended".into()))
    }

    pub fn cancel(&self, id: u64) -> bool {
        if let Some(cancel) = (match self.0.operations.lock() {
            | Ok(guard) => guard,
            | Err(error) => {
                self.0
                    .diagnostics
                    .warn(&format!("Operation registry lock poisoned; recovering entries for cleanup: {error}"));
                error.into_inner()
            },
        })
        .remove(&id)
        {
            (match self.0.interactions.lock() {
                | Ok(guard) => guard,
                | Err(error) => {
                    self.0
                        .diagnostics
                        .warn(&format!("Interaction registry lock poisoned; recovering entries for cleanup: {error}"));
                    error.into_inner()
                },
            })
            .retain(|_, request| request.operation_id != id);
            let _ = cancel.send(());
            true
        } else {
            false
        }
    }
}
