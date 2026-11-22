use super::*;
use gpui_agent_application::ports::{ComputerSession,
                                    ControlValidity,
                                    HostControl,
                                    RuntimeFuture};
use std::sync::atomic::AtomicIsize;

// Public fixture credentials must never reach real browser, device or desktop
// I/O.
struct FixtureControls(gpui_agent_control::Controls);
impl FixtureControls {
    fn new() -> Self { Self(gpui_agent_control::Controls::default().with_desktop_driver(Arc::new(DesktopFixture::default()))) }
}
impl HostControl for FixtureControls {
    fn computer_status(&self) -> RuntimeFuture<'_, ComputerStatus> { self.0.computer_status() }

    fn computer(&self, session: ComputerSession, command: ComputerCommand) -> RuntimeFuture<'_, Option<ComputerFrame>> { self.0.computer(session, command) }

    fn grant_computer(&self, grant: ComputerGrantRequest, active: ControlValidity) -> RuntimeFuture<'_, ()> { self.0.grant_computer(grant, active) }

    fn status(&self) -> RuntimeFuture<'_, ControlStatus> { client_fixture::NoControls.status() }

    fn browser(&self, command: ControlCommand) -> RuntimeFuture<'_, Option<ControlFrame>> { client_fixture::NoControls.browser(command) }

    fn desktop(&self, command: ControlCommand) -> RuntimeFuture<'_, Option<ControlFrame>> { client_fixture::NoControls.desktop(command) }

    fn enable_desktop(&self, enabled: bool) -> RuntimeFuture<'_, ()> { client_fixture::NoControls.enable_desktop(enabled) }

    fn shutdown(&self) { self.0.shutdown(); }
}

pub(super) async fn serve_fixture() -> Result<()> {
    let server = Server::open_controls(Provider::Codex, "workflow", Arc::new(FixtureControls::new())).await?;
    for provider in PROVIDERS {
        server.login(provider, "default").await?;
    }
    let job = server.enqueue(Provider::Codex, "default", "Computer use UI fixture", None).await?;
    server.pending(job.id, 0).await?;
    let token_file = server.root.path().join("home/api.token");
    fs::write(&token_file, TOKEN)?;
    println!("Computer fixture API: {}", server.url);
    println!("Fixture-only token file: {}", token_file.display());
    println!("Synthetic provider and desktop driver; real host controls disabled. Press Enter to stop and remove state.");
    tokio::task::spawn_blocking(|| std::io::stdin().read_line(&mut String::new())).await??;
    server.stop().await
}

#[derive(Default)]
pub(super) struct DesktopFixture(pub(super) std::sync::Mutex<Vec<String>>, std::sync::atomic::AtomicBool);
impl gpui_agent_control::DesktopDriver for DesktopFixture {
    fn execute(&self, command: ControlCommand, _target: &AtomicIsize, allowed: &dyn Fn() -> bool) -> Result<Option<ControlFrame>> {
        ensure!(allowed(), "Execution expired before native input");
        ensure!(!self.1.swap(false, std::sync::atomic::Ordering::SeqCst), "Native observation failed");
        match command {
            | ControlCommand::Capture {} => self.0.lock().unwrap().push("capture".into()),
            | ControlCommand::Click {
                ..
            } => self.0.lock().unwrap().push("click".into()),
            | ControlCommand::Text {
                text,
            } => self.0.lock().unwrap().push(text),
            | _ => anyhow::bail!("Unexpected native input"),
        }
        Ok(Some(ControlFrame {
            width: 1,
            height: 1,
            url: None,
            image: Attachment::Image {
                name: "fixture.png".into(),
                mime_type: "image/png".into(),
                data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
            },
        }))
    }
}

pub(super) async fn run() -> Result<()> {
    for provider in [Provider::Claude, Provider::Antigravity] {
        automatic_native_permission(provider).await?;
    }
    for provider in PROVIDERS {
        automatic_computer_access_is_scoped(provider).await?;
    }
    submission_waits_for_permission().await?;
    cancellation_before_native_input().await?;
    let fixture = FixtureControls::new();
    ensure!(!fixture.status().await?.desktop_available && fixture.computer_status().await?.available);
    ensure!(fixture.desktop(ControlCommand::Capture {}).await.is_err());
    ensure!(fixture.browser(ControlCommand::Capture {}).await.is_err());
    for provider in PROVIDERS {
        run_provider(provider).await?;
    }
    println!("ok: Windows computer grants, execution isolation, capture/input/re-observation and job lifetime across three native fixtures");
    Ok(())
}

