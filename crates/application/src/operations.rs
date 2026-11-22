//! Operation execution, cancellation and persistence-before-delivery ordering.
use crate::{Application,
            ports::{AgentRuntime,
                    Turn}};
use gpui_agent_contracts::StreamEvent;
use gpui_agent_domain::{AuthStatus,
                        Completion,
                        EventHandler,
                        Prompt};
use std::sync::{Arc,
                atomic::Ordering};
use tokio::sync::{Notify,
                  mpsc,
                  oneshot};

pub(super) enum Operation {
    Login(bool),
    Chat(Box<Prompt>, Option<i64>, bool),
}
enum OperationOutput {
    Authenticated(AuthStatus),
    Completed(Completion),
}

impl Application {
    pub fn steer(&self, id: u64, input: gpui_agent_ports::RuntimeInput) -> crate::ApplicationResult<()> {
        if input.message_id <= 0 || input.text.trim().is_empty() || input.text.len() > 33000 {
            return Err("Live input requires a positive message ID and 1–33000 bytes of text".into());
        }
        let inputs = self.0.inputs.lock().map_err(|_| "Live input registry unavailable")?;
        let sender = inputs.get(&id).ok_or(crate::ApplicationError(
            crate::ErrorKind::NotFound,
            "Operation is no longer accepting live input".into(),
        ))?;
        sender
            .try_send(input)
            .map_err(|_| crate::ApplicationError(crate::ErrorKind::Busy, "Live input queue is full or closed".into()))
    }

    pub(super) fn stream(
        &self,
        agent: Arc<dyn AgentRuntime>,
        operation: Operation,
        turn: Option<Turn>,
    ) -> crate::ApplicationResult<mpsc::Receiver<StreamEvent>> {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let (tx, rx) = mpsc::channel(256);
        let steering = matches!(&operation, Operation::Chat(..)) && agent.supports_steering();
        let job_id = match &operation {
            | Operation::Chat(_, job, _) => *job,
            | _ => None,
        };
        let (input_tx, input_rx) = mpsc::channel(16);
        let setup = (|| -> anyhow::Result<()> {
            let mut operations = match self.0.operations.lock() {
                | Ok(operations) => operations,
                | Err(error) => anyhow::bail!("Operation registry lock poisoned: {error}"),
            };
            match tx.try_send(StreamEvent::Started {
                operation_id: id,
            }) {
                | Ok(()) => {},
                | Err(error) => anyhow::bail!("Cannot queue operation start: {error}"),
            }
            if let Some(turn) = turn {
                match tx.try_send(StreamEvent::Conversation {
                    conversation_id: turn.conversation_id,
                }) {
                    | Ok(()) => {},
                    | Err(error) => anyhow::bail!("Cannot queue conversation start: {error}"),
                }
            }
            if steering {
                self.0
                    .inputs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Live input registry unavailable"))?
                    .insert(id, input_tx);
            }
            operations.insert(id, cancel_tx);
            Ok(())
        })();
        match setup {
            | Ok(()) => {},
            | Err(error) => {
                if let Some(turn) = turn {
                    let timeline = self.0.history.event(
                        turn,
                        &gpui_agent_domain::Event::TurnEnd {
                            completed: false,
                        },
                    );
                    let finish = self.0.history.finish(turn, None);
                    if let Err(finish_error) = timeline.and(finish) {
                        return Err(anyhow::anyhow!("{error}; cannot finalize unstarted conversation: {finish_error}").into());
                    }
                }
                return Err(error.into());
            },
        }
        let application = self.clone();
        tokio::spawn(async move {
            let mut input = OperationInput(input_rx);
            let overflow = Notify::new();
            let mut storage_error = None;
            let mut outcome = {
                let mut handler = OperationHandler {
                    job_id,
                    application: &application,
                    operation_id: id,
                    turn,
                    tx: &tx,
                    overflow: &overflow,
                    storage_error: &mut storage_error,
                };
                let work = async {
                    match operation {
                        | Operation::Login(device_code) => agent.login(device_code, &mut handler).await.map(OperationOutput::Authenticated),
                        | Operation::Chat(prompt, _, computer_use) => {
                            if computer_use
                                && !application.0.automatic_computer
                                && !matches!(
                                    handler
                                        .respond(gpui_agent_domain::Event::Permission {
                                            method: "computer/request_control".into(),
                                            details: serde_json::json!({"job_id":job_id}),
                                        })
                                        .await,
                                    gpui_agent_domain::InteractionResponse::Approval {
                                        allow: true
                                    }
                                )
                            {
                                return Err(anyhow::anyhow!("Computer task start was not approved"));
                            }
                            if steering {
                                agent.prompt_with_input(&prompt, &mut handler, &mut input).await
                            } else {
                                agent.prompt(&prompt, &mut handler).await
                            }
                            .map(OperationOutput::Completed)
                        },
                    }
                };
                tokio::select! {
                    biased;
                    _ = cancel_rx => Err(StreamEvent::Cancelled),
                    _ = tx.closed() => Err(StreamEvent::Cancelled),
                    _ = overflow.notified() => Err(StreamEvent::Error { message:"Client is too slow; operation stopped without dropping response text silently".into() }),
                    result = work => result.map_err(|error| StreamEvent::Error { message:error.to_string() }),
                }
            };
            if let Some(message) = storage_error {
                outcome = Err(StreamEvent::Error {
                    message,
                });
            }
            let completion = match &outcome {
                | Ok(OperationOutput::Completed(completion)) => Some(completion),
                | _ => None,
            };
            if let Some(turn) = turn {
                let timeline = application.0.history.event(
                    turn,
                    &gpui_agent_domain::Event::TurnEnd {
                        completed: completion.is_some(),
                    },
                );
                let finish = application.0.history.finish(turn, completion);
                if let Err(error) = timeline.and(finish) {
                    outcome = Err(StreamEvent::Error {
                        message: format!("Cannot finalize conversation: {error}"),
                    });
                }
            }
            (match application.0.interactions.lock() {
                | Ok(guard) => guard,
                | Err(error) => {
                    application
                        .0
                        .diagnostics
                        .warn(&format!("Interaction registry lock poisoned; recovering entries for cleanup: {error}"));
                    error.into_inner()
                },
            })
            .retain(|_, pending| pending.operation_id != id);
            (match application.0.operations.lock() {
                | Ok(guard) => guard,
                | Err(error) => {
                    application
                        .0
                        .diagnostics
                        .warn(&format!("Operation registry lock poisoned; recovering entries for cleanup: {error}"));
                    error.into_inner()
                },
            })
            .remove(&id);
            let terminal = match outcome {
                | Ok(OperationOutput::Authenticated(status)) => StreamEvent::Authenticated {
                    status,
                },
                | Ok(OperationOutput::Completed(reply)) => StreamEvent::Completed {
                    session_id: reply.session_id,
                    text: reply.text,
                },
                | Err(event) => event,
            };
            if let Ok(mut inputs) = application.0.inputs.lock() {
                inputs.remove(&id);
            }
            let _ = tx.send(terminal).await;
        });
        Ok(rx)
    }
}

