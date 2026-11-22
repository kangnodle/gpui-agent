use anyhow::Result;
use gpui_agent_application::{Application,
                             ErrorKind,
                             ports::{AgentRuntime,
                                     ConversationRepository,
                                     EditOutcome,
                                     RuntimeFuture,
                                     Turn}};
use gpui_agent_contracts::*;
use std::sync::{Arc,
                Mutex};

#[derive(Default)]
struct RecordingHistory {
    log: Mutex<Vec<String>>,
    fail_event: bool,
    fail_finish: bool,
    finished: tokio::sync::Notify,
}

impl ConversationRepository for RecordingHistory {
    fn list(&self, _: Provider, _: &str) -> Result<Vec<ConversationSummary>> { Ok(Vec::new()) }

    fn load(&self, _: Provider, _: i64) -> Result<Option<SavedConversation>> { Ok(None) }

    fn transfer(&self, _: Provider, _: &ContextTransferCommand) -> Result<ContextTransfer> { unreachable!() }

    fn transfers(&self, _: Provider, _: i64, _: i64) -> Result<Vec<ContextTransfer>> { unreachable!() }

    fn fork(&self, _: Provider, _: i64, _: i64, _: &str, _: &[Attachment]) -> Result<gpui_agent_application::ports::ForkOutcome> { unreachable!() }

    fn versions(&self, _: Provider, _: i64) -> Result<Vec<ConversationVersion>> { unreachable!() }

    fn begin(&self, _: Provider, conversation_id: Option<i64>, prompt: &mut Prompt) -> Result<Turn> {
        self.log.lock().unwrap().push("begin".into());
        assert_eq!(conversation_id, Some(7));
        prompt.session_id = Some("stored-session".into());
        Ok(Turn {
            conversation_id: 7,
            message_id: 8,
        })
    }

    fn rename(&self, _: Provider, _: i64, title: &str) -> Result<EditOutcome> {
        assert_eq!(title, "새 제목");
        Ok(EditOutcome::Busy)
    }

    fn set_state(&self, _provider: Provider, _id: i64, _state: &gpui_agent_domain::ConversationState) -> anyhow::Result<EditOutcome> {
        Ok(EditOutcome::NotFound)
    }

    fn delete(&self, _: Provider, _: i64) -> Result<EditOutcome> { Ok(EditOutcome::NotFound) }

    fn event(&self, _: Turn, event: &Event) -> Result<()> {
        if self.fail_event {
            anyhow::bail!("storage offline");
        }
        if let Event::Text {
            delta,
        } = event
        {
            self.log.lock().unwrap().push(delta.clone());
        }
        Ok(())
    }

    fn finish(&self, _: Turn, completion: Option<&Completion>) -> Result<()> {
        if self.fail_finish {
            self.log.lock().unwrap().push("finish failed".into());
            self.finished.notify_one();
            anyhow::bail!("finalization offline");
        }
        if let Some(completion) = completion {
            assert_eq!(completion.session_id, "stored-session");
            assert_eq!(completion.text, "최종 응답");
        }
        self.log
            .lock()
            .unwrap()
            .push(if completion.is_some() { "complete" } else { "incomplete" }.into());
        self.finished.notify_one();
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Steer,
    Burst,
    Complete,
    Fail,
    Pending,
    Interaction,
}
struct Runtime(Mode);

#[tokio::test]
async fn given_an_active_operation_when_live_input_arrives_then_delivery_is_observable_and_stale_operations_are_rejected() -> Result<()> {
    let app = application(Mode::Steer, Arc::new(RecordingHistory::default()));
    let mut events = app.chat(Provider::Claude, request("hello")).await?;
    let Some(StreamEvent::Started {
        operation_id,
    }) = events.recv().await
    else {
        panic!("Missing operation")
    };
    app.steer(
        operation_id,
        gpui_agent_ports::RuntimeInput {
            message_id: 1,
            text: "Updated instruction".into(),
        },
    )?;
    let mut delivered = false;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while let Some(event) = events.recv().await {
            if matches!(
                event,
                StreamEvent::Agent(Event::InputDelivery {
                    message_id: 1,
                    delivered: true
                })
            ) {
                delivered = true;
            }
            if event.is_terminal() {
                assert!(matches!(event, StreamEvent::Completed { .. }));
                break;
            }
        }
    })
    .await?;
    assert!(delivered);
    assert!(
        app.steer(
            operation_id,
            gpui_agent_ports::RuntimeInput {
                message_id: 2,
                text: "Too late".into()
            }
        )
        .is_err()
    );
    Ok(())
}
impl AgentRuntime for Runtime {
    fn supports_steering(&self) -> bool { matches!(self.0, Mode::Steer) }

