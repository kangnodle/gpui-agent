use gpui_agent_contracts::*;
use gpui_agent_infrastructure::{History,
                                Projects,
                                Settings};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn given_project_required_checks_when_api_clients_configure_or_use_legacy_chat_then_only_the_operator_can_configure_and_verification_cannot_be_bypassed()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let app = gpui_agent_application::Application::new(
        PROVIDERS.map(|_| Err(anyhow::anyhow!("fixture runtime unavailable"))),
        Arc::new(History::open(&settings.home.join("history.sqlite3"), root.path())?),
    );
    let workspaces = Projects::open(settings.clone(), app.clone())?.service();
    let project = workspaces.catalog().await?.projects[0].id;
    let jobs = settings.jobs(workspaces.clone())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let sessions = Arc::new(gpui_agent_infrastructure::McpSessions::new(format!("{base}/mcp"))?);
    let grant = sessions.issue_in_conversation(
        McpScope {
            provider: Provider::Claude,
            account_id: "default".into(),
            workspace_path: root.path().display().to_string(),
        },
        true,
        None,
    )?;
    let token = "verificationoperatorfixturetoken1234567890";
    let api = gpui_agent_api::Api::new(app, token.into())?
        .with_mcp_sessions(sessions)
        .with_workspaces(workspaces)
        .with_jobs(jobs);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let route = format!("{base}/v1/projects/{project}/verification");
    assert_eq!(client.put(&route).json(&json!({"commands":[]})).send().await?.status(), 401);
    assert_eq!(
        client
            .put(&route)
            .bearer_auth(&grant.connection().token)
            .json(&json!({"commands":[]}))
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        client.put(&route).bearer_auth(token).json(&json!({"commands":[[]]})).send().await?.status(),
        400
    );
    let catalog: ProjectCatalog = client
        .put(&route)
        .bearer_auth(token)
        .json(&json!({"commands":[["bun","test","server"],["bun","run","check"]]}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(catalog.projects[0].verification.commands.len(), 2);
    assert_eq!(
        client
            .post(format!("{base}/v1/providers/claude/chat"))
            .bearer_auth(token)
            .json(&json!({"text":"Attempt direct chat"}))
            .send()
            .await?
            .status(),
        409
    );
    let _ = stop.send(());
    host.await??;
    Ok(())
}
