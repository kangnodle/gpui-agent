use anyhow::Result;
use gpui_agent_application::{Application,
                             ports::{AgentRuntime,
                                     RuntimeFuture}};
use gpui_agent_contracts::*;
use gpui_agent_infrastructure::Settings;
use std::sync::{Arc,
                atomic::{AtomicUsize,
                         Ordering}};

#[tokio::test]
async fn given_operator_required_checks_when_the_project_store_reopens_then_the_plan_is_retained_and_scoped_agents_cannot_disable_it() -> Result<()> {
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let project = workspaces.catalog().await?.projects[0].id;
    let plan: ProjectVerification = serde_json::from_value(serde_json::json!({"commands":[["bun","test","server"],["vp","check"]]}))?;
    workspaces.set_project_verification(None, project, plan.clone()).await?;
    let caller = McpScope {
        provider: Provider::Claude,
        account_id: "default".into(),
        workspace_path: root.path().display().to_string(),
    };
    assert!(
        workspaces
            .set_project_verification(Some(&caller), project, ProjectVerification::default())
            .await
            .is_err()
    );
    drop(workspaces);
    let workspaces = settings.workspaces(settings.application()?)?;
    assert_eq!(workspaces.catalog().await?.projects[0].verification, plan);
    Ok(())
}

struct RepairAgent {
    prompts: std::sync::Mutex<Vec<(String, Option<PreparedGuidance>)>>,
    code: std::path::PathBuf,
    turns: AtomicUsize,
    repair: bool,
}
impl AgentRuntime for RepairAgent {
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

    fn models(&self) -> RuntimeFuture<'_, Vec<Model>> { Box::pin(async { Ok(vec![]) }) }

    fn prompt<'a>(&'a self, prompt: &'a Prompt, handler: &'a mut Handler<'_>) -> RuntimeFuture<'a, Completion> {
        Box::pin(async move {
            self.turns.fetch_add(1, Ordering::SeqCst);
            self.prompts.lock().unwrap().push((prompt.runtime_text(), prompt.guidance.clone()));
            if prompt.session_id.is_some() {
                assert!(prompt.text.contains("Host verification"));
                if self.repair {
                    std::fs::write(&self.code, "fixed")?;
                }
            }
            handler.emit(Event::Text {
                delta: "Implemented".into(),
            });
            Ok(Completion {
                session_id: "fixture-session".into(),
                text: "Implemented".into(),
            })
        })
    }
}

#[test]
fn verification_process_receives_latest_code() {
    if let Ok(code) = std::fs::read_to_string("verification-code.txt") {
        assert_eq!(code, "fixed");
    }
}

async fn start_verified_job(plan: ProjectVerification, repair: bool) -> Result<(tempfile::TempDir, gpui_agent_application::Jobs, Job, Arc<RepairAgent>)> {
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join(".gitignore"), "/home\n")?;
    std::fs::write(root.path().join("verification-code.txt"), "broken")?;
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()?
            .success()
    );
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let runtime = Arc::new(RepairAgent {
        prompts: Default::default(),
        code: root.path().join("verification-code.txt"),
        turns: AtomicUsize::new(0),
        repair,
    });
    let history = Arc::new(gpui_agent_infrastructure::History::open(&settings.home.join("history.sqlite3"), root.path())?);
    let app = Application::new(PROVIDERS.map(|_| Ok(runtime.clone() as Arc<dyn AgentRuntime>)), history);
    let workspaces = gpui_agent_infrastructure::Projects::open(settings.clone(), app.clone())?.service();
    let project = workspaces.catalog().await?.projects[0].id;
    workspaces.set_project_verification(None, project, plan).await?;
    let jobs = gpui_agent_infrastructure::JobStore::open(settings, workspaces.clone())?
        .with_application_factory(move |_| Ok(app.clone()))
        .service(workspaces);
    let job = jobs
        .enqueue(
            Provider::Claude,
            "default".into(),
            None,
            serde_json::from_value(serde_json::json!({"text":"Fix the implementation"}))?,
        )
        .await?;
    Ok((root, jobs, job, runtime))
}

async fn wait_terminal(jobs: &gpui_agent_application::Jobs, id: i64) -> Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if jobs.job(id).await?.state.terminal() {
                break Ok::<_, anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn given_a_normal_chat_with_required_commands_when_a_check_fails_then_the_host_repairs_and_verifies_before_completing() -> Result<()> {
    let executable = std::env::current_exe()?.display().to_string();
    let plan = ProjectVerification {
        commands: vec![
            vec![
                executable.clone(),
                "--exact".into(),
                "verification_process_receives_latest_code".into(),
                "--nocapture".into(),
            ],
            vec![executable, "--list".into()],
        ],
        ..Default::default()
    };
    let (_root, jobs, job, runtime) = start_verified_job(plan, true).await?;
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if jobs.job(job.id).await?.state.terminal() {
                break Ok::<_, anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await??;
    assert_eq!(jobs.job(job.id).await?.state, JobState::Completed);
    assert_eq!(runtime.turns.load(Ordering::SeqCst), 2);
    let events = jobs.events(job.id, 0).await?;
    let reports: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if let StreamEvent::Agent(Event::Verification {
                report,
            }) = &event.event
            {
                Some(report)
            } else {
                None
            }
        })
        .collect();
    let first = reports.iter().find(|report| report.attempt == 1 && report.finished_revision.is_some()).unwrap();
    let last = reports.last().unwrap();
    assert!(!first.passed());
    assert!(last.passed());
    assert_ne!(first.revision.fingerprint, last.revision.fingerprint);
    assert_eq!(last.checks.len(), 2);
    assert_eq!(events.iter().filter(|event| matches!(event.event, StreamEvent::Completed { .. })).count(), 1);
    let detail = jobs.get(job.id).await?;
    let saved = jobs
        .account_scope(Provider::Claude, "default".into(), None)
        .await?
        .application
        .conversation(Provider::Claude, detail.job.conversation_id.unwrap())
        .await?;
    assert!(
        saved
            .timeline
            .iter()
            .any(|entry| matches!(&entry.event,Event::Verification{report} if report.passed()))
    );
    assert_eq!(
        saved.messages.iter().filter(|message| message.user).count(),
        1,
        "Host repair instructions must not appear as user messages"
    );
    jobs.cancel_all();
    Ok(())
}