struct OperationInput(mpsc::Receiver<gpui_agent_ports::RuntimeInput>);
impl gpui_agent_ports::TurnInput for OperationInput {
    fn next(&mut self) -> gpui_agent_ports::RuntimeFuture<'_, Option<gpui_agent_ports::RuntimeInput>> { Box::pin(async { Ok(self.0.recv().await) }) }
}

struct OperationHandler<'a> {
    job_id: Option<i64>,
    application: &'a Application,
    operation_id: u64,
    turn: Option<Turn>,
    tx: &'a mpsc::Sender<StreamEvent>,
    overflow: &'a Notify,
    storage_error: &'a mut Option<String>,
}
impl gpui_agent_domain::EventHandler for OperationHandler<'_> {
    fn job_id(&self) -> Option<i64> { self.job_id }

    fn conversation_id(&self) -> Option<i64> { self.turn.map(|turn| turn.conversation_id) }

    fn emit_async(&mut self, event: gpui_agent_domain::Event) -> gpui_agent_domain::EmitFuture<'_> {
        Box::pin(async move {
            if let Some(turn) = self.turn
                && let Err(error) = self.application.0.history.event(turn, &event)
            {
                *self.storage_error = Some(format!("Cannot save conversation: {error}"));
                self.overflow.notify_one();
                return false;
            }
            if self.tx.send(StreamEvent::Agent(event)).await.is_err() {
                self.overflow.notify_one();
            }
            false
        })
    }

    fn emit(&mut self, event: gpui_agent_domain::Event) -> bool {
        if let Some(turn) = self.turn
            && let Err(error) = self.application.0.history.event(turn, &event)
        {
            *self.storage_error = Some(format!("Cannot save conversation: {error}"));
            self.overflow.notify_one();
            return false;
        }
        if self.tx.try_send(StreamEvent::Agent(event)).is_err() {
            self.overflow.notify_one();
        }
        false
    }

    fn respond(&mut self, event: gpui_agent_domain::Event) -> gpui_agent_domain::ResponseFuture<'_> {
        Box::pin(async move {
            let request_id = self.application.0.next_id.fetch_add(1, Ordering::Relaxed);
            if self.job_id.is_some()
                && self.application.0.automatic_computer
                && let Some(response) = event.automatic_computer_response()
            {
                self.emit_async(event.clone()).await;
                self.emit_async(gpui_agent_domain::Event::InteractionResolved {
                    request_id,
                    response: response.clone(),
                })
                .await;
                return if self.storage_error.is_some() {
                    gpui_agent_domain::InteractionResponse::Cancelled
                } else {
                    response
                };
            }
            let (sender, receiver) = oneshot::channel();
            match self.application.0.interactions.lock() {
                | Ok(mut interactions) => {
                    interactions.insert(
                        request_id,
                        crate::PendingInteraction {
                            operation_id: self.operation_id,
                            event: event.clone(),
                            sender,
                        },
                    );
                },
                | Err(error) => {
                    *self.storage_error = Some(format!("Cannot register interaction: registry lock poisoned: {error}"));
                    self.overflow.notify_one();
                    return gpui_agent_domain::InteractionResponse::Cancelled;
                },
            }
            // Save before exposing the request; a storage failure must never
            // grant permission.
            self.emit_async(event.clone()).await;
            if self.storage_error.is_some()
                || self
                    .tx
                    .send(StreamEvent::Interaction {
                        operation_id: self.operation_id,
                        request_id,
                        event,
                    })
                    .await
                    .is_err()
            {
                self.overflow.notify_one();
                return gpui_agent_domain::InteractionResponse::Cancelled;
            }
            let response = receiver.await.unwrap_or(gpui_agent_domain::InteractionResponse::Cancelled);
            self.emit_async(gpui_agent_domain::Event::InteractionResolved {
                request_id,
                response: response.clone(),
            })
            .await;
            if self.storage_error.is_some() {
                gpui_agent_domain::InteractionResponse::Cancelled
            } else {
                response
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorKind,
                ports::{ConversationRepository,
                        EditOutcome,
                        RuntimeFuture}};
    use gpui_agent_domain::{ConversationState,
                            ConversationSummary,
                            Event,
                            EventHandler,
                            Handler,
                            InteractionResponse,
                            Model,
                            Provider,
                            SavedConversation};
    use std::{panic::{AssertUnwindSafe,
                      catch_unwind},
              sync::atomic::AtomicBool};

    #[derive(Default)]
    struct History {
        ended: AtomicBool,
        finished: AtomicBool,
    }
    impl ConversationRepository for History {
        fn list(&self, _: Provider, _: &str) -> anyhow::Result<Vec<ConversationSummary>> { unreachable!() }

        fn load(&self, _: Provider, _: i64) -> anyhow::Result<Option<SavedConversation>> { unreachable!() }

        fn transfer(&self, _: Provider, _: &gpui_agent_domain::ContextTransferCommand) -> anyhow::Result<gpui_agent_domain::ContextTransfer> { unreachable!() }

        fn transfers(&self, _: Provider, _: i64, _: i64) -> anyhow::Result<Vec<gpui_agent_domain::ContextTransfer>> { unreachable!() }

        fn fork(&self, _: Provider, _: i64, _: i64, _: &str, _: &[gpui_agent_domain::Attachment]) -> anyhow::Result<crate::ports::ForkOutcome> {
            unreachable!()
        }

        fn versions(&self, _: Provider, _: i64) -> anyhow::Result<Vec<gpui_agent_domain::ConversationVersion>> { unreachable!() }

        fn begin(&self, _: Provider, _: Option<i64>, _: &mut Prompt) -> anyhow::Result<Turn> { unreachable!() }

        fn rename(&self, _: Provider, _: i64, _: &str) -> anyhow::Result<EditOutcome> { unreachable!() }

        fn set_state(&self, _: Provider, _: i64, _: &ConversationState) -> anyhow::Result<EditOutcome> { unreachable!() }

        fn delete(&self, _: Provider, _: i64) -> anyhow::Result<EditOutcome> { unreachable!() }

        fn event(&self, _: Turn, event: &Event) -> anyhow::Result<()> {
            assert!(matches!(
                event,
                Event::TurnEnd {
                    completed: false
                }
            ));
            self.ended.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn finish(&self, _: Turn, completion: Option<&Completion>) -> anyhow::Result<()> {
            assert!(completion.is_none());
            self.finished.store(true, Ordering::SeqCst);
            Ok(())
        }
    }
    struct NeverRun;
    impl AgentRuntime for NeverRun {
        fn login<'a>(&'a self, _: bool, _: &'a mut Handler<'_>) -> RuntimeFuture<'a, AuthStatus> { unreachable!() }

        fn status(&self) -> RuntimeFuture<'_, AuthStatus> { unreachable!() }

        fn logout(&self) -> RuntimeFuture<'_, ()> { unreachable!() }

        fn models(&self) -> RuntimeFuture<'_, Vec<Model>> { unreachable!() }

        fn prompt<'a>(&'a self, _: &'a Prompt, _: &'a mut Handler<'_>) -> RuntimeFuture<'a, Completion> { unreachable!() }
    }
    fn application(history: Arc<History>) -> Application { Application::new(std::array::from_fn(|_| Err(anyhow::anyhow!("unavailable"))), history) }

    #[tokio::test]
    async fn poisoned_operations_finalize_unstarted_turns_and_still_cancel_on_shutdown() {
        let history = Arc::new(History::default());
        let app = application(history.clone());
        let (cancel, cancelled) = oneshot::channel();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let mut operations = app.0.operations.lock().unwrap();
                operations.insert(7, cancel);
                panic!("poison operation registry");
            }))
            .is_err()
        );
        assert!(app.is_busy());
        let error = app
            .stream(
                Arc::new(NeverRun),
                Operation::Chat(Box::default(), None, false),
                Some(Turn {
                    conversation_id: 1,
                    message_id: 2,
                }),
            )
            .unwrap_err();
        assert_eq!(error.0, ErrorKind::Failure);
        assert!(error.1.contains("Operation registry lock poisoned"));
        assert!(history.ended.load(Ordering::SeqCst) && history.finished.load(Ordering::SeqCst));
        assert_eq!(
            app.respond(
                7,
                8,
                InteractionResponse::Approval {
                    allow: true
                }
            )
            .unwrap_err()
            .0,
            ErrorKind::Failure
        );
        app.cancel_all();
        tokio::time::timeout(std::time::Duration::from_secs(1), cancelled).await.unwrap().unwrap();
        drop(app);
    }

    #[tokio::test]
    async fn poisoned_interactions_reject_responses_and_release_waiters_on_cancel() {
        let app = application(Arc::new(History::default()));
        let (cancel, cancelled) = oneshot::channel();
        app.0.operations.lock().unwrap().insert(7, cancel);
        let (sender, receiver) = oneshot::channel();
        let event = Event::Permission {
            method: "tool".into(),
            details: serde_json::json!({}),
        };
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let mut interactions = app.0.interactions.lock().unwrap();
                interactions.insert(
                    8,
                    crate::PendingInteraction {
                        operation_id: 7,
                        event: event.clone(),
                        sender,
                    },
                );
                panic!("poison interaction registry");
            }))
            .is_err()
        );
        assert_eq!(
            app.respond(
                7,
                8,
                InteractionResponse::Approval {
                    allow: true
                }
            )
            .unwrap_err()
            .0,
            ErrorKind::Failure
        );
        let (tx, mut rx) = mpsc::channel(1);
        let overflow = Notify::new();
        let mut storage_error = None;
        let mut handler = OperationHandler {
            job_id: None,
            application: &app,
            operation_id: 7,
            turn: None,
            tx: &tx,
            overflow: &overflow,
            storage_error: &mut storage_error,
        };
        assert_eq!(handler.respond(event).await, InteractionResponse::Cancelled);
        assert!(storage_error.as_deref().unwrap().contains("registry lock poisoned"));
        assert!(rx.try_recv().is_err());
        assert!(app.cancel(7));
        tokio::time::timeout(std::time::Duration::from_secs(1), cancelled).await.unwrap().unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_secs(1), receiver).await.unwrap().is_err());
        drop(app);
    }
}
