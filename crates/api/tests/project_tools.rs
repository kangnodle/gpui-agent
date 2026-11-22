use anyhow::Result;
use gpui_agent_application::ports::JobRepository;
use gpui_agent_contracts::{JobScope,
                           McpScope,
                           ProjectCatalog,
                           Provider};
use gpui_agent_infrastructure::{JobStore,
                                McpSessions,
                                Projects,
                                Settings};
use serde_json::{Value,
                 json};
use std::{path::Path,
          sync::Arc,
          time::Duration};

const TOKEN: &str = "projectToolsOperatorCredential1234567890";

#[path = "project_tools/environment.rs"]
mod environment;
#[path = "project_tools/pull_requests.rs"]
mod pull_requests;

fn git(path: &Path, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "core.hooksPath=disabled-hooks",
            "-c",
            "user.name=Project Test",
            "-c",
            "user.email=test@example.invalid",
            "-C",
        ])
        .arg(path)
        .args(args)
        .output()?;
    anyhow::ensure!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    Ok(())
}

fn repo(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    git(path, &["init", "-b", "main"])?;
    std::fs::write(path.join("tracked.txt"), "original\n")?;
    git(path, &["add", "."])?;
    git(path, &["commit", "-m", "initial"])
}

struct Host {
    url: String,
    client: reqwest::Client,
    reader: String,
    writer: String,
    // Keep session credentials alive until the host is stopped.
    _leases: Vec<Box<dyn gpui_agent_application::ports::McpSessionLease>>,
    catalog: ProjectCatalog,
    store: Option<Arc<JobStore>>,
    sessions: Arc<McpSessions>,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<()>>,
    root: tempfile::TempDir,
}

impl Host {
    async fn start() -> Result<Self> { Self::with_jobs(false).await }

    async fn with_jobs(with_jobs: bool) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let cwd = root.path().join("project A 한글");
        let other = root.path().join("project B");
        repo(&cwd)?;
        repo(&other)?;
        let settings = Settings::discover(cwd.clone(), Some(root.path().join("home")))?;
        let app = gpui_agent_infrastructure::application_from_agents(
            std::array::from_fn(|_| Err(anyhow::anyhow!("no provider required"))),
            &settings.home.join("history.sqlite3"),
            &cwd,
        )?;
        let workspaces = Projects::open(settings.clone(), app.clone())?.service();
        let catalog = workspaces.register(other.display().to_string()).await?;
        let path = catalog.projects[0].workspaces[0].path.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/mcp", listener.local_addr()?);
        let sessions = Arc::new(McpSessions::new(url.clone())?);
        let scope = McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: path,
        };
        let reader = sessions.issue(scope.clone(), false)?;
        let writer = sessions.issue(scope, true)?;
        let reader_token = reader.connection().token.clone();
        let writer_token = writer.connection().token.clone();
        let mut api = gpui_agent_api::Api::new(app, TOKEN.into())?
            .with_workspaces(workspaces.clone())
            .with_mcp_sessions(sessions.clone());
        let store = if with_jobs {
            let store = Arc::new(JobStore::open(settings, workspaces.clone())?);
            api = api.with_jobs(gpui_agent_application::Jobs::new(store.clone(), workspaces));
            Some(store)
        } else {
            None
        };
        let (stop, done) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
            let _ = done.await;
        }));
        Ok(Self {
            root,
            url,
            client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(20)).build()?,
            reader: reader_token,
            writer: writer_token,
            _leases: vec![Box::new(reader), Box::new(writer)],
            catalog,
            store,
            sessions,
            stop,
            task,
        })
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> Result<Value> {
        Ok(self
            .client
            .post(&self.url)
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn call(&self, token: &str, name: &str, args: Value) -> Result<Value> { self.rpc(token, "tools/call", json!({"name":name,"arguments":args})).await }

    async fn stop(self) -> Result<()> {
        let _ = self.stop.send(());
        self.task.await??;
        Ok(())
    }
}

#[tokio::test]
async fn subdirectory_agent_reads_its_literal_changed_path() -> Result<()> {
    let host = Host::start().await?;
    let root = Path::new(&host.catalog.projects[0].workspaces[0].path);
    let notes = root.join("notes");
    std::fs::create_dir(&notes)?;
    for name in ["[a].txt", "a.txt"] {
        std::fs::write(notes.join(name), "original\n")?;
    }
    git(root, &["add", "notes"])?;
    git(root, &["commit", "-m", "add notes"])?;
    std::fs::write(notes.join("[a].txt"), "bracketed note\n")?;
    std::fs::write(notes.join("a.txt"), "plain note\n")?;
    std::fs::write(root.join("tracked.txt"), "root note\n")?;
    let catalog = host.call(TOKEN, "project_register", json!({"path":notes})).await?;
    let catalog: ProjectCatalog = serde_json::from_value(catalog["result"]["structuredContent"].clone())?;
    let workspace = catalog.projects[0]
        .workspaces
        .iter()
        .find(|workspace| Some(workspace.id) == catalog.registered_workspace_id)
        .unwrap();
    let lease = host.sessions.issue(
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: workspace.path.clone(),
        },
        false,
    )?;
    let token = &lease.connection().token;
    let changes = host.call(token, "workspace_changes", json!({})).await?;
    let files = changes["result"]["structuredContent"]["files"].as_array().unwrap();
    assert_eq!(
        files.iter().map(|file| file["path"].as_str().unwrap()).collect::<Vec<_>>(),
        ["notes/[a].txt", "notes/a.txt"]
    );
    let result = host.call(token, "workspace_diff", json!({"path":"notes/[a].txt"})).await?;
    let diff = result["result"]["structuredContent"]["unstaged"].as_str().unwrap();
    assert!(diff.contains("+bracketed note"));
    assert!(!diff.contains("+plain note"));
    let outside = host.call(token, "workspace_diff", json!({"path":"tracked.txt"})).await?;
    assert_eq!(outside["result"]["isError"], true);
    host.stop().await
}