async fn automatic_native_permission(provider: Provider) -> Result<()> {
    let controls = Arc::new(
        gpui_agent_control::Controls::default()
            .with_desktop_driver(Arc::new(DesktopFixture::default()))
            .with_automatic_desktop(),
    );
    let server = Server::open_controls(provider, "interactive-allow", controls).await?;
    let path = server.root.path().join("project 한글/fixture.json");
    let mut spec: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    spec["computerPermissions"] = json!(true);
    fs::write(path, serde_json::to_vec(&spec)?)?;
    server.login(provider, "default").await?;
    let job = server.enqueue(provider, "default", "Automatic native approval", None).await?;
    server.wait(job.id, JobState::Completed).await?;
    server.stop().await
}

async fn automatic_computer_access_is_scoped(provider: Provider) -> Result<()> {
    let controls = Arc::new(
        gpui_agent_control::Controls::default()
            .with_desktop_driver(Arc::new(DesktopFixture::default()))
            .with_automatic_desktop(),
    );
    let server = Server::open_controls(provider, "interactive-allow", controls).await?;
    server.login(provider, "default").await?;
    let job: Job = server
        .request(Method::POST, &format!("/v1/providers/{}/jobs", provider_name(provider)), "default")
        .json(&json!({"text":"Computer task", "computer_use":true}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let (_, sequence) = server.pending(job.id, 0).await?;
    let mut stream = server
        .request(Method::GET, &format!("/v1/jobs/{}/events?after={}", job.id, sequence - 1), "default")
        .send()
        .await?
        .error_for_status()?;
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        while !bytes.contains(&b'\n') {
            bytes.extend_from_slice(&stream.chunk().await?.context("Missing interaction")?);
        }
        Ok::<JobEvent, anyhow::Error>(serde_json::from_slice(bytes.split(|byte| *byte == b'\n').next().unwrap())?)
    })
    .await??;
    ensure!(matches!(event.event, StreamEvent::Interaction { event: Event::Permission { method, .. }, .. } if method != "computer/request_control"));
    let job = server.job(job.id).await?.job;
    let scope = McpScope {
        provider: job.scope.provider,
        account_id: job.scope.account_id.clone(),
        workspace_path: job.scope.workspace_path.clone(),
    };
    let unbound = server.sessions.issue_for_job(scope.clone(), false, job.conversation_id, None)?;
    ensure!(server.mcp("computer_act", json!({}), &unbound.connection().token).await?["error"]["code"] == -32001);
    let session = server.sessions.issue_for_job(scope, false, job.conversation_id, Some(job.id))?;
    let token = &session.connection().token;
    let capture = server.mcp("computer_capture", json!({}), token).await?;
    ensure!(capture["result"]["isError"] == false, "{capture}");
    let acted = server
        .mcp(
            "computer_act",
            json!({"revision":capture["result"]["structuredContent"]["revision"],"action":{"type":"text","text":"fixture"}}),
            token,
        )
        .await?;
    ensure!(acted["result"]["isError"] == false, "{acted}");
    ensure!(server.mcp("cancel_job", json!({"id":job.id}), token).await?["error"]["code"] == -32001);
    server.stop().await
}

