use super::*;

fn reopen(host: &Host, home: &str) -> Result<gpui_agent_application::Workspaces> {
    let cwd = Path::new(&host.catalog.projects[0].workspaces[0].path);
    let settings = Settings::discover(cwd.into(), Some(host.root.path().join(home)))?;
    let app = gpui_agent_infrastructure::application_from_agents(
        std::array::from_fn(|_| Err(anyhow::anyhow!("no runtime needed"))),
        &settings.home.join("history.sqlite3"),
        cwd,
    )?;
    Ok(Projects::open(settings, app)?.service())
}

#[tokio::test]
async fn scoped_readers_can_inspect_the_host_without_receiving_private_paths_or_credentials() -> Result<()> {
    // Given a scoped reader and an operator on a host without a Jobs service.
    let host = Host::start().await?;
    // When the reader inspects the environment, then only safe identity and
    // preferences are returned.
    let reply = host.call(&host.reader, "environment_read", json!({})).await?;
    let info = &reply["result"]["structuredContent"];
    assert!(
        info["environment_id"].as_str().is_some_and(|id| id.len() == 64),
        "Environment identity must be available"
    );
    assert_eq!(info["preferences"]["label"], "gpui-agent");
    assert_eq!(info["preferences"]["default_worktree_base"], "HEAD");
    assert_eq!(info["platform"]["os"], std::env::consts::OS);
    assert!(!reply.to_string().contains(TOKEN));
    assert!(!reply.to_string().contains(host.root.path().to_str().unwrap()));
    assert!(info.get("home").is_none());
    let other = reopen(&host, "independent-home")?.environment_read().await?;
    assert_ne!(other.environment_id, info["environment_id"]);
    let http: Value = host
        .client
        .get(format!("{}/v1/environment", host.url.trim_end_matches("/mcp")))
        .bearer_auth(TOKEN)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(&http, info);
    let tools = host.rpc(&host.reader, "tools/list", json!({})).await?;
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "environment_read")
    );
    host.stop().await
}