#[tokio::test]
async fn scoped_agents_can_discover_their_workspace_and_read_changes_without_a_job_service() -> Result<()> {
    // Given two projects and a read-only agent scoped to the first workspace.
    let host = Host::start().await?;
    let own = &host.catalog.projects[0].workspaces[0];
    let other = &host.catalog.projects[1].workspaces[0];
    let tools = host.rpc(&host.reader, "tools/list", json!({})).await?;
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"project_list"));
    assert!(names.contains(&"workspace_changes"));
    assert!(names.contains(&"workspace_diff"));
    assert!(!names.contains(&"worktree_create"));
    // When the session lists projects, then sibling projects and worktrees stay
    // private.
    let catalog = host.call(&host.reader, "project_list", json!({})).await?;
    let catalog = &catalog["result"]["structuredContent"];
    assert_eq!(catalog["projects"].as_array().unwrap().len(), 1);
    assert_eq!(catalog["projects"][0]["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(catalog["default_workspace_id"], own.id);
    assert!(!catalog.to_string().contains(&other.path));
    let other_reader = host.sessions.issue(
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: other.path.clone(),
        },
        false,
    )?;
    let other_catalog = host.call(&other_reader.connection().token, "project_list", json!({})).await?;
    assert_eq!(other_catalog["result"]["structuredContent"]["default_workspace_id"], other.id);
    let other_changes = host.call(&other_reader.connection().token, "workspace_changes", json!({})).await?;
    assert_eq!(other_changes["result"]["structuredContent"]["workspace_id"], other.id);
    std::fs::write(Path::new(&own.path).join("tracked.txt"), "changed\n")?;
    let changes = host.call(&host.reader, "workspace_changes", json!({})).await?;
    assert_eq!(changes["result"]["structuredContent"]["workspace_id"], own.id);
    assert_eq!(changes["result"]["structuredContent"]["files"][0]["path"], "tracked.txt");
    let diff = host.call(&host.reader, "workspace_diff", json!({"path":"tracked.txt"})).await?;
    assert!(diff["result"]["structuredContent"]["unstaged"].as_str().unwrap().contains("+changed"));
    for name in ["workspace_changes", "workspace_diff"] {
        let mut args = json!({"workspace":other.id});
        if name == "workspace_diff" {
            args["path"] = json!("tracked.txt");
        }
        assert_eq!(host.call(&host.reader, name, args).await?["result"]["isError"], true);
    }
    assert_eq!(
        host.call(&host.reader, "project_list", json!({"path":"forged"})).await?["result"]["isError"],
        true
    );
    let all = host.call(TOKEN, "project_list", json!({})).await?;
    assert_eq!(all["result"]["structuredContent"]["projects"].as_array().unwrap().len(), 2);
    host.stop().await
}

