use super::*;
use gpui_agent_application::ports::{AgentRuntime,
                                    RuntimeFuture};

struct LiveRuntime(gpui_agent::Agent);
impl AgentRuntime for LiveRuntime {
    fn status(&self) -> RuntimeFuture<'_, AuthStatus> { Box::pin(self.0.status()) }

    fn models(&self) -> RuntimeFuture<'_, Vec<Model>> { Box::pin(self.0.models()) }

    fn login<'a>(&'a self, _: bool, _: &'a mut gpui_agent::Handler<'_>) -> RuntimeFuture<'a, AuthStatus> {
        Box::pin(async { anyhow::bail!("Live acceptance does not perform login") })
    }

    fn logout(&self) -> RuntimeFuture<'_, ()> { Box::pin(async { anyhow::bail!("Live acceptance does not perform logout") }) }

    fn prompt<'a>(&'a self, prompt: &'a gpui_agent::Prompt, handler: &'a mut gpui_agent::Handler<'_>) -> RuntimeFuture<'a, gpui_agent::Completion> {
        Box::pin(self.0.prompt(prompt, handler))
    }
}

// Explicitly opt in to the real account; never copy credentials into a fixture.
fn application(home: &Path, cwd: &Path, sessions: &Arc<gpui_agent_infrastructure::McpSessions>) -> Result<Application> {
    let profile =
        std::path::PathBuf::from(std::env::var_os("GPUI_AGENT_QA_CODEX_HOME").context("Set GPUI_AGENT_QA_CODEX_HOME to an authenticated Codex profile")?);
    let executable = std::path::PathBuf::from(
        std::env::var_os("GPUI_AGENT_QA_CODEX_EXECUTABLE").context("Set GPUI_AGENT_QA_CODEX_EXECUTABLE to the installed native Codex executable")?,
    );
    ensure!(
        profile.is_absolute() && profile.is_dir() && executable.is_absolute() && executable.is_file(),
        "Live profile and executable must be existing absolute paths"
    );
    Ok(Application::new(
        PROVIDERS.map(|provider| {
            ensure!(provider == Provider::Codex, "Live acceptance only enables Codex");
            Ok(Arc::new(LiveRuntime(
                gpui_agent::Agent::new(gpui_agent::Config {
                    provider,
                    home: profile.clone(),
                    cwd: cwd.into(),
                    executable: executable.clone(),
                    harness: None,
                    browser_helper: Some(std::env::current_exe()?),
                    timeout: Duration::from_secs(600),
                    mcp_home: None,
                    mcp_selection: None,
                    plugin_mcp: vec![],
                })?
                .with_mcp_sessions(sessions.for_scope(
                    McpScope {
                        provider,
                        account_id: "default".into(),
                        workspace_path: cwd.to_string_lossy().into_owned(),
                    },
                    true,
                )),
            )) as Arc<dyn AgentRuntime>)
        }),
        Arc::new(gpui_agent_infrastructure::History::open(&home.join("history.sqlite3"), cwd)?),
    ))
}

pub async fn run() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cwd = root.path().join("computer-acceptance");
    fs::create_dir(&cwd)?;
    let settings = Settings::discover(cwd.clone(), Some(root.path().join("home")))?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let sessions = Arc::new(gpui_agent_infrastructure::McpSessions::new(format!("{url}/mcp"))?);
    let app = application(&settings.home, &cwd, &sessions)?;
    ensure!(app.status(Provider::Codex).await?.authenticated, "Live Codex profile is signed out");
    let factory_home = settings.home.clone();
    let factory_sessions = sessions.clone();
    let workspaces = Projects::open(settings.clone(), app.clone())?.service();
    let jobs = JobStore::open(settings.clone(), workspaces.clone())?
        .with_application_factory(move |scope| {
            ensure!(
                scope.provider == Provider::Codex && scope.account_id == "default",
                "Live acceptance only enables the default Codex account"
            );
            application(&factory_home, Path::new(&scope.workspace_path), &factory_sessions)
        })
        .service(workspaces.clone());
    let token_file = settings.home.join("api.token");
    let token = gpui_agent_infrastructure::load_or_create_token(&token_file)?;
    let api = gpui_agent_api::Api::new(app, token)?
        .with_mcp_sessions(sessions)
        .with_workspaces(workspaces)
        .with_jobs(jobs.clone())
        .with_controls(Arc::new(gpui_agent_control::Controls::default()));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = rx.await;
    }));
    println!("Live Codex client: {url}");
    println!("Bearer token file: {}", token_file.display());
    println!("Real account and Windows controls. Temporary job state; Enter cancels jobs and stops the host.");
    tokio::task::spawn_blocking(|| std::io::stdin().read_line(&mut String::new())).await??;
    for job in jobs.list().await? {
        if matches!(job.state, JobState::Queued | JobState::Running | JobState::Waiting) {
            jobs.cancel(job.id).await?;
        }
    }
    let _ = tx.send(());
    task.await??;
    Ok(())
}

