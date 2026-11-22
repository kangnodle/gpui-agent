use gpui_agent_contracts::{Job,
                           StreamEvent};
use gpui_agent_dci::job_execution::JobProjectionContext;

pub fn project_job_event(job: &Job, event: &StreamEvent) -> Option<Job> { JobProjectionContext::new(job).apply(event) }

pub fn apply_job_event(job: &mut Job, event: &StreamEvent) -> bool {
    match project_job_event(job, event) {
        | Some(next) => {
            *job = next;
            true
        },
        | None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_agent_contracts::{Event,
                               JobState};

    fn running() -> Job {
        Job {
            id: 1,
            scope: gpui_agent_contracts::JobScope {
                provider: gpui_agent_contracts::Provider::Codex,
                account_id: "default".into(),
                workspace_id: Some(2),
                workspace_path: "project".into(),
            },
            state: JobState::Running,
            conversation_id: None,
            operation_id: None,
            pending_request_id: None,
            last_sequence: 0,
            preview: "task".into(),
            cancel_requested: false,
            error: None,
            created_at: 100,
        }
    }

    #[test]
    fn approvals_resume_and_terminal_events_cannot_be_replayed() {
        for (terminal, state) in [
            (
                StreamEvent::Completed {
                    session_id: "native".into(),
                    text: "done".into(),
                },
                JobState::Completed,
            ),
            (
                StreamEvent::Error {
                    message: "runtime failed".into(),
                },
                JobState::Failed,
            ),
            (StreamEvent::Cancelled, JobState::Cancelled),
        ] {
            let mut job = running();
            assert!(apply_job_event(
                &mut job,
                &StreamEvent::Started {
                    operation_id: u64::MAX
                }
            ));
            assert!(apply_job_event(
                &mut job,
                &StreamEvent::Conversation {
                    conversation_id: 7
                }
            ));
            let request = StreamEvent::Interaction {
                operation_id: u64::MAX,
                request_id: 9,
                event: Event::Permission {
                    method: "tool".into(),
                    details: serde_json::json!({}),
                },
            };
            assert!(apply_job_event(&mut job, &request));
            assert_eq!(job.state, JobState::Waiting);
            assert_eq!(job.pending_request_id, Some(9));
            assert!(apply_job_event(
                &mut job,
                &StreamEvent::Agent(Event::InteractionResolved {
                    request_id: 9,
                    response: gpui_agent_contracts::InteractionResponse::Approval {
                        allow: false
                    },
                })
            ));
            assert_eq!(job.state, JobState::Running);
            assert_eq!(job.pending_request_id, None);
            assert!(apply_job_event(&mut job, &request));
            assert!(apply_job_event(&mut job, &terminal));
            assert_eq!(job.state, state);
            assert_eq!(job.pending_request_id, None);
            assert_eq!(job.operation_id, Some(u64::MAX));
            assert_eq!(job.conversation_id, Some(7));
            assert_eq!(job.error.as_deref(), (state == JobState::Failed).then_some("runtime failed"));
            let saved = serde_json::to_value(&job).unwrap();
            assert!(!apply_job_event(&mut job, &request));
            assert!(!apply_job_event(&mut job, &StreamEvent::Cancelled));
            assert_eq!(serde_json::to_value(&job).unwrap(), saved);
        }
    }
}
