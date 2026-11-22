use gpui_agent_contracts::StreamEvent;
use gpui_agent_domain::{ChatRequest,
                        CodeVersion,
                        Event,
                        Job,
                        JobMachine,
                        JobTransitionInput,
                        JobTransitionOutcome,
                        Transition,
                        VerificationCheck,
                        VerificationEffect,
                        VerificationInput,
                        VerificationMachine,
                        VerificationReport,
                        transition_job,
                        transition_verification};

pub trait JobEventProjector {
    fn machine(&self) -> JobMachine;
    fn with_machine(&self, machine: &JobMachine) -> Job;
}

impl JobEventProjector for Job {
    fn machine(&self) -> JobMachine { JobMachine::from_job(self) }

    fn with_machine(&self, machine: &JobMachine) -> Job { machine.project_onto(self) }
}

pub trait TurnEventClassifier {
    fn to_job_transition_input(&self) -> JobTransitionInput;
}

impl TurnEventClassifier for StreamEvent {
    fn to_job_transition_input(&self) -> JobTransitionInput {
        match self {
            | StreamEvent::Started {
                operation_id,
            } => JobTransitionInput::OperationStarted {
                operation_id: *operation_id,
            },
            | StreamEvent::Conversation {
                conversation_id,
            } => JobTransitionInput::ConversationBound {
                conversation_id: *conversation_id,
            },
            | StreamEvent::Interaction {
                request_id, ..
            } => JobTransitionInput::InteractionRequested {
                request_id: *request_id,
            },
            | StreamEvent::Agent(Event::InteractionResolved {
                ..
            }) => JobTransitionInput::InteractionResolved,
            | StreamEvent::Completed {
                ..
            } => JobTransitionInput::Completed,
            | StreamEvent::Error {
                message,
            } => JobTransitionInput::Failed {
                message: message.clone(),
            },
            | StreamEvent::Cancelled => JobTransitionInput::Cancelled,
            | _ => JobTransitionInput::Passthrough,
        }
    }
}

pub trait RepairRequestBuilder {
    fn next_repair_turn(&self, report: &VerificationReport, conversation_id: Option<i64>) -> ChatRequest;
}

impl RepairRequestBuilder for ChatRequest {
    fn next_repair_turn(&self, report: &VerificationReport, conversation_id: Option<i64>) -> ChatRequest {
        ChatRequest {
            text: report.repair_instruction(),
            conversation_id,
            session_id: None,
            attachments: Vec::new(),
            ..self.clone()
        }
    }
}

pub struct JobProjectionContext<'a, J: JobEventProjector> {
    projector: &'a J,
}

impl<'a, J: JobEventProjector> JobProjectionContext<'a, J> {
    pub fn new(projector: &'a J) -> Self {
        Self {
            projector,
        }
    }

