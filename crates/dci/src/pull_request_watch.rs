use gpui_agent_contracts::{ChatRequest,
                           JobState,
                           PullRequestGuide,
                           PullRequestGuideOutcome,
                           PullRequestLink,
                           PullRequestSnapshot,
                           PullRequestTarget,
                           PullRequestWatchEffect,
                           PullRequestWatchEvent,
                           PullRequestWatchMachine,
                           transition_pull_request_watch};

pub trait WatchSubject {
    fn watch_machine(&self) -> PullRequestWatchMachine;
    fn target(&self) -> &PullRequestTarget;
    fn conversation_id(&self) -> i64;
    fn project_watch(&self, machine: &PullRequestWatchMachine) -> PullRequestLink;
}

impl WatchSubject for PullRequestLink {
    fn watch_machine(&self) -> PullRequestWatchMachine { PullRequestWatchMachine::from_link(self) }

    fn target(&self) -> &PullRequestTarget { &self.target }

    fn conversation_id(&self) -> i64 { self.conversation_id }

    fn project_watch(&self, machine: &PullRequestWatchMachine) -> PullRequestLink { machine.project_onto(self) }
}

#[derive(Clone, Debug)]
pub enum PullRequestPollInput {
    ActiveSnapshot(PullRequestSnapshot),
    RateLimited,
    ReadFailed,
    ConversationUnavailable,
}

#[derive(Clone, Debug)]
pub struct PullRequestPollOutcome {
    pub next_link: PullRequestLink,
    pub notification: Option<ChatRequest>,
}

pub struct PullRequestPollContext<'a, W: WatchSubject> {
    subject: &'a W,
}

impl<'a, W: WatchSubject> PullRequestPollContext<'a, W> {
    pub fn new(subject: &'a W) -> Self {
        Self {
            subject,
        }
    }

    pub fn evaluate(&self, input: PullRequestPollInput, now: i64) -> PullRequestPollOutcome {
        let event = match input {
            | PullRequestPollInput::ActiveSnapshot(current) => PullRequestWatchEvent::SnapshotObserved {
                current,
                now,
            },
            | PullRequestPollInput::RateLimited => PullRequestWatchEvent::ReadFailed {
                now,
                rate_limited: true,
            },
            | PullRequestPollInput::ReadFailed => PullRequestWatchEvent::ReadFailed {
                now,
                rate_limited: false,
            },
            | PullRequestPollInput::ConversationUnavailable => PullRequestWatchEvent::ConversationUnavailable,
        };
        let transition = transition_pull_request_watch(self.subject.target(), self.subject.conversation_id(), self.subject.watch_machine(), event);
        let notification = transition
            .effects
            .into_iter()
            .map(|PullRequestWatchEffect::NotifyConversation(request)| request)
            .next();
        PullRequestPollOutcome {
            next_link: self.subject.project_watch(&transition.state),
            notification,
        }
    }
}

pub struct PullRequestGuideContext;

impl PullRequestGuideContext {
    pub fn classify_start(target: PullRequestTarget, result: Result<(PullRequestGuide, bool), String>) -> PullRequestGuideOutcome {
        match result {
            | Ok((run, started)) => PullRequestGuideOutcome {
                target: Some(target),
                outcome: if started {
                    "started"
                } else if run.job.state == JobState::Completed {
                    "current"
                } else {
                    "running"
                },
                detail: None,
                run: Some(run),
            },
            | Err(detail) => PullRequestGuideOutcome {
                target: Some(target),
                outcome: "failed",
                detail: Some(detail),
                run: None,
            },
        }
    }

    pub fn is_stale(run: Option<&PullRequestGuide>, current_head_sha: Option<&str>, current_base_sha: Option<&str>) -> Option<bool> {
        run.and_then(|guide| {
            current_head_sha
                .zip(current_base_sha)
                .map(|(head, base)| head != guide.head_sha || base != guide.base_sha)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_agent_contracts::{JobScope,
                               Provider,
                               PullRequestState,
                               PullRequestWatch};

    fn watching_link() -> PullRequestLink {
        PullRequestLink {
            id: 1,
            scope: JobScope {
                provider: Provider::Codex,
                account_id: "default".into(),
                workspace_id: Some(1),
                workspace_path: "/repo".into(),
            },
            conversation_id: 5,
            target: PullRequestTarget::parse(None, Some("org/repo"), Some(12)).unwrap(),
            snapshot: Some(PullRequestSnapshot {
                title: "PR".into(),
                head_sha: "h1".into(),
                base_sha: "b1".into(),
                head_branch: "feat".into(),
                base_branch: "main".into(),
                state: PullRequestState::Open,
                draft: false,
                conflicting: Some(false),
                checks: vec![],
                remarks: vec![],
            }),
            watch: Some(PullRequestWatch {
                viewer: "me".into(),
                failures: 0,
                comment_wakes: 0,
            }),
            next_poll: Some(100),
            revision: 1,
            error: None,
            last_notification_job: None,
        }
    }

    #[test]
    fn poll_context_stops_watch_when_conversation_becomes_unavailable() {
        let link = watching_link();
        let outcome = PullRequestPollContext::new(&link).evaluate(PullRequestPollInput::ConversationUnavailable, 200);
        assert!(outcome.next_link.watch.is_none());
        assert!(outcome.next_link.next_poll.is_none());
        assert_eq!(
            outcome.next_link.error.as_deref(),
            Some("Conversation is unavailable or archived; watch stopped")
        );
        assert!(outcome.notification.is_none());
    }
}
