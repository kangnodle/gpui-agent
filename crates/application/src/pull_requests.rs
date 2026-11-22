use crate::{AccountScope,
            ApplicationResult,
            ErrorKind,
            Jobs};
use gpui_agent_contracts::*;
use gpui_agent_dci::pull_request_watch::{PullRequestGuideContext,
                                         PullRequestPollContext,
                                         PullRequestPollInput};

impl Jobs {
    pub async fn write_pull_request_guides(
        &self,
        caller: Option<&McpScope>,
        request: WritePullRequestGuides,
    ) -> ApplicationResult<Vec<PullRequestGuideOutcome>> {
        if request.pull_requests.is_empty() || request.pull_requests.len() > 20 {
            return Err("Supply 1–20 pull requests".into());
        }
        let (scope, context) = self.conversation_scope(caller, request.scope).await?;
        let mut outcomes = vec![];
        for input in request.pull_requests {
            let target = match input.target() {
                | Ok(target) => target,
                | Err(error) => {
                    outcomes.push(PullRequestGuideOutcome {
                        target: None,
                        outcome: "failed",
                        detail: Some(error.into()),
                        run: None,
                    });
                    continue;
                },
            };
            let start = async {
                if self.is_stopping() {
                    return Err(crate::ApplicationError(ErrorKind::Unavailable, "Queue is stopping".into()));
                }
                let data = self
                    .0
                    .workspaces
                    .pull_request(&scope, &context._workspace, target.clone(), PullRequestReadKind::Guide)
                    .await?;
                let chat = data.guide_request(&target)?;
                let _guard = self.0.mutations.lock().await;
                let scope = scope.clone();
                let target = target.clone();
                self.io(move |repo| repo.start_pull_request_guide(&scope, &target, &data.snapshot.head_sha, &data.snapshot.base_sha, &chat))
                    .await
            }
            .await;
            if let Ok((run, _)) = &start
                && !run.job.state.terminal()
            {
                self.wake(run.job.scope.clone());
            }
            outcomes.push(PullRequestGuideContext::classify_start(target, start.map_err(|error| error.1)));
        }
        Ok(outcomes)
    }

    pub async fn read_pull_request_guide(&self, caller: Option<&McpScope>, request: ReadPullRequestGuide) -> ApplicationResult<PullRequestGuideRead> {
        let target = PullRequestTarget::parse(request.url.as_deref(), request.repository.as_deref(), request.number)?;
        let (scope, context) = self.conversation_scope(caller, request.scope).await?;
        let run = self
            .io({
                let scope = scope.clone();
                let target = target.clone();
                move |repo| repo.pull_request_guide(&scope, &target)
            })
            .await?;
        let (current_head_sha, current_base_sha, read_error) = match self
            .0
            .workspaces
            .pull_request(&scope, &context._workspace, target.clone(), PullRequestReadKind::Metadata)
            .await
        {
            | Ok(data) => (Some(data.snapshot.head_sha), Some(data.snapshot.base_sha), None),
            | Err(error) if error.0 == ErrorKind::Invalid => return Err(error),
            | Err(error) => (None, None, Some(error.1)),
        };
        let stale = PullRequestGuideContext::is_stale(run.as_ref(), current_head_sha.as_deref(), current_base_sha.as_deref());
        Ok(PullRequestGuideRead {
            target,
            run,
            current_head_sha,
            current_base_sha,
            stale,
            read_error,
        })
    }