async fn cancellation_before_native_input() -> Result<()> {
    struct PausedInput {
        driver: Arc<DesktopFixture>,
        entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl gpui_agent_control::DesktopDriver for PausedInput {
        fn execute(&self, command: ControlCommand, target: &AtomicIsize, allowed: &dyn Fn() -> bool) -> Result<Option<ControlFrame>> {
            if matches!(command, ControlCommand::Text { .. }) {
                if let Some(entered) = self.entered.lock().unwrap().take() {
                    let _ = entered.send(());
                }
                self.release.lock().unwrap().recv_timeout(Duration::from_secs(5))?;
            }
            gpui_agent_control::DesktopDriver::execute(self.driver.as_ref(), command, target, allowed)
        }
    }
    // Given input admitted by HTTP/MCP but paused at native I/O, when the job
    // is cancelled, then the application validity callback denies delivery.
    let driver = Arc::new(DesktopFixture::default());
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let controls = Arc::new(gpui_agent_control::Controls::default().with_desktop_driver(Arc::new(PausedInput {
        driver: driver.clone(),
        entered: std::sync::Mutex::new(Some(entered)),
        release: std::sync::Mutex::new(resume),
    })));
    let server = Server::open_controls(Provider::Codex, "interactive-allow", controls).await?;
    server.login(Provider::Codex, "default").await?;
    let job = server.enqueue(Provider::Codex, "default", "Cancel admitted computer input", None).await?;
    server.pending(job.id, 0).await?;
    let job = server.job(job.id).await?.job;
    let session = server.sessions.issue_for_job(
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: job.scope.workspace_path.clone(),
        },
        true,
        job.conversation_id,
        Some(job.id),
    )?;
    server
        .request(Method::POST, "/v1/control/computer/grant", "default")
        .json(&json!({"job_id":job.id,"enabled":true}))
        .send()
        .await?
        .error_for_status()?;
    let capture = server.mcp("computer_capture", json!({}), &session.connection().token).await?;
    let command = json!({"revision":capture["result"]["structuredContent"]["revision"],"action":{"type":"text","text":"cancelled before I/O"}});
    let (input, cancel) = tokio::join!(server.mcp("computer_act", command, &session.connection().token), async {
        tokio::time::timeout(Duration::from_secs(5), ready).await??;
        server
            .request(Method::DELETE, &format!("/v1/jobs/{}", job.id), "default")
            .send()
            .await?
            .error_for_status()?;
        release.send(())?;
        Ok::<_, anyhow::Error>(())
    });
    cancel?;
    ensure!(input?["result"]["isError"] == true);
    server.wait(job.id, JobState::Cancelled).await?;
    ensure!(
        *driver.0.lock().unwrap() == ["capture"],
        "Cancellation allowed native input after acknowledgement"
    );
    server.stop().await
}

async fn submission_waits_for_permission() -> Result<()> {
    // Given a submitted computer task, when permission is pending, then the
    // real provider turn cannot race the operator's decision.
    let server = Server::open(Provider::Codex).await?;
    server.login(Provider::Codex, "default").await?;
    let job: Job = server
        .request(Method::POST, "/v1/providers/codex/jobs", "default")
        .json(&json!({"text":"Computer task", "computer_use":true}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let (request, sequence) = server.pending(job.id, 0).await?;
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = server
            .request(Method::GET, &format!("/v1/jobs/{}/events?after={}", job.id, sequence - 1), "default")
            .send()
            .await?
            .error_for_status()?;
        let mut bytes = Vec::new();
        while !bytes.contains(&b'\n') {
            bytes.extend_from_slice(&stream.chunk().await?.context("Missing approval event")?);
        }
        Ok::<JobEvent, anyhow::Error>(serde_json::from_slice(bytes.split(|b| *b == b'\n').next().unwrap())?)
    })
    .await??;
    ensure!(
        matches!(event.event, StreamEvent::Interaction {event: Event::Permission {method,..},..} if method=="computer/request_control"),
        "Computer task must wait before invoking the native provider"
    );
    server
        .request(Method::POST, &format!("/v1/jobs/{}/interactions/{request}", job.id), "default")
        .json(&json!({"type":"approval","allow":false}))
        .send()
        .await?
        .error_for_status()?;
    server.wait(job.id, JobState::Failed).await?;
    ensure!(serde_json::to_value(server.job(job.id).await?.request)?["computer_use"] == true);
    server.stop().await
}