#[tokio::test]
async fn given_failing_required_checks_when_retries_are_exhausted_then_the_job_fails_without_a_completed_event() -> Result<()> {
    let executable = std::env::current_exe()?.display().to_string();
    let plan = ProjectVerification {
        commands: vec![vec![executable, "--exact".into(), "verification_process_receives_latest_code".into()]],
        max_attempts: 2,
        ..Default::default()
    };
    let (_root, jobs, job, runtime) = start_verified_job(plan, false).await?;
    wait_terminal(&jobs, job.id).await?;
    assert_eq!(jobs.job(job.id).await?.state, JobState::Failed);
    assert_eq!(runtime.turns.load(Ordering::SeqCst), 2);
    assert!(
        !jobs
            .events(job.id, 0)
            .await?
            .iter()
            .any(|event| matches!(event.event, StreamEvent::Completed { .. }))
    );
    jobs.cancel_all();
    Ok(())
}
#[test]
fn verification_process_changes_code() {
    if std::path::Path::new("verification-code.txt").exists() {
        std::fs::write("verification-code.txt", "changed by check").unwrap();
    }
}
#[tokio::test]
async fn given_a_successful_command_that_changes_code_when_verification_finishes_then_its_stale_result_cannot_complete_the_job() -> Result<()> {
    let plan = ProjectVerification {
        commands: vec![vec![
            std::env::current_exe()?.display().to_string(),
            "--exact".into(),
            "verification_process_changes_code".into(),
        ]],
        max_attempts: 1,
        ..Default::default()
    };
    let (_root, jobs, job, _) = start_verified_job(plan, false).await?;
    wait_terminal(&jobs, job.id).await?;
    assert_eq!(jobs.job(job.id).await?.state, JobState::Failed);
    let events = jobs.events(job.id, 0).await?;
    let report = events
        .iter()
        .rev()
        .find_map(|e| {
            if let StreamEvent::Agent(Event::Verification {
                report,
            }) = &e.event
            {
                Some(report)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(report.checks[0].exit_code, Some(0));
    assert_ne!(report.finished_revision.as_ref(), Some(&report.revision));
    assert!(!report.passed());
    jobs.cancel_all();
    Ok(())
}
#[test]
fn verification_process_waits() {
    if std::path::Path::new("verification-code.txt").exists() {
        std::fs::write("home/check-running", std::process::id().to_string()).unwrap();
        std::thread::park_timeout(std::time::Duration::from_secs(60));
    }
}
#[tokio::test]
async fn given_a_running_host_check_when_cancelled_then_the_owned_process_is_stopped_and_the_job_never_completes() -> Result<()> {
    let plan = ProjectVerification {
        commands: vec![vec![
            std::env::current_exe()?.display().to_string(),
            "--exact".into(),
            "verification_process_waits".into(),
        ]],
        ..Default::default()
    };
    let (root, jobs, job, runtime) = start_verified_job(plan, false).await?;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !root.path().join("home/check-running").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    assert_eq!(jobs.job(job.id).await?.state, JobState::Running);
    jobs.cancel(job.id).await?;
    wait_terminal(&jobs, job.id).await?;
    assert_eq!(jobs.job(job.id).await?.state, JobState::Cancelled);
    assert_eq!(runtime.turns.load(Ordering::SeqCst), 1);
    assert!(!jobs.events(job.id, 0).await?.iter().any(|e| matches!(e.event, StreamEvent::Completed { .. })));
    jobs.cancel_all();
    Ok(())
}
#[tokio::test]
async fn given_a_command_timeout_when_no_retry_budget_remains_then_the_failure_and_no_exit_code_are_recorded() -> Result<()> {
    let plan = ProjectVerification {
        commands: vec![vec![
            std::env::current_exe()?.display().to_string(),
            "--exact".into(),
            "verification_process_waits".into(),
        ]],
        max_attempts: 1,
        timeout_seconds: 1,
    };
    let (_root, jobs, job, _) = start_verified_job(plan, false).await?;
    wait_terminal(&jobs, job.id).await?;
    assert_eq!(jobs.job(job.id).await?.state, JobState::Failed);
    let events = jobs.events(job.id, 0).await?;
    assert!(events.iter().any(|e|matches!(&e.event,StreamEvent::Agent(Event::Verification{report}) if report.checks.iter().any(|check|check.exit_code.is_none() && check.output.contains("timed out")))));
    jobs.cancel_all();
    Ok(())
}
