use anyhow::{Result,
             ensure};
use gpui_agent_application::ports::{ConversationRepository,
                                    JobRepository};
use gpui_agent_contracts::*;
use gpui_agent_infrastructure::{JobStore,
                                McpSessions,
                                Projects,
                                Settings};
use serde_json::{Value,
                 json};
use std::{future::Future,
          path::Path,
          sync::Arc,
          time::Duration};
#[allow(dead_code)]
#[path = "../../agent/tests/protocols.rs"]
mod native;

const TOKEN: &str = "pullRequestFixtureOperator1234567890123456789";
fn gh(args: &[String]) -> Result<()> {
    let root = std::env::current_exe()?.parent().unwrap().to_path_buf();
    ensure!(std::env::var("GH_HOST")? == "github.com");
    let mode = std::fs::read_to_string(root.join("gh-mode")).unwrap_or_default();
    if mode == "rate" {
        eprintln!("HTTP 429 rate limit");
        std::process::exit(1);
    }
    if mode == "failed" {
        eprintln!("Read failed");
        std::process::exit(1);
    }
    match args.get(1).map(String::as_str) {
        | Some("view") => {
            ensure!(args.get(2).map(String::as_str) == Some("7"));
            ensure!(args.windows(2).any(|v| v == ["--repo", "github.com/owner/repo"]));
            print!("{}", std::fs::read_to_string(root.join("snapshot.json"))?);
        },
        | Some("diff") => print!("diff --git a/test.rs b/test.rs\n--- a/test.rs\n+++ b/test.rs\n@@ -1 +1 @@\n-old\n+new\n"),
        | Some("user") => println!("fixture-user"),
        | _ => anyhow::bail!("Unexpected remote command: {args:?}"),
    }
    Ok(())
}
fn git(cwd: &Path, args: &[&str]) -> Result<()> {
    ensure!(std::process::Command::new("git").args(["-C"]).arg(cwd).args(args).status()?.success());
    Ok(())
}
fn snapshot() -> Value {
    json!({"number":7,"url":"https://github.com/owner/repo/pull/7","title":"Fix behavior","body":"Description",
        "headRefOid":"1111111111111111111111111111111111111111","baseRefOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","headRefName":"fix","baseRefName":"main",
        "state":"OPEN","isDraft":false,"mergeable":"MERGEABLE","comments":[],"reviews":[{"id":"pending-review","state":"PENDING","submittedAt":null,"body":"","author":{"login":"fixture-user"}}],
        "statusCheckRollup":[{"__typename":"CheckRun","name":"tests","context":"","state":"","status":"IN_PROGRESS","conclusion":""}]})
}

