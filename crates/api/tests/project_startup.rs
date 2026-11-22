use gpui_agent_api::Api;
use gpui_agent_contracts::ProjectCatalog;
use gpui_agent_infrastructure::{Projects,
                                Settings};
use reqwest::StatusCode;
use serde_json::{Value,
                 json};

#[tokio::test]
async fn given_a_fresh_desktop_profile_when_opened_then_only_explicitly_opened_projects_are_persisted() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let startup = temp.path().join("startup-folder");
    let chosen = temp.path().join("chosen-project");
    std::fs::create_dir(&startup)?;
    std::fs::create_dir(&chosen)?;
    std::fs::write(startup.join("startup-only.txt"), "startup")?;
    std::fs::write(chosen.join("chosen-only.txt"), "chosen")?;
    let settings = Settings::discover(startup, Some(temp.path().join("home")))?;
    let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(10)).build()?;
    let token = "freshprojectprofiletoken12345678901234567890";
    for reopened in [false, true] {
        let application = settings.application()?;
        let workspaces = Projects::open_catalog(settings.clone(), application.clone())?.service();
        let jobs = settings.jobs(workspaces.clone())?;
        let api = Api::new(application, token.into())?.with_workspaces(workspaces).with_jobs(jobs);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
            let _ = stopped.await;
        }));
        let mut catalog: ProjectCatalog = client
            .get(format!("{origin}/v1/projects"))
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if !reopened {
            // Given an empty profile, starting the host never registers its
            // cwd.
            assert!(catalog.projects.is_empty());
            assert_eq!(catalog.default_workspace_id, None);
            assert_eq!(
                client.get(format!("{origin}/v1/providers")).bearer_auth(token).send().await?.status(),
                StatusCode::OK
            );
            let accounts: Value = client
                .get(format!("{origin}/v1/providers/claude/accounts"))
                .bearer_auth(token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            assert_eq!(accounts[0]["id"], "default");
            assert_eq!(
                client
                    .post(format!("{origin}/v1/providers/claude/jobs"))
                    .bearer_auth(token)
                    .json(&json!({"text":"no project selected"}))
                    .send()
                    .await?
                    .status(),
                StatusCode::NOT_FOUND
            );
            // When the user opens a folder, only that project is registered.
            catalog = client
                .post(format!("{origin}/v1/projects"))
                .bearer_auth(token)
                .json(&json!({"path":chosen}))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            assert_eq!(catalog.registered_workspace_id, catalog.default_workspace_id);
        }
        // Then reopening the profile restores the selection without adding cwd.
        assert_eq!(catalog.projects.len(), 1);
        assert_eq!(catalog.projects[0].name, "chosen-project");
        let id = catalog.default_workspace_id.unwrap();
        let listing: Value = client
            .get(format!("{origin}/v1/providers/claude/files"))
            .bearer_auth(token)
            .header("X-KND-Workspace", id)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(listing["entries"], json!([{"path":"chosen-only.txt","directory":false}]));
        stop.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), task).await???;
    }
    Ok(())
}