#[tokio::test]
async fn worktree_creation_uses_the_persisted_default_and_explicit_bases_take_precedence() -> Result<()> {
    // Given a project whose configured baseline differs from its current HEAD.
    let host = Host::start().await?;
    let project = &host.catalog.projects[0];
    let root = Path::new(&project.path);
    git(root, &["branch", "baseline"])?;
    std::fs::write(root.join("tracked.txt"), "new HEAD\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-m", "advance main"])?;
    let patched = host
        .call(TOKEN, "environment_preferences_update", json!({"default_worktree_base":"baseline"}))
        .await?;
    assert_ne!(patched["result"]["isError"], true);
    // When base is omitted in MCP or empty in the desktop HTTP contract, then
    // both use baseline.
    let created = host
        .call(TOKEN, "worktree_create", json!({"project":project.id,"branch":"from-default-mcp"}))
        .await?;
    let catalog: ProjectCatalog = serde_json::from_value(created["result"]["structuredContent"].clone())?;
    let tree = catalog
        .projects
        .iter()
        .flat_map(|p| &p.workspaces)
        .find(|w| w.branch.as_deref() == Some("from-default-mcp"))
        .unwrap();
    assert_eq!(std::fs::read_to_string(Path::new(&tree.path).join("tracked.txt"))?, "original\n");
    let endpoint = format!("{}/v1/projects/{}/worktrees", host.url.trim_end_matches("/mcp"), project.id);
    for (branch, base, expected) in [("from-default-http", "", "original\n"), ("from-explicit-head", "HEAD", "new HEAD\n")] {
        let catalog: ProjectCatalog = host
            .client
            .post(&endpoint)
            .bearer_auth(TOKEN)
            .json(&json!({"branch":branch,"base":base}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let tree = catalog
            .projects
            .iter()
            .flat_map(|p| &p.workspaces)
            .find(|w| w.branch.as_deref() == Some(branch))
            .unwrap();
        assert_eq!(std::fs::read_to_string(Path::new(&tree.path).join("tracked.txt"))?, expected);
    }
    // A syntactically valid default may be absent in a different project: fail
    // without silently using HEAD.
    host.call(TOKEN, "environment_preferences_update", json!({"default_worktree_base":"missing-revision"}))
        .await?;
    let result = host
        .call(TOKEN, "worktree_create", json!({"project":project.id,"branch":"must-not-exist"}))
        .await?;
    assert_eq!(result["result"]["isError"], true);
    let catalog = host.call(TOKEN, "project_list", json!({})).await?;
    assert!(!catalog.to_string().contains("must-not-exist"));
    assert_eq!(std::fs::read_to_string(root.join("tracked.txt"))?, "new HEAD\n");
    host.stop().await
}

#[tokio::test]
async fn scoped_writers_cannot_change_host_preferences() -> Result<()> {
    // Given every provider's read-only and write-capable scoped sessions.
    let host = Host::start().await?;
    let path = host.catalog.projects[0].workspaces[0].path.clone();
    for provider in gpui_agent_contracts::PROVIDERS {
        for can_write in [false, true] {
            let lease = host.sessions.issue(
                McpScope {
                    provider,
                    account_id: "default".into(),
                    workspace_path: path.clone(),
                },
                can_write,
            )?;
            let credential = &lease.connection().token;
            // When tools are discovered or called directly, then host-wide
            // writes stay operator-only.
            let tools = host.rpc(credential, "tools/list", json!({})).await?;
            assert!(
                !tools["result"]["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["name"] == "environment_preferences_update")
            );
            assert_eq!(
                host.call(credential, "environment_preferences_update", json!({"label":"unauthorized"})).await?["error"]["code"],
                -32001
            );
            assert_eq!(
                host.client
                    .patch(format!("{}/v1/environment/preferences", host.url.trim_end_matches("/mcp")))
                    .bearer_auth(credential)
                    .json(&json!({"label":"unauthorized"}))
                    .send()
                    .await?
                    .status(),
                401
            );
        }
    }
    assert_eq!(
        host.call(TOKEN, "environment_read", json!({})).await?["result"]["structuredContent"]["preferences"]["label"],
        "gpui-agent"
    );
    host.stop().await
}

#[tokio::test]
async fn operator_patches_preserve_other_preferences_and_survive_reopening_the_store() -> Result<()> {
    // Given an existing environment identity.
    let host = Host::start().await?;
    let original = host.call(TOKEN, "environment_read", json!({})).await?;
    // When settings are patched through MCP and HTTP, then omitted fields are
    // preserved.
    let changed = host.call(TOKEN, "environment_preferences_update", json!({"label":"Build host 한글"})).await?;
    assert_eq!(changed["result"]["structuredContent"]["label"], "Build host 한글");
    let endpoint = format!("{}/v1/environment/preferences", host.url.trim_end_matches("/mcp"));
    let changed: Value = host
        .client
        .patch(&endpoint)
        .bearer_auth(TOKEN)
        .json(&json!({"default_worktree_base":"baseline"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(changed["label"], "Build host 한글");
    assert_eq!(changed["default_worktree_base"], "baseline");
    // A fresh adapter at the same home observes the saved settings and same
    // identity.
    let reopened = reopen(&host, "home")?.environment_read().await?;
    assert_eq!(reopened.environment_id, original["result"]["structuredContent"]["environment_id"]);
    assert_eq!(reopened.preferences.label, "Build host 한글");
    assert_eq!(reopened.preferences.default_worktree_base, "baseline");
    // Invalid and unknown fields cannot partly update persisted preferences.
    for patch in [
        json!({}),
        json!({"label":" ","default_worktree_base":"HEAD"}),
        json!({"label":"replace","default_worktree_base":"-unsafe"}),
        json!({"label":"replace","home":"elsewhere"}),
        json!({"default_worktree_base":"HEAD\n"}),
    ] {
        assert!(
            host.client
                .patch(&endpoint)
                .bearer_auth(TOKEN)
                .json(&patch)
                .send()
                .await?
                .status()
                .is_client_error()
        );
    }
    let current = host.call(TOKEN, "environment_read", json!({})).await?;
    assert_eq!(current["result"]["structuredContent"]["preferences"], changed);
    host.stop().await
}

#[tokio::test]
async fn independent_adapters_merge_concurrent_partial_updates() -> Result<()> {
    // Given independent adapters on the same persistent environment.
    let host = Host::start().await?;
    let other = reopen(&host, "home")?;
    let patch = gpui_agent_contracts::EnvironmentPreferencesPatch {
        label: None,
        default_worktree_base: Some("origin/main".into()),
    };
    // When disjoint fields change concurrently through MCP and the application
    // seam.
    let (one, two) = tokio::join!(
        host.call(TOKEN, "environment_preferences_update", json!({"label":"Concurrent host"})),
        other.environment_preferences_update(None, patch)
    );
    assert_ne!(one?["result"]["isError"], true);
    two?;
    // Then neither update is lost, even without a shared in-memory service
    // lock.
    let info = other.environment_read().await?;
    assert_eq!(info.preferences.label, "Concurrent host");
    assert_eq!(info.preferences.default_worktree_base, "origin/main");
    host.stop().await
}