    fn prompt_with_input<'a>(
        &'a self,
        prompt: &'a Prompt,
        handler: &'a mut Handler<'_>,
        input: &'a mut dyn gpui_agent_ports::TurnInput,
    ) -> RuntimeFuture<'a, Completion> {
        if !matches!(self.0, Mode::Steer) {
            return self.prompt(prompt, handler);
        }
        Box::pin(async move {
            let message = input.next().await?.unwrap();
            assert_eq!(message.text, "Updated instruction");
            handler.emit(Event::InputDelivery {
                message_id: message.message_id,
                delivered: true,
            });
            Ok(Completion {
                session_id: "stored-session".into(),
                text: "최종 응답".into(),
            })
        })
    }

    fn login<'a>(&'a self, _: bool, _: &'a mut Handler<'_>) -> RuntimeFuture<'a, AuthStatus> { self.status() }

    fn status(&self) -> RuntimeFuture<'_, AuthStatus> {
        Box::pin(async {
            Ok(AuthStatus {
                authenticated: true,
                account: None,
                method: None,
            })
        })
    }

    fn logout(&self) -> RuntimeFuture<'_, ()> { Box::pin(async { Ok(()) }) }

    fn models(&self) -> RuntimeFuture<'_, Vec<Model>> { Box::pin(async { Ok(Vec::new()) }) }

    fn prompt<'a>(&'a self, prompt: &'a Prompt, handler: &'a mut Handler<'_>) -> RuntimeFuture<'a, Completion> {
        Box::pin(async move {
            assert_eq!(prompt.text, "hello");
            assert_eq!(prompt.session_id.as_deref(), Some("stored-session"));
            assert!(
                !handler
                    .emit_async(Event::Text {
                        delta: "부분 응답".into()
                    })
                    .await
            );
            if matches!(self.0, Mode::Burst) {
                for index in 0 .. 1500 {
                    handler
                        .emit_async(Event::Text {
                            delta: format!("chunk-{index}"),
                        })
                        .await;
                }
                tokio::task::yield_now().await;
            }
            match self.0 {
                | Mode::Complete | Mode::Burst => Ok(Completion {
                    session_id: "stored-session".into(),
                    text: "최종 응답".into(),
                }),
                | Mode::Fail => anyhow::bail!("runtime failed"),
                | Mode::Pending | Mode::Steer => std::future::pending().await,
                | Mode::Interaction => {
                    let _ = handler
                        .respond(Event::Permission {
                            method: "fixture".into(),
                            details: serde_json::json!({}),
                        })
                        .await;
                    Ok(Completion {
                        session_id: "stored-session".into(),
                        text: "최종 응답".into(),
                    })
                },
            }
        })
    }
}

fn application(mode: Mode, history: Arc<RecordingHistory>) -> Application {
    Application::new(
        [Ok(Arc::new(Runtime(mode))), Err(anyhow::anyhow!("missing")), Err(anyhow::anyhow!("missing"))],
        history,
    )
}

fn request(text: &str) -> ChatRequest {
    ChatRequest {
        extensions: None,
        skills: vec![],
        computer_use: false,
        claude_permission_mode: None,
        codex_execution_policy: None,
        attachments: Vec::new(),
        text: text.into(),
        model: None,
        session_id: Some("untrusted-session".into()),
        conversation_id: Some(7),
    }
}

#[tokio::test]
async fn given_a_burst_larger_than_the_stream_buffer_when_drained_then_every_chunk_arrives_in_order_and_completes() -> Result<()> {
    let app = application(Mode::Burst, Arc::new(RecordingHistory::default()));
    let mut events = app.chat(Provider::Claude, request("hello")).await?;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut chunks = 0;
        let mut completed = false;
        while let Some(event) = events.recv().await {
            match event {
                | StreamEvent::Agent(Event::Text {
                    delta,
                }) if delta != "부분 응답" => {
                    assert_eq!(delta, format!("chunk-{chunks}"));
                    chunks += 1;
                },
                | StreamEvent::Completed {
                    ..
                } => completed = true,
                | StreamEvent::Error {
                    message,
                } => panic!("Stream failed: {message}"),
                | _ => {},
            }
        }
        assert!(completed);
        assert_eq!(chunks, 1500);
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn given_a_full_stream_when_cancelled_or_disconnected_then_the_operation_stops_and_finalizes() -> Result<()> {
    for disconnect in [false, true] {
        let history = Arc::new(RecordingHistory::default());
        let app = application(Mode::Burst, history.clone());
        let mut events = app.chat(Provider::Claude, request("hello")).await?;
        let Some(StreamEvent::Started {
            operation_id,
        }) = events.recv().await
        else {
            panic!("Missing operation")
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while events.capacity() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        if disconnect {
            drop(events);
        } else {
            assert!(app.cancel(operation_id));
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while let Some(event) = events.recv().await {
                    if event.is_terminal() {
                        assert!(matches!(event, StreamEvent::Cancelled));
                        return;
                    }
                }
                panic!("Missing cancellation");
            })
            .await?;
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), history.finished.notified()).await?;
        assert_eq!(history.log.lock().unwrap().last().unwrap(), "incomplete");
        assert!(!app.cancel(operation_id));
    }
    Ok(())
}