    pub fn apply<E: TurnEventClassifier>(&self, event: &E) -> Option<Job> {
        match transition_job(self.projector.machine(), event.to_job_transition_input()) {
            | JobTransitionOutcome::Accepted(next) => Some(self.projector.with_machine(&next)),
            | JobTransitionOutcome::IgnoredTerminal(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TurnBatchOutcome {
    pub events: Vec<StreamEvent>,
    pub started_operations: Vec<u64>,
    pub withheld_completion: Option<StreamEvent>,
    pub saw_terminal: bool,
}

pub struct TurnBatchContext {
    verified: bool,
}

impl TurnBatchContext {
    pub fn new(verified: bool) -> Self {
        Self {
            verified,
        }
    }

    pub fn process(&self, batch: Vec<StreamEvent>, prior_terminal: bool) -> TurnBatchOutcome {
        batch.into_iter().fold(
            TurnBatchOutcome {
                events: Vec::new(),
                started_operations: Vec::new(),
                withheld_completion: None,
                saw_terminal: prior_terminal,
            },
            |mut acc, event| {
                if let StreamEvent::Started {
                    operation_id,
                } = &event
                {
                    acc.started_operations.push(*operation_id);
                }
                acc.saw_terminal = event.is_terminal();
                if self.verified && matches!(event, StreamEvent::Completed { .. }) {
                    acc.withheld_completion = Some(event);
                    acc.events.push(StreamEvent::Agent(Event::TurnEnd {
                        completed: true,
                    }));
                } else {
                    acc.events.push(event);
                }
                acc
            },
        )
    }
}

pub struct VerificationLoopContext {
    machine: VerificationMachine,
}

impl VerificationLoopContext {
    pub fn new(max_attempts: u32) -> Self {
        Self {
            machine: VerificationMachine::initial(max_attempts),
        }
    }

    pub fn from_machine(machine: VerificationMachine) -> Self {
        Self {
            machine,
        }
    }

    pub fn state(&self) -> &VerificationMachine { &self.machine }

    pub fn into_state(self) -> VerificationMachine { self.machine }

    pub fn step(self, input: VerificationInput) -> Transition<Self, VerificationEffect> {
        transition_verification(self.machine, input).map_state(Self::from_machine)
    }

    pub fn start_attempt(self, job_id: i64, initial_revision: CodeVersion, commands: Vec<Vec<String>>) -> Transition<Self, VerificationEffect> {
        self.step(VerificationInput::StartVerification {
            job_id,
            initial_revision,
            commands,
        })
    }

    pub fn mark_running(self) -> Transition<Self, VerificationEffect> { self.step(VerificationInput::CheckRunning) }

    pub fn complete_check(self, result: VerificationCheck) -> Transition<Self, VerificationEffect> {
        self.step(VerificationInput::CheckCompleted {
            result,
        })
    }

    pub fn finish_revision(self, finished_revision: CodeVersion) -> Transition<Self, VerificationEffect> {
        self.step(VerificationInput::FinishedRevisionCaptured {
            finished_revision,
        })
    }

    pub fn cancel(self) -> Transition<Self, VerificationEffect> { self.step(VerificationInput::Cancel) }

    pub fn prepare_repair<R: RepairRequestBuilder>(request: &R, report: &VerificationReport, conversation_id: Option<i64>) -> ChatRequest {
        request.next_repair_turn(report, conversation_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_agent_contracts::{JobScope,
                               JobState,
                               Provider};

    fn sample_job(state: JobState) -> Job {
        Job {
            id: 1,
            scope: JobScope {
                provider: Provider::Codex,
                account_id: "default".into(),
                workspace_id: Some(1),
                workspace_path: "/repo".into(),
            },
            state,
            conversation_id: None,
            operation_id: None,
            pending_request_id: None,
            last_sequence: 0,
            preview: "task".into(),
            cancel_requested: false,
            error: None,
            created_at: 1,
        }
    }

    #[test]
    fn job_projection_context_projects_immutably_and_ignores_post_terminal_events() {
        let initial = sample_job(JobState::Running);
        let started = JobProjectionContext::new(&initial)
            .apply(&StreamEvent::Started {
                operation_id: 77,
            })
            .unwrap();
        assert_eq!(started.operation_id, Some(77));
        assert_eq!(initial.operation_id, None);

        let completed = JobProjectionContext::new(&started)
            .apply(&StreamEvent::Completed {
                session_id: "s".into(),
                text: "ok".into(),
            })
            .unwrap();
        assert_eq!(completed.state, JobState::Completed);
        assert!(JobProjectionContext::new(&completed).apply(&StreamEvent::Cancelled).is_none());
    }

    #[test]
    fn turn_batch_context_withholds_completion_when_verification_is_enabled() {
        let batch = vec![
            StreamEvent::Started {
                operation_id: 9,
            },
            StreamEvent::Completed {
                session_id: "s1".into(),
                text: "done".into(),
            },
        ];
        let outcome = TurnBatchContext::new(true).process(batch, false);
        assert_eq!(outcome.started_operations, vec![9]);
        assert!(outcome.saw_terminal);
        assert!(matches!(outcome.withheld_completion, Some(StreamEvent::Completed { .. })));
        assert!(matches!(
            outcome.events.last(),
            Some(StreamEvent::Agent(Event::TurnEnd {
                completed: true
            }))
        ));
    }
}