    pub async fn poll_pull_requests_at(&self, now: i64) -> ApplicationResult<()> {
        if self.is_stopping() {
            return Ok(());
        }
        for link in self.io(move |repo| repo.due_pull_requests(now)).await? {
            if self.is_stopping() {
                break;
            }
            let target = ConversationTarget {
                provider: link.scope.provider,
                account: Some(link.scope.account_id.clone()),
                workspace: link.scope.workspace_id,
            };
            let context = self.conversation_scope(None, target).await;
            let outcome = if let Ok((scope, context)) = &context {
                if scope != &link.scope {
                    None
                } else {
                    Some(
                        self.0
                            .workspaces
                            .pull_request(&link.scope, &context._workspace, link.target.clone(), PullRequestReadKind::Metadata)
                            .await,
                    )
                }
            } else {
                None
            };
            let _guard = self.0.mutations.lock().await;
            if self.is_stopping() {
                break;
            }
            let active = match &context {
                | Ok((scope, context)) if scope == &link.scope => context
                    .application
                    .conversations(scope.provider, String::new())
                    .await?
                    .iter()
                    .any(|conversation| conversation.id == link.conversation_id && !conversation.archived),
                | _ => false,
            };
            let poll_input = if !active {
                PullRequestPollInput::ConversationUnavailable
            } else {
                match outcome {
                    | Some(Ok(data)) => PullRequestPollInput::ActiveSnapshot(data.snapshot),
                    | Some(Err(error)) if error.0 == ErrorKind::Busy => PullRequestPollInput::RateLimited,
                    | Some(Err(_)) | None => PullRequestPollInput::ReadFailed,
                }
            };
            let evaluated = PullRequestPollContext::new(&link).evaluate(poll_input, now);
            let next = evaluated.next_link;
            let notification = evaluated.notification;
            if let Some(job) = self.io(move |repo| repo.apply_pull_request_poll(&link, &next, notification.as_ref())).await? {
                self.wake(job.scope);
            }
        }
        Ok(())
    }

    async fn pull_request_scope(&self, caller: Option<&McpScope>, request: PullRequestConversation) -> ApplicationResult<(JobScope, AccountScope)> {
        if request.id <= 0 {
            return Err("A positive conversation ID is required".into());
        }
        let (scope, context) = self.conversation_scope(caller, request.scope).await?;
        context.application.conversation(scope.provider, request.id).await?;
        Ok((scope, context))
    }

    pub async fn pull_request_links(&self, caller: Option<&McpScope>, request: PullRequestConversation) -> ApplicationResult<Vec<PullRequestLink>> {
        let id = request.id;
        let (scope, _context) = self.pull_request_scope(caller, request).await?;
        self.io(move |repo| repo.pull_request_links(&scope, id)).await
    }

    pub async fn set_pull_request_link(&self, caller: Option<&McpScope>, request: PullRequestCommand, linked: bool) -> ApplicationResult<bool> {
        let target = request.target()?;
        let id = request.id;
        let (scope, _context) = self
            .pull_request_scope(
                caller,
                PullRequestConversation {
                    scope: request.scope,
                    id,
                },
            )
            .await?;
        let _guard = self.0.mutations.lock().await;
        self.io(move |repo| repo.set_pull_request_link(&scope, id, &target, linked)).await
    }

    pub async fn set_pull_request_watch(&self, caller: Option<&McpScope>, request: PullRequestCommand, watching: bool) -> ApplicationResult<bool> {
        let target = request.target()?;
        let id = request.id;
        let (scope, context) = self
            .pull_request_scope(
                caller,
                PullRequestConversation {
                    scope: request.scope,
                    id,
                },
            )
            .await?;
        let baseline = if watching {
            Some(
                self.0
                    .workspaces
                    .pull_request(&scope, &context._workspace, target.clone(), PullRequestReadKind::Watch)
                    .await?,
            )
        } else {
            None
        };
        let _guard = self.0.mutations.lock().await;
        if watching
            && !context
                .application
                .conversations(scope.provider, String::new())
                .await?
                .iter()
                .any(|conversation| conversation.id == id && !conversation.archived)
        {
            return Err("Cannot watch PRs for an archived or unavailable conversation".into());
        }
        let now = self.0.clock.now_unix_secs() as i64;
        self.io(move |repo| repo.set_pull_request_watch(&scope, id, &target, baseline.as_ref(), now))
            .await
    }
}
