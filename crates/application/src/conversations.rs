use crate::{AccountScope,
            ApplicationError,
            ApplicationResult,
            ErrorKind,
            Jobs};
use gpui_agent_contracts::*;

impl Jobs {
    pub(crate) async fn conversation_scope(&self, caller: Option<&McpScope>, target: ConversationTarget) -> ApplicationResult<(JobScope, AccountScope)> {
        let account = target
            .account
            .unwrap_or_else(|| caller.map_or("default", |scope| scope.account_id.as_str()).to_owned());
        let workspace = if let Some(caller) = caller {
            if caller.provider != target.provider || caller.account_id != account {
                return Err(ApplicationError(ErrorKind::NotFound, "Conversation scope is outside this MCP session".into()));
            }
            Some(self.session_workspace(caller, target.workspace).await?)
        } else {
            target.workspace
        };
        let (scope, lease) = self.resolve(target.provider, account, workspace).await?;
        if caller.is_some_and(|caller| !caller.allows(&scope)) {
            return Err(ApplicationError(ErrorKind::NotFound, "Conversation scope is outside this MCP session".into()));
        }
        let check = scope.clone();
        let application = self.io(move |repo| repo.application(&check)).await?;
        Ok((
            scope,
            AccountScope {
                application,
                _workspace: lease,
            },
        ))
    }

    pub async fn conversation_list(&self, caller: Option<&McpScope>, request: ConversationListRequest) -> ApplicationResult<ConversationListPage> {
        if !(1 ..= 50).contains(&request.limit) {
            return Err("Conversation list limit must be 1–50".into());
        }
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        // shortcut: full reads, add DB paging if history gets slow.
        let items = context.application.conversations(scope.provider, request.query.unwrap_or_default()).await?;
        let mut page = ConversationListPage::new(items, request.archived, request.offset, request.limit)?;
        self.io(move |repo| {
            for item in &mut page.items {
                let job = match repo.conversation_job(&scope, item.id, None, true)? {
                    | Some(job) => Some(job),
                    | None => repo.conversation_job(&scope, item.id, None, false)?,
                };
                item.execution_state = job.map(|job| job.state);
            }
            Ok(page)
        })
        .await
    }

    pub async fn conversation_read(&self, caller: Option<&McpScope>, request: ConversationReadRequest) -> ApplicationResult<ConversationPage> {
        if request.id <= 0 || !(1 ..= 20).contains(&request.limit) {
            return Err("Conversation ID must be positive and message limit 1–20".into());
        }
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        Ok(context
            .application
            .conversation(scope.provider, request.id)
            .await?
            .page(request.cursor, request.limit)?)
    }

    pub async fn conversation_rename(&self, caller: Option<&McpScope>, request: ConversationRenameRequest) -> ApplicationResult<()> {
        if request.id <= 0 {
            return Err("Conversation ID must be positive".into());
        }
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        context.application.rename_conversation(scope.provider, request.id, request.title).await
    }

    pub async fn conversation_organize(&self, caller: Option<&McpScope>, request: ConversationOrganizeRequest) -> ApplicationResult<()> {
        if request.id <= 0 || (request.pinned.is_none() && request.archived.is_none()) {
            return Err("A positive conversation ID and pinned or archived state are required".into());
        }
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        let _guard = self.0.mutations.lock().await;
        context
            .application
            .set_conversation_state(
                scope.provider,
                request.id,
                ConversationState {
                    pinned: request.pinned,
                    archived: request.archived,
                },
            )
            .await?;
        if request.archived == Some(true) {
            self.io(move |repo| repo.stop_pull_request_watches(&scope, request.id)).await?;
        }
        Ok(())
    }

    pub async fn conversation_send(&self, caller: Option<&McpScope>, request: ConversationSendRequest) -> ApplicationResult<Job> {
        if request.id.is_some_and(|id| id <= 0) {
            return Err("Conversation ID must be positive".into());
        }
        if request
            .model
            .as_ref()
            .is_some_and(|model| model.trim().is_empty() || model.len() > 512 || model.chars().any(char::is_control))
        {
            return Err("Invalid model ID (1–512 bytes)".into());
        }
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        drop(context);
        let chat = ChatRequest {
            extensions: None,
            skills: request.skills,
            computer_use: false,
            text: request.text,
            model: request.model,
            conversation_id: request.id,
            session_id: None,
            attachments: request.attachments,
            claude_permission_mode: None,
            codex_execution_policy: None,
        };
        self.enqueue_uploaded_once(scope, chat, request.command_id, request.attachment_ids).await
    }

    pub async fn conversation_wait(&self, caller: Option<&McpScope>, request: ConversationWaitRequest) -> ApplicationResult<ConversationWaitResult> {
        if request.id <= 0 || request.job_id.is_some_and(|id| id <= 0) || request.timeout_ms > 10_000 {
            return Err("Positive IDs and a timeout of 0–10000 ms are required".into());
        }
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        context.application.conversation(scope.provider, request.id).await?;
        let selected = self.io(move |repo| repo.conversation_job(&scope, request.id, request.job_id, false)).await?;
        let Some(job) = selected else {
            if request.job_id.is_some() {
                return Err(ApplicationError(ErrorKind::NotFound, "Job not found in this conversation".into()));
            }
            return Ok(ConversationWaitResult {
                job: None,
                timed_out: false,
            });
        };
        if request.timeout_ms == 0 || job.state.terminal() {
            let timed_out = !job.state.terminal();
            return Ok(ConversationWaitResult {
                job: Some(job),
                timed_out,
            });
        }
        let id = job.id;
        let finished = tokio::time::timeout(std::time::Duration::from_millis(request.timeout_ms), async {
            loop {
                let job = self.job(id).await?;
                if job.state.terminal() {
                    return Ok::<_, ApplicationError>(job);
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;
        let job = match finished {
            | Ok(result) => result?,
            | Err(_) => self.job(id).await?,
        };
        let timed_out = !job.state.terminal();
        Ok(ConversationWaitResult {
            job: Some(job),
            timed_out,
        })
    }

    pub async fn conversation_interrupt(&self, caller: Option<&McpScope>, request: ConversationInterruptRequest) -> ApplicationResult<Option<Job>> {
        if request.id <= 0 || request.job_id.is_some_and(|id| id <= 0) {
            return Err("Conversation and job IDs must be positive".into());
        }
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        context.application.conversation(scope.provider, request.id).await?;
        let _guard = self.0.mutations.lock().await;
        let selected = self
            .io({
                let scope = scope.clone();
                move |repo| repo.conversation_job(&scope, request.id, request.job_id, request.job_id.is_none())
            })
            .await?;
        if selected.is_none() && request.job_id.is_some() {
            return Err(ApplicationError(ErrorKind::NotFound, "Job not found in this conversation".into()));
        }
        self.io(move |repo| repo.stop_pull_request_watches(&scope, request.id)).await?;
        let Some(job) = selected else {
            if request.job_id.is_some() {
                return Err(ApplicationError(ErrorKind::NotFound, "Job not found in this conversation".into()));
            }
            return Ok(None);
        };
        if !job.state.terminal()
            && let Err(error) = self.cancel(job.id).await
            && (error.0 != ErrorKind::Busy || !self.job(job.id).await?.state.terminal())
        {
            return Err(error);
        }
        Ok(Some(self.job(job.id).await?))
    }
}