#[tokio::test]
async fn injected_ports_preserve_streaming_storage_and_cancellation() -> Result<()> {
    for (mode, fail_event, fail_finish, cancel, complete) in [
        (Mode::Complete, false, false, false, true),
        (Mode::Fail, false, false, false, false),
        (Mode::Complete, true, false, false, false),
        (Mode::Complete, false, true, false, false),
        (Mode::Pending, false, false, true, false),
    ] {
        let history = Arc::new(RecordingHistory {
            fail_event,
            fail_finish,
            ..Default::default()
        });
        let app = application(mode, history.clone());
        let mut events = app.chat(Provider::Claude, request("hello")).await?;
        let StreamEvent::Started {
            operation_id,
        } = events.recv().await.unwrap()
        else {
            panic!("missing operation id")
        };
        assert!(matches!(
            events.recv().await,
            Some(StreamEvent::Conversation {
                conversation_id: 7
            })
        ));
        let mut terminal = None;
        while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv()).await? {
            if let StreamEvent::Agent(Event::Text {
                ..
            }) = &event
            {
                assert_eq!(history.log.lock().unwrap()[1], "부분 응답", "persist before delivery");
                if cancel {
                    assert!(app.cancel(operation_id));
                }
            }
            if event.is_terminal() {
                terminal = Some(event);
            }
        }
        let terminal = terminal.unwrap();
        assert_eq!(matches!(terminal, StreamEvent::Completed { .. }), complete);
        if fail_event {
            assert!(matches!(terminal, StreamEvent::Error { ref message } if message.contains("storage offline")));
        }
        if fail_finish {
            assert!(matches!(terminal, StreamEvent::Error { ref message } if message.contains("finalization offline")));
        }
        if cancel {
            assert!(matches!(terminal, StreamEvent::Cancelled));
        }
        assert_eq!(
            history.log.lock().unwrap().last().unwrap(),
            if fail_finish {
                "finish failed"
            } else if complete {
                "complete"
            } else {
                "incomplete"
            }
        );
        assert!(!app.cancel(operation_id), "terminal operations must be removed");
    }
    Ok(())
}

#[tokio::test]
async fn validation_and_history_do_not_require_a_native_runtime() -> Result<()> {
    let history = Arc::new(RecordingHistory::default());
    let app = application(Mode::Complete, history.clone());
    assert!(matches!(app.chat(Provider::Claude, request(" ")).await, Err(error) if error.0 == ErrorKind::Invalid));
    assert!(history.log.lock().unwrap().is_empty());
    assert!(!app.providers()[1].available);
    assert!(matches!(app.status(Provider::Codex).await, Err(error) if error.0 == ErrorKind::Unavailable));
    assert!(app.conversations(Provider::Codex, "세계".into()).await?.is_empty());
    assert!(matches!(app.conversation(Provider::Codex, 7).await, Err(error) if error.0 == ErrorKind::NotFound));
    assert!(matches!(app.rename_conversation(Provider::Codex, 7, "  새 제목  ".into()).await, Err(error) if error.0 == ErrorKind::Busy));
    assert!(matches!(app.delete_conversation(Provider::Codex, 7).await, Err(error) if error.0 == ErrorKind::NotFound));
    assert!(matches!(app.conversations(Provider::Codex, "한".repeat(257)).await, Err(error) if error.0 == ErrorKind::Invalid));
    Ok(())
}

#[tokio::test]
async fn disconnect_and_host_shutdown_finalize_interrupted_turns() -> Result<()> {
    for disconnect in [true, false] {
        let history = Arc::new(RecordingHistory::default());
        let app = application(Mode::Pending, history.clone());
        let mut events = app.chat(Provider::Claude, request("hello")).await?;
        let StreamEvent::Started {
            operation_id,
        } = events.recv().await.unwrap()
        else {
            panic!("missing operation id")
        };
        assert!(matches!(events.recv().await, Some(StreamEvent::Conversation { .. })));
        assert!(matches!(events.recv().await, Some(StreamEvent::Agent(Event::Text { .. }))));
        if disconnect {
            drop(events);
        } else {
            app.cancel_all();
            assert!(matches!(events.recv().await, Some(StreamEvent::Cancelled)));
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), history.finished.notified()).await?;
        assert_eq!(*history.log.lock().unwrap(), ["begin", "부분 응답", "incomplete"]);
        assert!(!app.cancel(operation_id));
    }
    Ok(())
}

#[tokio::test]
async fn pending_interactions_are_revoked_on_disconnect_and_host_shutdown() -> Result<()> {
    for disconnect in [true, false] {
        let history = Arc::new(RecordingHistory::default());
        let app = application(Mode::Interaction, history.clone());
        let mut events = app.chat(Provider::Claude, request("hello")).await?;
        let (operation, request_id) = loop {
            if let Some(StreamEvent::Interaction {
                operation_id,
                request_id,
                ..
            }) = events.recv().await
            {
                break (operation_id, request_id);
            }
        };
        if disconnect {
            drop(events);
        } else {
            app.cancel_all();
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), history.finished.notified()).await?;
        assert!(matches!(app.respond(operation,request_id,InteractionResponse::Approval { allow:true }),Err(error) if error.0 == ErrorKind::NotFound));
        assert_eq!(history.log.lock().unwrap().last().unwrap(), "incomplete");
    }
    Ok(())
}