async fn run_provider(provider: Provider) -> Result<()> {
    // Given a running conversation job with an authenticated execution, but no
    // operator grant, when it captures the desktop, then no screen is
    // disclosed.
    let driver = Arc::new(DesktopFixture::default());
    let controls = Arc::new(gpui_agent_control::Controls::default().with_desktop_driver(driver.clone()));
    let server = Server::open_controls(provider, "interactive-allow", controls).await?;
    server.login(provider, "default").await?;
    let job = server.enqueue(provider, "default", "Computer use", None).await?;
    let (approval, _) = server.pending(job.id, 0).await?;
    let job = server.job(job.id).await?.job;
    let scope = McpScope {
        provider: job.scope.provider,
        account_id: job.scope.account_id.clone(),
        workspace_path: job.scope.workspace_path.clone(),
    };
    let session = server.sessions.issue_for_job(scope.clone(), true, job.conversation_id, Some(job.id))?;
    // Given no computer grant, when the operator reads control state, then
    // the desktop is available and no task owns it. Agent tokens cannot read
    // it.
    let state: serde_json::Value = server
        .request(Method::GET, "/v1/control/computer", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(state["available"] == true && state["job_id"].is_null() && state["execution_bound"] == false && state["expires_in_seconds"] == 0);
    ensure!(
        server
            .client
            .get(format!("{}/v1/control/computer", server.url))
            .bearer_auth(&session.connection().token)
            .send()
            .await?
            .status()
            == StatusCode::UNAUTHORIZED
    );
    ensure!(
        server
            .client
            .post(format!("{}/v1/control/computer/grant", server.url))
            .bearer_auth(&session.connection().token)
            .json(&json!({"job_id":job.id,"enabled":true}))
            .send()
            .await?
            .status()
            == StatusCode::UNAUTHORIZED
    );
    let denied = server.mcp("computer_capture", json!({}), &session.connection().token).await?;
    ensure!(denied["result"]["isError"] == true, "Capture must be denied as a tool error: {denied}");
    ensure!(denied["result"]["content"][0]["text"].as_str().is_some_and(|text| text.contains("grant")));
    // Given an operator grant for this job, when an execution claims it, then
    // another execution in the very same conversation cannot observe its
    // screen.
    server
        .request(Method::POST, "/v1/control/computer/grant", "default")
        .json(&json!({"job_id":job.id,"enabled":true}))
        .send()
        .await?
        .error_for_status()?;
    let granted: serde_json::Value = server
        .request(Method::GET, "/v1/control/computer", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(granted["job_id"] == job.id && granted["execution_bound"] == false && granted["expires_in_seconds"].as_u64().is_some_and(|n| n > 0 && n <= 600));
    let captured = server.mcp("computer_capture", json!({}), &session.connection().token).await?;
    ensure!(captured["result"]["isError"] == false, "{captured}");
    ensure!(captured["result"]["content"][1]["type"] == "image");
    ensure!(captured["result"]["structuredContent"].get("image").is_none());
    let bound: serde_json::Value = server
        .request(Method::GET, "/v1/control/computer", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(bound["job_id"] == job.id && bound["execution_bound"] == true);
    let other = server.sessions.issue_for_job(scope.clone(), true, job.conversation_id, Some(job.id))?;
    ensure!(server.mcp("computer_capture", json!({}), &other.connection().token).await?["result"]["isError"] == true);
    ensure!(server.mcp("computer_close", json!({}), &other.connection().token).await?["result"]["isError"] == true);
    ensure!(
        !server
            .request(Method::POST, "/v1/control/desktop/grant", "default")
            .send()
            .await?
            .status()
            .is_success()
    );
    // When the owning execution inputs text against its observed revision,
    // then it receives a new capture and the old revision cannot replay input.
    let input = json!({"revision":captured["result"]["structuredContent"]["revision"],"action":{"type":"text","text":"MVP 한글"}});
    let acted = server.mcp("computer_act", input.clone(), &session.connection().token).await?;
    ensure!(acted["result"]["isError"] == false, "Input and re-observation failed: {acted}");
    ensure!(acted["result"]["content"][1]["type"] == "image");
    ensure!(acted["result"]["structuredContent"]["revision"] != input["revision"]);
    ensure!(server.mcp("computer_act", input, &session.connection().token).await?["result"]["isError"] == true);
    ensure!(*driver.0.lock().unwrap() == ["capture", "MVP 한글"]);
    let reader = server.sessions.issue_for_job(scope.clone(), false, job.conversation_id, Some(job.id))?;
    ensure!(
        server
            .mcp(
                "computer_act",
                json!({"revision":"1","action":{"type":"text","text":"denied"}}),
                &reader.connection().token
            )
            .await?["error"]["code"]
            == -32001
    );
    for args in [
        json!({"job_id":job.id}),
        json!({"execution_id":1}),
        json!({"scope":scope}),
        json!({"type":"capture"}),
    ] {
        ensure!(server.mcp("computer_capture", args, &session.connection().token).await?["result"]["isError"] == true);
    }
    for action in [
        json!({"type":"click","x":1,"y":0}),
        json!({"type":"open","url":"https://example.invalid"}),
        json!({"type":"key","key":"F24"}),
        json!({"type":"text","text":"bad\u{0000}input"}),
    ] {
        ensure!(
            server
                .mcp(
                    "computer_act",
                    json!({"revision":acted["result"]["structuredContent"]["revision"],"action":action}),
                    &session.connection().token
                )
                .await?["result"]["isError"]
                == true
        );
    }
    let foreign = server.sessions.issue_for_job(
        McpScope {
            account_id: "0123456789abcdef0123456789abcdef".into(),
            ..scope.clone()
        },
        true,
        job.conversation_id,
        Some(job.id),
    )?;
    ensure!(server.mcp("computer_capture", json!({}), &foreign.connection().token).await?["result"]["isError"] == true);
    ensure!(
        *driver.0.lock().unwrap() == ["capture", "MVP 한글"],
        "Denied commands must never reach native I/O"
    );
    // Given an observation failure, when the caller reuses its last successful
    // revision, then it must observe again before any further input.
    driver.1.store(true, std::sync::atomic::Ordering::SeqCst);
    ensure!(server.mcp("computer_capture", json!({}), &session.connection().token).await?["result"]["isError"] == true);
    ensure!(
        server
            .mcp(
                "computer_act",
                json!({"revision":acted["result"]["structuredContent"]["revision"],"action":{"type":"text","text":"must observe again"}}),
                &session.connection().token
            )
            .await?["result"]["isError"]
            == true
    );
    let before_revoke = server.mcp("computer_capture", json!({}), &session.connection().token).await?;
    ensure!(
        before_revoke["result"]["isError"] == false,
        "Revocation test requires a fresh successful observation"
    );
    let io_before_revoke = driver.0.lock().unwrap().clone();
    // Given a native request waiting for approval, when control is revoked,
    // then a delayed action with its valid observation cannot reach I/O.
    server
        .request(Method::POST, "/v1/control/computer/grant", "default")
        .json(&json!({"job_id":job.id,"enabled":false}))
        .send()
        .await?
        .error_for_status()?;
    let revoked: serde_json::Value = server
        .request(Method::GET, "/v1/control/computer", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(revoked["job_id"].is_null() && revoked["expires_in_seconds"] == 0 && revoked["execution_bound"] == false);
    let late = server
        .mcp(
            "computer_act",
            json!({"revision":before_revoke["result"]["structuredContent"]["revision"],"action":{"type":"text","text":"must not type after revoke"}}),
            &session.connection().token,
        )
        .await?;
    ensure!(late["result"]["isError"] == true);
    ensure!(*driver.0.lock().unwrap() == io_before_revoke, "Revoked input reached the native driver");
    server
        .request(Method::POST, "/v1/control/computer/grant", "default")
        .json(&json!({"job_id":job.id,"enabled":true}))
        .send()
        .await?
        .error_for_status()?;
    server.respond(job.id, approval).await?;
    server.wait(job.id, JobState::Completed).await?;
    // Given a finished job, when the same conversation starts another job,
    // then the previous execution cannot inherit the new job's grant.
    let next = server.enqueue(provider, "default", "Next computer job", job.conversation_id).await?;
    let (next_approval, _) = server.pending(next.id, 0).await?;
    server
        .request(Method::POST, "/v1/control/computer/grant", "default")
        .json(&json!({"job_id":next.id,"enabled":true}))
        .send()
        .await?
        .error_for_status()?;
    ensure!(
        server.mcp("computer_capture", json!({}), &session.connection().token).await?["result"]["isError"] == true,
        "An old execution must not inherit a later job grant"
    );
    let next_session = server.sessions.issue_for_job(scope, true, next.conversation_id, Some(next.id))?;
    let next_capture = server.mcp("computer_capture", json!({}), &next_session.connection().token).await?;
    ensure!(next_capture["result"]["isError"] == false);
    let io_before_cancel = driver.0.lock().unwrap().clone();
    server
        .request(Method::DELETE, &format!("/v1/jobs/{}", next.id), "default")
        .send()
        .await?
        .error_for_status()?;
    server.wait(next.id, JobState::Cancelled).await?;
    let cancelled: serde_json::Value = server
        .request(Method::GET, "/v1/control/computer", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(cancelled["job_id"].is_null() && cancelled["expires_in_seconds"] == 0 && cancelled["execution_bound"] == false);
    ensure!(server.mcp("computer_capture", json!({}), &next_session.connection().token).await?["result"]["isError"] == true);
    let late = server
        .mcp(
            "computer_act",
            json!({"revision":next_capture["result"]["structuredContent"]["revision"],"action":{"type":"text","text":"must not type after cancel"}}),
            &next_session.connection().token,
        )
        .await?;
    ensure!(late["result"]["isError"] == true);
    ensure!(
        !server
            .request(Method::POST, &format!("/v1/jobs/{}/interactions/{next_approval}", next.id), "default")
            .json(&json!({"type":"approval","allow":true}))
            .send()
            .await?
            .status()
            .is_success()
    );
    ensure!(*driver.0.lock().unwrap() == io_before_cancel, "Cancelled input reached the native driver");
    let diagnostics = serde_json::to_string(&*server.diagnostics.lock().unwrap())?;
    ensure!(!diagnostics.contains("MVP 한글") && !diagnostics.contains(&session.connection().token) && !diagnostics.contains("iVBOR"));
    server.stop().await?;
    Ok(())
}
