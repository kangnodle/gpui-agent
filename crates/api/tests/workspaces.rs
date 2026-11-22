#[allow(dead_code)]
#[path = "../../agent/tests/protocols.rs"]
mod native;
use anyhow::{Context,
             Result,
             ensure};
use gpui_agent_api::{Api,
                     FileDiff,
                     GitChanges,
                     ProjectCatalog,
                     StreamEvent};
use gpui_agent_application::{Application,
                             ports::ConversationRepository};
use gpui_agent_infrastructure::{History,
                                Projects,
                                Settings};
use reqwest::{Method,
              StatusCode};
use serde_json::{Value,
                 json};
use std::{fs,
          path::{Path,
                 PathBuf},
          time::Duration};
const TOKEN: &str = "workspaceintegrationtokenlongenough123456789";
fn git(path: &Path, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "core.hooksPath=disabled-hooks",
            "-c",
            "user.name=Workspace Test",
            "-c",
            "user.email=workspace@example.invalid",
            "-C",
        ])
        .arg(path)
        .args(args)
        .output()?;
    ensure!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    Ok(())
}
fn repo(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    git(path, &["init", "-b", "main"])?;
    fs::write(path.join("tracked.txt"), "original\n")?;
    fs::write(path.join("fixture.json"), json!({"provider":"codex","scenario":"normal"}).to_string())?;
    git(path, &["add", "."])?;
    git(path, &["commit", "-m", "initial"])
}
fn application(settings: &Settings, cwd: &Path) -> Result<Application> {
    let config = gpui_agent::Config {
        provider: gpui_agent::Provider::Codex,
        executable: std::env::current_exe()?,
        home: settings.home.join("codex"),
        cwd: cwd.into(),
        harness: None,
        browser_helper: None,
        timeout: Duration::from_secs(30),
        mcp_home: None,
        mcp_selection: None,
        plugin_mcp: vec![],
    };
    gpui_agent_infrastructure::application_from_agents(
        [Err(anyhow::anyhow!("fixture")), gpui_agent::Agent::new(config), Err(anyhow::anyhow!("fixture"))],
        &settings.home.join("history.sqlite3"),
        cwd,
    )
}
struct Server {
    workspaces: gpui_agent_application::Workspaces,
    url: String,
    client: reqwest::Client,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl Server {
    async fn start(settings: Settings) -> Result<Self> {
        let app = application(&settings, &settings.cwd)?;
        let factory_settings = settings.clone();
        let workspaces = Projects::open(settings.clone(), app.clone())?
            .with_application_factory(move |cwd| application(&factory_settings, cwd))
            .service();
        let api = Api::new(app, TOKEN.into())?.with_workspaces(workspaces.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let (stop, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
            let _ = rx.await;
        }));
        Ok(Self {
            workspaces,
            url,
            client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(20)).build()?,
            stop,
            task,
        })
    }

    fn request(&self, method: Method, path: &str, id: Option<i64>) -> reqwest::RequestBuilder {
        let request = self.client.request(method, format!("{}{path}", self.url)).bearer_auth(TOKEN);
        if let Some(id) = id {
            request.header("X-KND-Workspace", id.to_string())
        } else {
            request
        }
    }

    async fn stop(self) -> Result<()> {
        let _ = self.stop.send(());
        self.task.await??;
        Ok(())
    }

    async fn list(&self, id: Option<i64>) -> Result<Vec<Value>> {
        Ok(self
            .request(Method::GET, "/v1/providers/codex/conversations", id)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn chat(&self, id: i64, conversation: Option<i64>) -> Result<i64> {
        let mut response = self
            .request(Method::POST, "/v1/providers/codex/chat", Some(id))
            .json(&json!({"text":format!("scope-{id}"),"conversation_id":conversation}))
            .send()
            .await?
            .error_for_status()?;
        let mut buffer = Vec::new();
        let mut saved = None;
        let mut completed = false;
        while let Some(event) = next(&mut response, &mut buffer).await? {
            match event {
                | StreamEvent::Conversation {
                    conversation_id,
                } => saved = Some(conversation_id),
                | StreamEvent::Interaction {
                    operation_id,
                    request_id,
                    ..
                } => {
                    self.request(Method::POST, &format!("/v1/operations/{operation_id}/interactions/{request_id}"), Some(id))
                        .json(&json!({"type":"approval","allow":false}))
                        .send()
                        .await?
                        .error_for_status()?;
                },
                | StreamEvent::Completed {
                    text, ..
                } => {
                    ensure!(text == "Hello 세계");
                    completed = true;
                },
                | StreamEvent::Error {
                    message,
                } => anyhow::bail!(message),
                | _ => {},
            }
        }
        ensure!(completed);
        saved.context("Missing conversation")
    }
}
async fn next(response: &mut reqwest::Response, buffer: &mut Vec<u8>) -> Result<Option<StreamEvent>> {
    loop {
        if let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
            return Ok(Some(serde_json::from_slice(&buffer.drain(..= end).collect::<Vec<_>>())?));
        }
        let Some(chunk) = response.chunk().await? else {
            ensure!(buffer.is_empty());
            return Ok(None);
        };
        buffer.extend_from_slice(&chunk);
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    if gpui_agent::run_browser_helper() {
        return Ok(());
    }
    if let Ok(spec) = fs::read_to_string("fixture.json") {
        return native::fixture(serde_json::from_str(&spec)?);
    }
    native::test_arguments(&[])?;
    let root = tempfile::tempdir()?;
    let repo_a = root.path().join("project A 한글");
    let repo_b = root.path().join("project B");
    repo(&repo_a)?;
    repo(&repo_b)?;
    let settings = Settings::discover(repo_a.clone(), Some(root.path().join("home")))?;
    let server = Server::start(settings.clone()).await?;
    let catalog: ProjectCatalog = server
        .request(Method::GET, "/v1/projects", None)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let main_a = catalog.default_workspace_id.unwrap();
    let project_a = catalog.projects[0].id;
    ensure!(
        server
            .client
            .post(format!("{}/v1/projects", server.url))
            .json(&json!({"path":repo_b}))
            .send()
            .await?
            .status()
            == StatusCode::UNAUTHORIZED
    );
    for path in [".", "", "does-not-exist"] {
        ensure!(
            server
                .request(Method::POST, "/v1/projects", None)
                .json(&json!({"path":path}))
                .send()
                .await?
                .status()
                == StatusCode::BAD_REQUEST
        );
    }
    ensure!(server.request(Method::GET, "/v1/providers", Some(999999)).send().await?.status() == StatusCode::NOT_FOUND);
    let catalog: ProjectCatalog = server
        .request(Method::POST, "/v1/projects", None)
        .json(&json!({"path":repo_b}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let main_b = catalog.projects.iter().find(|project| project.id != project_a).unwrap().workspaces[0].id;
    ensure!(catalog.registered_workspace_id == Some(main_b));
    for branch in ["../escape", "--force", "bad branch", "@{-1}"] {
        ensure!(
            server
                .request(Method::POST, &format!("/v1/projects/{project_a}/worktrees"), None)
                .json(&json!({"branch":branch}))
                .send()
                .await?
                .status()
                == StatusCode::BAD_REQUEST
        );
    }
    let catalog: ProjectCatalog = server
        .request(Method::POST, &format!("/v1/projects/{project_a}/worktrees"), None)
        .json(&json!({"branch":"feature/workspace-test"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let tree = catalog
        .projects
        .iter()
        .find(|project| project.id == project_a)
        .unwrap()
        .workspaces
        .iter()
        .find(|workspace| workspace.managed)
        .unwrap()
        .clone();
    let reopened: ProjectCatalog = server
        .request(Method::POST, "/v1/projects", None)
        .json(&json!({"path":Path::new(&tree.path).join(".")}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(reopened.registered_workspace_id == Some(tree.id));
    ensure!(reopened.default_workspace_id == Some(main_a));
    fs::write(settings.home.join("codex/fixture-auth"), "fixture")?;
    for (path, value) in [(&repo_a, "context-A"), (&repo_b, "context-B"), (&PathBuf::from(&tree.path), "context-tree")] {
        fs::write(path.join("context.txt"), value)?;
    }
    for (workspace, expected) in [(main_a, "context-A"), (main_b, "context-B"), (tree.id, "context-tree")] {
        let context: Value = server
            .request(Method::POST, "/v1/providers/codex/context", Some(workspace))
            .json(&json!({"path":"context.txt"}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(context["text"] == expected, "File context leaked across workspace scopes");
    }
    fs::remove_file(PathBuf::from(&tree.path).join("context.txt"))?;
    let mut conversations = Vec::new();
    for workspace in [main_a, tree.id, main_b] {
        ensure!(server.list(Some(workspace)).await?.is_empty());
        let id = server.chat(workspace, None).await?;
        conversations.push((workspace, id));
    }
    let (a, b) = tokio::join!(server.list(Some(main_a)), server.list(Some(main_b)));
    ensure!(a?[0]["id"] == conversations[0].1 && b?[0]["id"] == conversations[2].1);
    for (workspace, id) in &conversations {
        let list = server.list(Some(*workspace)).await?;
        ensure!(list.len() == 1 && list[0]["id"] == *id);
        let other = if *workspace == main_a { main_b } else { main_a };
        for method in [Method::GET, Method::DELETE] {
            ensure!(
                server
                    .request(method, &format!("/v1/providers/codex/conversations/{id}"), Some(other))
                    .send()
                    .await?
                    .status()
                    == StatusCode::NOT_FOUND
            );
        }
        ensure!(
            !server
                .request(Method::POST, "/v1/providers/codex/chat", Some(other))
                .json(&json!({"text":"must not resume another workspace","conversation_id":id}))
                .send()
                .await?
                .status()
                .is_success()
        );
        ensure!(server.chat(*workspace, Some(*id)).await? == *id);
    }
    ensure!(server.list(None).await?[0]["id"] == conversations[0].1);
    let requests = fs::read_to_string(settings.home.join("codex/requests.jsonl"))?;
    let starts: Vec<Value> = requests.lines().map(serde_json::from_str).collect::<Result<_, _>>()?;
    for path in [&repo_a, &repo_b, &PathBuf::from(&tree.path)] {
        ensure!(
            starts.iter().any(|value| value["method"] == "thread/start"
                && Path::new(value["params"]["cwd"].as_str().unwrap_or("")).canonicalize().ok() == path.canonicalize().ok()),
            "Native runtime did not receive {path:?}"
        );
    }
    println!("ok: project/worktree-scoped chat, runtime cwd, history, resume, cross-scope denial and concurrent clients");
    let project_b = catalog.projects.iter().find(|project| project.id != project_a).unwrap().id;
    ensure!(server.client.delete(format!("{}/v1/projects/{project_b}", server.url)).send().await?.status() == StatusCode::UNAUTHORIZED);
    let before = server.list(Some(main_b)).await?;
    let hidden: ProjectCatalog = server
        .request(Method::DELETE, &format!("/v1/projects/{project_b}"), None)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(hidden.projects.iter().all(|project| project.id != project_b));
    ensure!(fs::read_to_string(repo_b.join("tracked.txt"))? == "original\n");
    let restored: ProjectCatalog = server
        .request(Method::POST, "/v1/projects", None)
        .json(&json!({"path":repo_b}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(restored.registered_workspace_id == Some(main_b));
    ensure!(
        server.list(Some(main_b)).await? == before,
        "Removing from the list must retain all conversations"
    );
    ensure!(server.request(Method::DELETE, "/v1/projects/999999", None).send().await?.status() == StatusCode::NOT_FOUND);
    let tree_path = Path::new(&tree.path);
    fs::write(tree_path.join("tracked.txt"), "staged line\n")?;
    git(tree_path, &["add", "tracked.txt"])?;
    fs::write(tree_path.join("tracked.txt"), "unstaged line\n")?;
    fs::write(tree_path.join("new 한글.txt"), "new body\n")?;
    let changes: GitChanges = server
        .request(Method::GET, &format!("/v1/workspaces/{}/changes", tree.id), None)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(changes.files.len() == 2);
    ensure!(fs::read_to_string(repo_a.join("tracked.txt"))? == "original\n");
    let diff: FileDiff = server
        .request(Method::GET, &format!("/v1/workspaces/{}/diff", tree.id), None)
        .query(&[("path", "tracked.txt")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(diff.staged.contains("+staged line") && diff.unstaged.contains("+unstaged line"));
    let diff: FileDiff = server
        .request(Method::GET, &format!("/v1/workspaces/{}/diff", tree.id), None)
        .query(&[("path", "new 한글.txt")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(diff.untracked && diff.unstaged.contains("+new body"));
    ensure!(
        server
            .request(Method::GET, &format!("/v1/workspaces/{}/diff", tree.id), None)
            .query(&[("path", "../outside")])
            .send()
            .await?
            .status()
            == StatusCode::NOT_FOUND
    );
    ensure!(
        server
            .request(Method::DELETE, &format!("/v1/workspaces/{}", tree.id), None)
            .send()
            .await?
            .status()
            == StatusCode::CONFLICT
    );
    ensure!(tree_path.exists());
    git(tree_path, &["restore", "--staged", "--worktree", "--", "tracked.txt"])?;
    fs::remove_file(tree_path.join("new 한글.txt"))?;
    fs::write(tree_path.join("fixture.json"), json!({"provider":"codex","scenario":"cancel"}).to_string())?;
    let mut response = server
        .request(Method::POST, "/v1/providers/codex/chat", Some(tree.id))
        .json(&json!({"text":"cancel"}))
        .send()
        .await?
        .error_for_status()?;
    let mut buffer = Vec::new();
    let mut operation = None;
    while let Some(event) = next(&mut response, &mut buffer).await? {
        if let StreamEvent::Started {
            operation_id,
        } = event
        {
            operation = Some(operation_id);
        }
        if matches!(event, StreamEvent::Agent(gpui_agent_api::Event::Session { .. })) {
            break;
        }
    }
    ensure!(
        server
            .request(Method::DELETE, &format!("/v1/workspaces/{}", tree.id), None)
            .send()
            .await?
            .status()
            == StatusCode::CONFLICT
    );
    ensure!(
        server
            .request(Method::DELETE, &format!("/v1/operations/{}", operation.unwrap()), Some(main_a))
            .send()
            .await?
            .status()
            == StatusCode::NOT_FOUND
    );
    // The active operation is in a child worktree of this project.
    ensure!(
        server
            .request(Method::DELETE, &format!("/v1/projects/{project_a}"), None)
            .send()
            .await?
            .status()
            == StatusCode::CONFLICT
    );
    let lease = server.workspaces.scope(Some(tree.id)).await?;
    let workspaces = server.workspaces.clone();
    let queued_creation = tokio::spawn(async move { workspaces.create(project_a, "feature/concurrent".into(), "HEAD".into()).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    server
        .request(Method::DELETE, &format!("/v1/operations/{}", operation.unwrap()), Some(tree.id))
        .timeout(Duration::from_secs(2))
        .send()
        .await?
        .error_for_status()?;
    let mut cancelled = false;
    while let Some(event) = next(&mut response, &mut buffer).await? {
        cancelled |= matches!(event, StreamEvent::Cancelled);
    }
    ensure!(cancelled);
    drop(lease);
    let created = queued_creation.await??;
    let concurrent = created
        .projects
        .iter()
        .flat_map(|project| &project.workspaces)
        .find(|workspace| workspace.branch.as_deref() == Some("feature/concurrent"))
        .unwrap();
    server
        .request(Method::DELETE, &format!("/v1/workspaces/{}", concurrent.id), None)
        .send()
        .await?
        .error_for_status()?;
    git(tree_path, &["restore", "--", "fixture.json"])?;
    server.stop().await?;
    let server = Server::start(settings.clone()).await?;
    let catalog: ProjectCatalog = server
        .request(Method::GET, "/v1/projects", None)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        catalog.projects.len() == 2
            && catalog
                .projects
                .iter()
                .flat_map(|project| &project.workspaces)
                .any(|workspace| workspace.id == tree.id)
    );
    for (workspace, id) in &conversations {
        ensure!(server.list(Some(*workspace)).await?.iter().any(|saved| saved["id"] == *id));
    }
    let history = History::open(&settings.home.join("history.sqlite3"), tree_path)?;
    let catalog: ProjectCatalog = server
        .request(Method::DELETE, &format!("/v1/workspaces/{}", tree.id), None)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        !tree_path.exists()
            && !catalog
                .projects
                .iter()
                .flat_map(|project| &project.workspaces)
                .any(|workspace| workspace.id == tree.id)
    );
    ensure!(
        !history.list(gpui_agent_api::Provider::Codex, "")?.is_empty(),
        "Removing a worktree must retain chat history"
    );
    ensure!(
        server
            .request(Method::GET, "/v1/providers/codex/conversations", Some(tree.id))
            .send()
            .await?
            .status()
            == StatusCode::NOT_FOUND
    );
    git(&repo_a, &["show-ref", "--verify", "refs/heads/feature/workspace-test"])?;
    server.stop().await?;
    println!("ok: staged/unstaged/new-file diff, dirty/busy deletion guard, cancellation, restart and history-retaining removal");
    Ok(())
}
