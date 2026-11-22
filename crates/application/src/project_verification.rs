use crate::{Application,
            ApplicationResult,
            Jobs};
use gpui_agent_contracts::*;
use gpui_agent_dci::job_execution::VerificationLoopContext;

pub(crate) enum VerificationAttemptOutcome {
    Passed,
    Repair { instruction: String },
    Cancelled,
    Exhausted { max_attempts: u32 },
}

impl Jobs {
    async fn record_check(&self, id: i64, application: &Application, turn: crate::ports::Turn, report: &VerificationReport) -> ApplicationResult<()> {
        application.record_verification(turn, report.clone()).await?;
        let event = StreamEvent::Agent(Event::Verification {
            report: report.clone(),
        });
        self.io(move |repo| repo.append(id, &event)).await
    }

    pub(crate) async fn run_verification_attempt(
        &self,
        id: i64,
        scope: &JobScope,
        application: &Application,
        turn: crate::ports::Turn,
        plan: &ProjectVerification,
        loop_context: VerificationLoopContext,
    ) -> ApplicationResult<(VerificationLoopContext, VerificationAttemptOutcome)> {
        let verifier = self
            .0
            .verifier
            .as_ref()
            .ok_or("Required project verification has no configured host verifier")?;
        let initial_revision = verifier.revision(scope).await?;
        let mut transition = loop_context.start_attempt(id, initial_revision, plan.commands.clone());
        loop {
            let mut next_input = None;
            let mut terminal_outcome = None;
            for effect in transition.effects {
                match effect {
                    | VerificationEffect::RecordReport(report) => {
                        self.record_check(id, application, turn, &report).await?;
                    },
                    | VerificationEffect::ExecuteCheck {
                        command, ..
                    } => {
                        let (cancel, cancelled) = tokio::sync::oneshot::channel();
                        self.0
                            .checks
                            .lock()
                            .map_err(|_| "Verification cancellation registry unavailable")?
                            .insert(id, cancel);
                        if self.job(id).await?.cancel_requested || self.is_stopping() {
                            self.0.checks.lock().map_err(|_| "Verification cancellation registry unavailable")?.remove(&id);
                        }
                        let running = transition.state.mark_running();
                        for running_effect in running.effects {
                            if let VerificationEffect::RecordReport(report) = running_effect {
                                self.record_check(id, application, turn, &report).await?;
                            }
                        }
                        transition.state = running.state;
                        let result = tokio::select! {
                            biased;
                            _ = cancelled => None,
                            result = verifier.check(scope, &command, plan.timeout_seconds) => Some(result),
                        };
                        self.0.checks.lock().map_err(|_| "Verification cancellation registry unavailable")?.remove(&id);
                        next_input = Some(match result {
                            | None => VerificationInput::Cancel,
                            | Some(result) => VerificationInput::CheckCompleted {
                                result: result.unwrap_or_else(|error| VerificationCheck {
                                    command,
                                    status: VerificationStatus::Failed,
                                    exit_code: None,
                                    output: error.to_string(),
                                }),
                            },
                        });
                    },
                    | VerificationEffect::CaptureFinishedRevision => {
                        let finished_revision = verifier.revision(scope).await?;
                        next_input = Some(VerificationInput::FinishedRevisionCaptured {
                            finished_revision,
                        });
                    },
                    | VerificationEffect::CompleteJob => {
                        terminal_outcome = Some(VerificationAttemptOutcome::Passed);
                    },
                    | VerificationEffect::ScheduleRepair {
                        instruction, ..
                    } => {
                        terminal_outcome = Some(VerificationAttemptOutcome::Repair {
                            instruction,
                        });
                    },
                    | VerificationEffect::CancelJob => {
                        terminal_outcome = Some(VerificationAttemptOutcome::Cancelled);
                    },
                    | VerificationEffect::FailExhausted {
                        max_attempts,
                    } => {
                        terminal_outcome = Some(VerificationAttemptOutcome::Exhausted {
                            max_attempts,
                        });
                    },
                }
            }
            if let Some(outcome) = terminal_outcome {
                return Ok((transition.state, outcome));
            }
            let Some(input) = next_input else {
                return Ok((transition.state, VerificationAttemptOutcome::Cancelled));
            };
            transition = transition.state.step(input);
        }
    }
}