async fn scenario(root: &Path) -> Result<()> {
    // Given a real Git/SQLite host, a saved conversation and an external gh
    // fixture.
    let cwd = root.join("repo");
    std::fs::create_dir(&cwd)?;
    std::fs::write(cwd.join("fixture.json"), json!({"provider":"auto","scenario":"interactive-deny"}).to_string())?;
    git(&cwd, &["init", "-b", "main"])?;
    git(&cwd, &["remote", "add", "origin", "https://github.com/owner/repo.git"])?;
    let settings = Settings::discover(cwd.clone(), Some(root.join("home")))?;
    for provider in PROVIDERS {
        let profile = settings.home.join(provider_name(provider));
        let token = if provider == Provider::Antigravity {
            profile.join("antigravity-acp/acp_token.json")
        } else {
            profile.join("fixture-auth")
        };
        std::fs::create_dir_all(token.parent().unwrap())?;
        std::fs::write(token, "fixture")?;
    }
    let app = gpui_agent_infrastructure::application_from_agents(
        PROVIDERS.map(|provider| {
            gpui_agent::Agent::new(gpui_agent::Config {
                provider,
                home: settings.home.join(provider_name(provider)),
                cwd: cwd.clone(),
                executable: std::env::current_exe().unwrap(),
                harness: Some(std::env::current_exe().unwrap()),
                browser_helper: Some(std::env::current_exe().unwrap()),
                timeout: Duration::from_secs(20),
                mcp_home: None,
                mcp_selection: None,
                plugin_mcp: vec![],
            })
        }),
        &settings.home.join("history.sqlite3"),
        &cwd,
    )?;
    let history = gpui_agent_infrastructure::History::open(&settings.home.join("history.sqlite3"), &cwd)?;
    let mut prompt = Prompt {
        mcp_servers: None,
        skills: vec![],
        guidance: None,
        text: "Review".into(),
        attachments: vec![],
        model: None,
        session_id: None,
        claude_permission_mode: None,
        codex_execution_policy: None,
    };
    let turn = history.begin(Provider::Codex, None, &mut prompt)?;
    history.finish(
        turn,
        Some(&Completion {
            text: "Ready".into(),
            session_id: "codex-session".into(),
        }),
    )?;
    let workspaces = Projects::open(settings.clone(), app.clone())?.service();
    let store = Arc::new(JobStore::open(settings, workspaces.clone())?);
    let jobs = gpui_agent_application::Jobs::new(store.clone(), workspaces.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/mcp", listener.local_addr()?);
    let sessions = Arc::new(McpSessions::new(url.clone())?);
    let catalog = workspaces.catalog().await?;
    let scope = McpScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_path: catalog.projects[0].workspaces[0].path.clone(),
    };
    let writer = sessions.issue(scope.clone(), true)?;
    let api = gpui_agent_api::Api::new(app, TOKEN.into())?
        .with_workspaces(workspaces.clone())
        .with_jobs(jobs.clone())
        .with_mcp_sessions(sessions);
    let (stop, done) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = done.await;
    }));
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(20)).build()?;
    let invoke = |token: &str, name: &str, args: Value| {
        let request = client
            .post(&url)
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}));
        async move { Ok::<Value, anyhow::Error>(request.send().await?.error_for_status()?.json().await?) }
    };
    let call = |name: &'static str| {
        let request = client.post(&url).bearer_auth(&writer.connection().token).header("Accept","application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{"provider":"codex","id":turn.conversation_id,"repository":"owner/repo","number":7}}}));
        async move { Ok::<Value, anyhow::Error>(request.send().await?.error_for_status()?.json().await?) }
    };
    // When a watch starts, then the current snapshot is a baseline, not an
    // alert.
    std::fs::write(root.join("snapshot.json"), snapshot().to_string())?;
    let started = call("watch_pull_request").await?;
    ensure!(started["result"]["isError"] == false, "{started}");
    ensure!(started["result"]["structuredContent"]["watching"] == true, "{started}");
    ensure!(store.list()?.is_empty());
    ensure!(call("watch_pull_request").await?["result"]["structuredContent"]["was_watching"] == true);
    let job_scope = JobScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_id: Some(catalog.projects[0].workspaces[0].id),
        workspace_path: scope.workspace_path,
    };
    // Given an existing read lease and a queued writer, PR reads must finish
    // using that lease rather than requesting another fair-lock read.
    let lease = workspaces.scope(job_scope.workspace_id).await?;
    let mut update = Box::pin(workspaces.environment_preferences_update(
        None,
        EnvironmentPreferencesPatch {
            label: Some("After PR read".into()),
            default_worktree_base: None,
        },
    ));
    ensure!(update.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop())).is_pending());
    let read = tokio::time::timeout(
        Duration::from_secs(5),
        workspaces.pull_request(
            &job_scope,
            &lease,
            PullRequestTarget {
                repository: "owner/repo".into(),
                number: 7,
            },
            PullRequestReadKind::Metadata,
        ),
    )
    .await;
    ensure!(read.is_ok(), "A queued writer must not deadlock an existing PR read");
    read??;
    drop(lease);
    update.await?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
    jobs.poll_pull_requests_at(now + 121).await?;
    ensure!(store.list()?.is_empty(), "Unchanged PRs must stay quiet");
    let mut changed = snapshot();
    changed["statusCheckRollup"][0]["status"] = json!("COMPLETED");
    changed["statusCheckRollup"][0]["conclusion"] = json!("FAILURE");
    changed["comments"] = json!([
        {"id":"own","createdAt":"2026-10-24T12:00:00Z","author":{"login":"fixture-user"},"body":"own message"},
        {"id":"other","createdAt":"2026-10-24T12:00:00Z","author":{"login":"reviewer"},"body":"Please check cancellation"}
    ]);
    std::fs::write(root.join("snapshot.json"), changed.to_string())?;
    // When checks fail and someone comments, then one durable turn contains the
    // news.
    jobs.poll_pull_requests_at(now + 242).await?;
    let alerts = store.list()?;
    ensure!(alerts.len() == 1, "{alerts:?}");
    let alert = store.get(alerts[0].id)?;
    ensure!(alert.request.conversation_id == Some(turn.conversation_id));
    ensure!(alert.request.text.contains("checks failed") && alert.request.text.contains("Please check cancellation"));
    ensure!(!alert.request.text.contains("own message"));
    jobs.poll_pull_requests_at(now + 363).await?;
    ensure!(store.list()?.len() == 1, "The same news must not wake twice");
    finish(&jobs, alerts[0].id).await?;
    changed["state"] = json!("MERGED");
    std::fs::write(root.join("snapshot.json"), changed.to_string())?;
    jobs.poll_pull_requests_at(now + 484).await?;
    for alert in store.list()? {
        finish(&jobs, alert.id).await?;
    }
    let links = store.pull_request_links(&job_scope, turn.conversation_id)?;
    ensure!(links.len() == 1 && links[0].watch.is_none());
    ensure!(links[0].snapshot.as_ref().is_some_and(|s| s.state == PullRequestState::Merged));
    ensure!(call("watch_pull_request").await?["result"]["isError"] == true, "Closed PRs must not be watched");
    std::fs::write(root.join("snapshot.json"), snapshot().to_string())?;
    ensure!(call("watch_pull_request").await?["result"]["isError"] == false);
    ensure!(call("unwatch_pull_request").await?["result"]["structuredContent"]["was_watching"] == true);
    ensure!(call("unwatch_pull_request").await?["result"]["structuredContent"]["was_watching"] == false);
    let guides: Value = client.post(&url).bearer_auth(&writer.connection().token).header("Accept","application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"write_pull_request_guides","arguments":{"provider":"codex","pull_requests":[{"repository":"owner/repo","number":7}]}}}))
        .send().await?.error_for_status()?.json().await?;
    ensure!(guides["result"]["isError"] == false, "{guides}");
    ensure!(guides["result"]["structuredContent"]["guides"][0]["outcome"] == "started", "{guides}");
    let guide_id = guides["result"]["structuredContent"]["guides"][0]["run"]["job"]["id"].as_i64().unwrap();
    let chat = store.get(guide_id)?.request;
    ensure!(chat.session_id.is_none() && chat.conversation_id.is_none() && chat.claude_permission_mode.is_none());
    ensure!(chat.attachments.iter().any(|a| a.text_context().is_some_and(|t| t.contains("+new"))));
    finish(&jobs, guide_id).await?;
    for provider in PROVIDERS {
        let target = json!({"provider":provider,"repository":"owner/repo","number":7});
        if provider != Provider::Codex {
            let reply = invoke(
                TOKEN,
                "write_pull_request_guides",
                json!({"provider":provider,"pull_requests":[{"repository":"owner/repo","number":7}]}),
            )
            .await?;
            let id = reply["result"]["structuredContent"]["guides"][0]["run"]["job"]["id"].as_i64().unwrap();
            finish(&jobs, id).await?;
        }
        let read = invoke(TOKEN, "read_pull_request_guide", target.clone()).await?;
        ensure!(read["result"]["structuredContent"]["run"]["text"] == "Hello 세계", "{read}");
        ensure!(read["result"]["structuredContent"]["stale"] == false, "{read}");
        let retry = invoke(
            TOKEN,
            "write_pull_request_guides",
            json!({"provider":provider,"pull_requests":[{"repository":"owner/repo","number":7}]}),
        )
        .await?;
        ensure!(retry["result"]["structuredContent"]["guides"][0]["outcome"] == "current", "{retry}");
    }
    let mut base_changed = snapshot();
    base_changed["baseRefOid"] = json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    std::fs::write(root.join("snapshot.json"), base_changed.to_string())?;
    let base_read = invoke(
        TOKEN,
        "read_pull_request_guide",
        json!({"provider":"codex","repository":"owner/repo","number":7}),
    )
    .await?;
    ensure!(
        base_read["result"]["structuredContent"]["stale"] == true,
        "A changed base also changes the reviewed diff: {base_read}"
    );
    let mut moved = snapshot();
    moved["headRefOid"] = json!("2222222222222222222222222222222222222222");
    std::fs::write(root.join("snapshot.json"), moved.to_string())?;
    let read = invoke(
        TOKEN,
        "read_pull_request_guide",
        json!({"provider":"codex","repository":"owner/repo","number":7}),
    )
    .await?;
    ensure!(read["result"]["structuredContent"]["stale"] == true);
    std::fs::write(root.join("gh-mode"), "rate")?;
    let offline = invoke(
        TOKEN,
        "read_pull_request_guide",
        json!({"provider":"codex","repository":"owner/repo","number":7}),
    )
    .await?;
    ensure!(offline["result"]["structuredContent"]["stale"].is_null() && offline["result"]["structuredContent"]["read_error"].is_string());
    ensure!(offline["result"]["structuredContent"]["run"]["text"] == "Hello 세계");
    std::fs::remove_file(root.join("gh-mode"))?;
    ensure!(call("watch_pull_request").await?["result"]["isError"] == false);
    // Archiving immediately disarms the watch, including an already-read poll.
    let archived = invoke(
        &writer.connection().token,
        "conversation_organize",
        json!({"provider":"codex","id":turn.conversation_id,"archived":true}),
    )
    .await?;
    ensure!(archived["result"]["isError"] == false);
    ensure!(store.pull_request_links(&job_scope, turn.conversation_id)?[0].watch.is_none());
    let state_url = format!(
        "{}/v1/providers/codex/conversations/{}/state",
        url.trim_end_matches("/mcp"),
        turn.conversation_id
    );
    client
        .patch(&state_url)
        .bearer_auth(TOKEN)
        .json(&json!({"archived":false}))
        .send()
        .await?
        .error_for_status()?;
    let mut empty = snapshot();
    empty["statusCheckRollup"] = Value::Null;
    std::fs::write(root.join("snapshot.json"), empty.to_string())?;
    let empty_watch = call("watch_pull_request").await?;
    ensure!(
        empty_watch["result"]["isError"] == false,
        "A PR without checks is still watchable: {empty_watch}"
    );
    let interrupted = invoke(
        &writer.connection().token,
        "conversation_interrupt",
        json!({"provider":"codex","id":turn.conversation_id}),
    )
    .await?;
    ensure!(interrupted["result"]["isError"] == false);
    ensure!(store.pull_request_links(&job_scope, turn.conversation_id)?[0].watch.is_none());
    ensure!(call("watch_pull_request").await?["result"]["isError"] == false);
    client
        .patch(&state_url)
        .bearer_auth(TOKEN)
        .json(&json!({"archived":true}))
        .send()
        .await?
        .error_for_status()?;
    ensure!(store.pull_request_links(&job_scope, turn.conversation_id)?[0].watch.is_none());
    let _ = stop.send(());
    task.await??;
    Ok(())
}
async fn finish(jobs: &gpui_agent_application::Jobs, id: i64) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let job = jobs.job(id).await?;
            if job.state.terminal() {
                ensure!(job.state == JobState::Completed, "{job:?}");
                return Ok::<_, anyhow::Error>(());
            }
            if let Some(request) = job.pending_request_id {
                jobs.respond(id, request, serde_json::from_value(json!({"type":"approval","allow":false}))?)?;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?
}
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(args.first().map(String::as_str), Some("pr" | "api")) {
        return gh(&args);
    }
    if let Ok(spec) = std::fs::read_to_string("fixture.json") {
        return native::fixture(serde_json::from_str(&spec)?);
    }
    ensure!(args.is_empty(), "pull_requests accepts no filters");
    let root = tempfile::tempdir()?;
    let executable = root.path().join(if cfg!(windows) { "gh.exe" } else { "gh" });
    std::fs::copy(std::env::current_exe()?, &executable)?;
    // This fixture owns its process and sets host configuration before starting
    // threads.
    unsafe {
        std::env::set_var("gpui_agent_GH_EXECUTABLE", executable);
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(scenario(root.path()))?;
    println!("ok: GitHub PR watch through authenticated MCP and native gh");
    Ok(())
}
