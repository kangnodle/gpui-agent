use gpui_agent_application::{Application,
                             ApplicationResult,
                             Workspaces,
                             ports::WorkspaceBackend};
use gpui_agent_contracts::*;
use reqwest::Method;
use std::sync::{Arc,
                Mutex};
struct Host {
    app: Application,
    calls: Mutex<Vec<GitCommand>>,
}
fn version() -> GitVersion {
    GitVersion {
        branch: None,
        head: Some("head".into()),
        index: "index".into(),
        worktree: "tree".into(),
    }
}
impl WorkspaceBackend for Host {
    fn git(&self, id: i64, command: &GitCommand) -> ApplicationResult<GitOverview> {
        self.calls.lock().unwrap().push(command.clone());
        Ok(GitOverview {
            proposal: None,
            workspace_id: id,
            version: version(),
            checkpoints: vec![],
            files: vec![],
            diff: String::new(),
            truncated: false,
            backup: None,
            commit: None,
            pr: Some(PullRequest {
                repository: "owner/repo".into(),
                remote: "origin".into(),
                branch: "feature/test".into(),
                base: "main".into(),
                base_oid: "base".into(),
                commits: "head fix".into(),
                diff: "+change".into(),
                truncated: false,
                needs_push: false,
                url: matches!(command, GitCommand::CreatePr { .. }).then(|| "https://github.com/owner/repo/pull/7".into()),
            }),
        })
    }

    fn runtime(&self, provider: Provider, command: &RuntimeCommand) -> ApplicationResult<RuntimeInfo> {
        Ok(RuntimeInfo {
            provider,
            active_path: None,
            managed_version: match command {
                | RuntimeCommand::Install {
                    version, ..
                } => Some(version.clone()),
                | _ => None,
            },
            override_active: false,
            releases: if provider == Provider::Antigravity {
                vec![
                    RuntimeRelease {
                        version: "1.4.5".into(),
                        url: "https://dl.google.com/new.zip".into(),
                        sha256: "b".repeat(64),
                        size: None,
                    },
                    RuntimeRelease {
                        version: "1.0.0".into(),
                        url: "https://dl.google.com/old.zip".into(),
                        sha256: "a".repeat(64),
                        size: Some(10),
                    },
                ]
            } else {
                vec![]
            },
            issue: None,
            release: Some(RuntimeRelease {
                version: "1.0.0".into(),
                url: "https://dl.google.com/test".into(),
                sha256: "a".repeat(64),
                size: Some(10),
            }),
        })
    }

    fn application(&self, _: Option<i64>) -> ApplicationResult<Application> { Ok(self.app.clone()) }

    fn active_application(&self, _: Option<i64>) -> Option<Application> { Some(self.app.clone()) }

    fn cancel_all(&self) { self.app.cancel_all() }

    fn catalog(&self) -> ApplicationResult<ProjectCatalog> { Ok(ProjectCatalog::default()) }

    fn register(&self, _: &str) -> ApplicationResult<ProjectCatalog> { unreachable!() }

    fn forget_project(&self, _: i64) -> ApplicationResult<ProjectCatalog> { unreachable!() }

    fn create_worktree(&self, _: i64, _: &str, _: &str) -> ApplicationResult<ProjectCatalog> { unreachable!() }

    fn remove_worktree(&self, _: i64) -> ApplicationResult<ProjectCatalog> { unreachable!() }

    fn changes(&self, _: i64) -> ApplicationResult<GitChanges> { unreachable!() }

    fn diff(&self, _: i64, _: &str) -> ApplicationResult<FileDiff> { unreachable!() }
}
#[tokio::test]
async fn pr_and_runtime_http_contracts_are_authenticated_and_forward_reviewed_values() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let app = Application::new(
        std::array::from_fn(|_| Err(anyhow::anyhow!("unused"))),
        Arc::new(gpui_agent_infrastructure::History::open(&root.path().join("history.sqlite3"), root.path())?),
    );
    let backend = Arc::new(Host {
        app: app.clone(),
        calls: Mutex::new(vec![]),
    });
    let api = gpui_agent_api::Api::new(app, "maintenancecontracttokenlongenough123456789".into())?.with_workspaces(Workspaces::new(Some(1), backend.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let (stop, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = rx.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let request = |method, path: &str| {
        client
            .request(method, format!("{url}{path}"))
            .bearer_auth("maintenancecontracttokenlongenough123456789")
    };
    assert_eq!(client.get(format!("{url}/v1/runtimes/claude")).send().await?.status(), 401);
    assert_eq!(
        request(Method::GET, "/v1/runtimes/claude")
            .header("Origin", "https://evil.test")
            .send()
            .await?
            .status(),
        401
    );
    let preview: GitOverview = request(Method::POST, "/v1/workspaces/1/git")
        .json(&GitCommand::PreviewPr {
            remote: "origin".into(),
            base: "main".into(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(preview.pr.unwrap().url.is_none());
    let command = GitCommand::CreatePr {
        remote: "origin".into(),
        base: "main".into(),
        title: "Fix 한글".into(),
        body: "exact\n`body` $(body)".into(),
        draft: true,
        push: true,
        expected: version(),
        expected_base: "base".into(),
        expected_branch: "feature/test".into(),
        expected_repository: "owner/repo".into(),
    };
    let result: GitOverview = request(Method::POST, "/v1/workspaces/1/git")
        .json(&command)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(result.pr.unwrap().url.as_deref(), Some("https://github.com/owner/repo/pull/7"));
    assert!(matches!(backend.calls.lock().unwrap().last(),Some(GitCommand::CreatePr {body,draft:true,push:true,..}) if body=="exact\n`body` $(body)"));
    for provider in PROVIDERS {
        let path = format!("/v1/runtimes/{}", provider_name(provider));
        let checked: RuntimeInfo = request(Method::POST, &path)
            .json(&RuntimeCommand::Check)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if provider == Provider::Antigravity {
            assert_eq!(
                checked.releases.iter().map(|release| release.version.as_str()).collect::<Vec<_>>(),
                ["1.4.5", "1.0.0"]
            );
        }
        for release in checked.available_releases() {
            let installed: RuntimeInfo = request(Method::POST, &path)
                .json(&RuntimeCommand::Install {
                    version: release.version.clone(),
                    sha256: release.sha256.clone(),
                })
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            assert_eq!(installed.managed_version.as_deref(), Some(release.version.as_str()));
            assert_eq!(installed.provider, provider);
        }
        assert_eq!(
            request(Method::POST, &path)
                .json(&RuntimeCommand::Install {
                    version: "../bad".into(),
                    sha256: "a".repeat(64)
                })
                .send()
                .await?
                .status(),
            400
        );
    }
    let _ = stop.send(());
    task.await??;
    Ok(())
}