// A real model and existing profile, with synthetic pixels and input only.
pub async fn automatic() -> Result<()> {
    let provider = match std::env::var("GPUI_AGENT_QA_PROVIDER")?.as_str() {
        | "claude" => Provider::Claude,
        | "codex" => Provider::Codex,
        | "antigravity" => Provider::Antigravity,
        | _ => anyhow::bail!("Select claude, codex or antigravity"),
    };
    let profile = std::path::PathBuf::from(std::env::var_os("GPUI_AGENT_QA_PROFILE").context("Missing existing QA profile")?);
    let executable = std::path::PathBuf::from(std::env::var_os("GPUI_AGENT_QA_EXECUTABLE").context("Missing native executable")?);
    ensure!(
        profile.is_absolute() && profile.is_dir() && executable.is_absolute() && executable.is_file(),
        "Existing absolute profile/executable required"
    );
    let root = tempfile::tempdir()?;
    let cwd = root.path().join("synthetic-desktop");
    fs::create_dir(&cwd)?;
    let settings = Settings::discover(cwd.clone(), Some(root.path().join("home")))?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let sessions = Arc::new(gpui_agent_infrastructure::McpSessions::new(format!("{url}/mcp"))?);
    let factory_home = settings.home.clone();
    let authorizer = sessions.clone();
    let harness = (provider == Provider::Antigravity).then(|| {
        executable.with_file_name(if cfg!(windows) {
            "localharness_external.exe"
        } else {
            "localharness_external"
        })
    });
    ensure!(harness.as_ref().is_none_or(|path| path.is_file()), "Antigravity native harness is missing");
    let make_application = move |cwd: &Path| -> Result<Application> {
        let agent = Arc::new(LiveRuntime(
            gpui_agent::Agent::new(gpui_agent::Config {
                provider,
                home: profile.clone(),
                cwd: cwd.into(),
                executable: executable.clone(),
                harness: harness.clone(),
                browser_helper: Some(std::env::current_exe()?),
                timeout: Duration::from_secs(180),
                mcp_home: None,
                mcp_selection: None,
                plugin_mcp: vec![],
            })?
            .with_mcp_sessions(sessions.for_scope(
                McpScope {
                    provider,
                    account_id: "default".into(),
                    workspace_path: cwd.to_string_lossy().into_owned(),
                },
                false,
            )),
        ));
        Ok(Application::new(
            PROVIDERS.map(|candidate| {
                ensure!(candidate == provider, "Only the selected live provider is enabled");
                Ok(agent.clone() as Arc<dyn AgentRuntime>)
            }),
            Arc::new(gpui_agent_infrastructure::History::open(&factory_home.join("history.sqlite3"), cwd)?),
        )
        .with_automatic_computer(true))
    };
    let app = make_application(&cwd)?;
    ensure!(app.status(provider).await?.authenticated, "Selected profile is signed out");
    let workspaces = Projects::open(settings.clone(), app.clone())?.service();
    let jobs = JobStore::open(settings, workspaces.clone())?
        .with_application_factory(move |scope| make_application(Path::new(&scope.workspace_path)))
        .service(workspaces.clone());
    let driver = Arc::new(super::computer::DesktopFixture::default());
    let controls = Arc::new(
        gpui_agent_control::Controls::default()
            .with_desktop_driver(driver.clone())
            .with_automatic_desktop(),
    );
    #[derive(Default)]
    struct ToolReceipts(std::sync::Mutex<Vec<(String, bool)>>);
    impl gpui_agent_application::ports::McpDiagnosticSink for ToolReceipts {
        fn record(&self, diagnostic: &McpDiagnostic) { self.0.lock().unwrap().push((diagnostic.tool.clone(), diagnostic.ok)); }
    }
    let receipts = Arc::new(ToolReceipts::default());
    // The authorizer shares the issuer held by the runtime applications.
    let api = gpui_agent_api::Api::new(app, gpui_agent_infrastructure::new_token()?)?
        .with_mcp_sessions(authorizer)
        .with_mcp_diagnostics(receipts.clone())
        .with_workspaces(workspaces)
        .with_jobs(jobs.clone())
        .with_controls(controls);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let result = async {
        let request: ChatRequest = serde_json::from_value(json!({"text":"This is a synthetic 1x1 desktop acceptance test. Use only gpui-agent-session computer tools. Call computer_capture, then computer_act click at (0,0), then computer_act text AUTOMATIC_QA, then computer_close, using each returned revision. Do not use shell or filesystem tools. Finish after these calls.","computer_use":true,"model":if provider==Provider::Claude {Some("haiku")} else {None}}))?;
        let job = jobs.enqueue(provider, "default".into(), None, request).await?;
        tokio::time::timeout(Duration::from_secs(200), async {
            loop {
                let current = jobs.job(job.id).await?;
                if current.state == JobState::Waiting {
                    for event in jobs.events(job.id, 0).await? {
                        if let StreamEvent::Interaction { event, .. } = event.event {
                            match event {
                                Event::Elicitation { server_name, message, schema, url } => eprintln!("QA approval metadata: host={} computer_name={} empty_form={} url={}", server_name == "gpui-agent-session", message.contains("computer_act"), schema.as_ref().and_then(|schema| schema["properties"].as_object()).is_some_and(|fields| fields.is_empty()), url.is_some()),
                                Event::Permission { method, .. } => eprintln!("QA approval method: {}", method),
                                _ => eprintln!("QA approval: other interaction"),
                            }
                        }
                    }
                    anyhow::bail!("Native interaction requires review");
                }
                if matches!(current.state, JobState::Completed | JobState::Failed | JobState::Cancelled) {
                    ensure!(current.state == JobState::Completed, "Live job did not complete");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok::<_, anyhow::Error>(())
        }).await??;
        let calls = driver.0.lock().unwrap();
        ensure!(receipts.0.lock().unwrap().iter().any(|(tool, ok)| tool == "computer_close" && *ok), "Missing successful computer_close receipt");
        ensure!(calls.iter().any(|call| call == "capture") && calls.iter().any(|call| call == "click") && calls.iter().any(|call| call == "AUTOMATIC_QA"), "Model did not perform the required synthetic computer actions");
        println!("ok: {} real model, synthetic capture/click/text, no operator approvals", provider_name(provider));
        Ok::<_, anyhow::Error>(())
    }.await;
    let _ = stop.send(());
    task.await??;
    result
}