#[tokio::test]
async fn operator_manages_worktrees_without_losing_dirty_files_branches_or_project_folders() -> Result<()> {
    // Given an operator and scoped agent writers, only the operator can manage
    // host directories.
    let host = Host::start().await?;
    let project = host.catalog.projects[0].id;
    let main = &host.catalog.projects[0].workspaces[0];
    let other = &host.catalog.projects[1];
    let tools = host.rpc(TOKEN, "tools/list", json!({})).await?;
    let writer_tools = host.rpc(&host.writer, "tools/list", json!({})).await?;
    for name in ["project_register", "project_forget", "worktree_create", "worktree_remove"] {
        assert!(tools["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == name));
        assert!(!writer_tools["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == name));
        assert_eq!(host.call(&host.writer, name, json!({})).await?["error"]["code"], -32001);
        assert_eq!(host.call(&host.reader, name, json!({})).await?["error"]["code"], -32001);
    }
    // When a worktree is created, invalid or duplicate branches cannot
    // overwrite it.
    for branch in ["../escape", "--force", "bad branch", "@{-1}"] {
        assert_eq!(
            host.call(TOKEN, "worktree_create", json!({"project":project,"branch":branch})).await?["result"]["isError"],
            true
        );
    }
    let created = host
        .call(TOKEN, "worktree_create", json!({"project":project,"branch":"feature/mcp-test"}))
        .await?;
    assert_ne!(created["result"]["isError"], true, "{created}");
    let catalog: ProjectCatalog = serde_json::from_value(created["result"]["structuredContent"].clone())?;
    let tree = catalog.projects[0].workspaces.iter().find(|workspace| workspace.managed).unwrap();
    assert!(Path::new(&tree.path).join("tracked.txt").exists());
    assert_eq!(
        host.call(TOKEN, "worktree_create", json!({"project":project,"branch":"feature/mcp-test"}))
            .await?["result"]["isError"],
        true
    );
    let own_catalog = host.call(&host.writer, "project_list", json!({})).await?;
    assert_eq!(
        own_catalog["result"]["structuredContent"]["projects"][0]["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        host.call(&host.writer, "workspace_changes", json!({"workspace":tree.id})).await?["result"]["isError"],
        true
    );
    assert_eq!(host.call(TOKEN, "worktree_remove", json!({"id":main.id})).await?["result"]["isError"], true);
    std::fs::write(Path::new(&tree.path).join("untracked.txt"), "keep me")?;
    assert_eq!(host.call(TOKEN, "worktree_remove", json!({"id":tree.id})).await?["result"]["isError"], true);
    assert_eq!(std::fs::read_to_string(Path::new(&tree.path).join("untracked.txt"))?, "keep me");
    assert_eq!(
        host.call(TOKEN, "worktree_remove", json!({"id":tree.id,"force":true})).await?["result"]["isError"],
        true
    );
    std::fs::remove_file(Path::new(&tree.path).join("untracked.txt"))?;
    git(Path::new(&main.path), &["worktree", "lock", &tree.path])?;
    assert_eq!(host.call(TOKEN, "worktree_remove", json!({"id":tree.id})).await?["result"]["isError"], true);
    git(Path::new(&main.path), &["worktree", "unlock", &tree.path])?;
    assert_ne!(host.call(TOKEN, "worktree_remove", json!({"id":tree.id})).await?["result"]["isError"], true);
    assert!(!Path::new(&tree.path).exists());
    git(Path::new(&main.path), &["show-ref", "--verify", "refs/heads/feature/mcp-test"])?;
    // Then forgetting only hides a project, and registration restores its
    // stable ID.
    let forgotten = host.call(TOKEN, "project_forget", json!({"id":other.id})).await?;
    assert_eq!(forgotten["result"]["structuredContent"]["projects"].as_array().unwrap().len(), 1);
    assert_eq!(std::fs::read_to_string(Path::new(&other.path).join("tracked.txt"))?, "original\n");
    let restored = host.call(TOKEN, "project_register", json!({"path":other.path})).await?;
    assert_eq!(restored["result"]["structuredContent"]["registered_workspace_id"], other.workspaces[0].id);
    for path in [".", "", "does-not-exist"] {
        assert_eq!(host.call(TOKEN, "project_register", json!({"path":path})).await?["result"]["isError"], true);
    }
    let plain = host.root.path().join("plain project");
    std::fs::create_dir(&plain)?;
    let registered = host.call(TOKEN, "project_register", json!({"path":plain})).await?;
    assert_eq!(registered["result"]["structuredContent"]["projects"].as_array().unwrap().len(), 3);
    host.stop().await
}

#[tokio::test]
async fn pending_work_in_a_child_worktree_blocks_mcp_and_http_removal_until_cancelled() -> Result<()> {
    // Given a durable queued job in a managed worktree, both clients must
    // preserve it.
    let host = Host::with_jobs(true).await?;
    let project = host.catalog.projects[0].id;
    let created = host
        .call(TOKEN, "worktree_create", json!({"project":project,"branch":"feature/queued"}))
        .await?;
    let catalog: ProjectCatalog = serde_json::from_value(created["result"]["structuredContent"].clone())?;
    let tree = catalog.projects[0].workspaces.iter().find(|workspace| workspace.managed).unwrap();
    let job = host.store.as_ref().unwrap().enqueue(
        &JobScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_id: Some(tree.id),
            workspace_path: tree.path.clone(),
        },
        &serde_json::from_value(json!({"text":"queued work"}))?,
    )?;
    for (name, id, path) in [
        ("project_forget", project, format!("/v1/projects/{project}")),
        ("worktree_remove", tree.id, format!("/v1/workspaces/{}", tree.id)),
    ] {
        assert_eq!(host.call(TOKEN, name, json!({"id":id})).await?["result"]["isError"], true);
        assert_eq!(
            host.client
                .delete(format!("{}{path}", host.url.trim_end_matches("/mcp")))
                .bearer_auth(TOKEN)
                .send()
                .await?
                .status(),
            409
        );
    }
    assert!(Path::new(&tree.path).exists());
    // When cancelled, then the clean worktree can be removed through the same
    // MCP service.
    assert_ne!(host.call(TOKEN, "cancel_job", json!({"id":job.id})).await?["result"]["isError"], true);
    let uploaded = host
        .call(
            TOKEN,
            "attachment_upload",
            json!({"provider":"codex","workspace":tree.id,"attachment":{"type":"text","name":"notes.txt","text":"keep this snapshot"}}),
        )
        .await?;
    assert_ne!(uploaded["result"]["isError"], true, "{uploaded}");
    assert_eq!(
        host.call(TOKEN, "worktree_remove", json!({"id":tree.id})).await?["result"]["isError"],
        true,
        "Staged uploads must remain manageable until explicitly deleted"
    );
    assert_eq!(
        host.client
            .delete(format!("{}/v1/workspaces/{}", host.url.trim_end_matches("/mcp"), tree.id))
            .bearer_auth(TOKEN)
            .send()
            .await?
            .status(),
        409
    );
    let upload_id = &uploaded["result"]["structuredContent"]["id"];
    assert_ne!(
        host.call(TOKEN, "attachment_delete", json!({"provider":"codex","workspace":tree.id,"id":upload_id}))
            .await?["result"]["isError"],
        true
    );
    assert_ne!(host.call(TOKEN, "worktree_remove", json!({"id":tree.id})).await?["result"]["isError"], true);
    assert!(!Path::new(&tree.path).exists());
    host.stop().await
}

#[tokio::test]
async fn a_registered_directory_replaced_by_a_link_cannot_expose_another_workspace() -> Result<()> {
    // Given an authorized path replaced by a directory link after registration.
    let host = Host::start().await?;
    let own = Path::new(&host.catalog.projects[0].workspaces[0].path);
    let other = Path::new(&host.catalog.projects[1].workspaces[0].path);
    std::fs::write(other.join("tracked.txt"), "private change\n")?;
    assert_eq!(own.parent().unwrap().canonicalize()?, host.root.path().canonicalize()?);
    std::fs::rename(own, host.root.path().join("original project"))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(other, own)?;
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(own).arg(other).output()?;
        anyhow::ensure!(
            output.status.success(),
            "Cannot create fixture junction: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // When changes/diff are requested, then canonical path validation rejects
    // the redirect.
    for (name, args) in [("workspace_changes", json!({})), ("workspace_diff", json!({"path":"tracked.txt"}))] {
        let response = host.call(&host.reader, name, args).await?;
        assert_eq!(response["result"]["isError"], true, "{response}");
        assert!(!response.to_string().contains("private change"));
    }
    #[cfg(windows)]
    std::fs::remove_dir(own)?;
    #[cfg(unix)]
    std::fs::remove_file(own)?;
    host.stop().await
}

#[tokio::test]
async fn given_scoped_agent_changes_when_proposed_then_only_operator_can_commit_selected_files() -> Result<()> {
    // Given staged unrelated work and a scoped agent's changed and new files.
    let host = Host::with_jobs(true).await?;
    let workspace = &host.catalog.projects[0].workspaces[0];
    let path = Path::new(&workspace.path);
    git(path, &["config", "user.name", "Proposal Test"])?;
    git(path, &["config", "user.email", "proposal@example.invalid"])?;
    std::fs::write(path.join("unrelated.txt"), "keep staged")?;
    git(path, &["add", "unrelated.txt"])?;
    std::fs::write(path.join("intent.txt"), "keep intent to add")?;
    git(path, &["add", "--intent-to-add", "intent.txt"])?;
    std::fs::write(path.join("tracked.txt"), "agent edit\n")?;
    std::fs::write(path.join("new.txt"), "agent new\n")?;
    // When a reader agent proposes a commit, it persists intent without
    // changing Git.
    let proposal = host
        .call(
            &host.reader,
            "workspace_propose_commit",
            json!({"message":"Agent proposal", "paths":["tracked.txt","new.txt"]}),
        )
        .await?;
    assert_ne!(proposal["result"]["isError"], true, "{proposal}");
    let overview = &proposal["result"]["structuredContent"];
    assert_eq!(overview["proposal"]["message"], "Agent proposal", "{proposal}");
    assert!(overview["commit"].is_null());
    let version = overview["version"].clone();
    assert_eq!(overview["proposal"]["expected"], version);
    let route = host.url.replace("/mcp", &format!("/v1/workspaces/{}/git", workspace.id));
    let read: Value = host.client.get(&route).bearer_auth(TOKEN).send().await?.error_for_status()?.json().await?;
    assert_eq!(read["version"], version);
    assert_eq!(read["proposal"], overview["proposal"]);
    let command = json!({"type":"commit_selected", "message":"Reviewed message", "paths":["new.txt"], "expected":version});
    assert_eq!(
        host.client.post(&route).bearer_auth(&host.writer).json(&command).send().await?.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let other = host
        .call(
            &host.reader,
            "workspace_propose_commit",
            json!({"workspace":host.catalog.projects[1].workspaces[0].id,"message":"escape","paths":["tracked.txt"]}),
        )
        .await?;
    assert_eq!(other["result"]["isError"], true);
    // Then operator approval commits exactly the edited selection and preserves
    // the unrelated index.
    let response = host
        .client
        .post(&route)
        .bearer_auth(TOKEN)
        .header("x-knd-workspace", workspace.id)
        .json(&command)
        .send()
        .await?;
    let status = response.status();
    let committed: Value = response.json().await?;
    assert!(status.is_success(), "{status}: {committed}");
    assert!(committed["commit"].is_string());
    assert!(committed["proposal"].is_null());
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["show", "--format=%s", "--name-only", "HEAD"])
        .output()?;
    assert_eq!(String::from_utf8(output.stdout)?.trim(), "Reviewed message\n\nnew.txt");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["diff", "--cached", "--name-only"])
        .output()?;
    assert_eq!(String::from_utf8(output.stdout)?.trim(), "unrelated.txt");
    git(path, &["ls-files", "--error-unmatch", "intent.txt"])?;
    assert_eq!(std::fs::read_to_string(path.join("tracked.txt"))?, "agent edit\n");
    host.stop().await
}
