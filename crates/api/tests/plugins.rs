use serde_json::{Value,
                 json};

#[tokio::test]
async fn given_a_bundle_when_imported_enabled_disabled_and_removed_then_guidance_and_legacy_bytes_follow_the_catalog() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    std::fs::create_dir(&repo)?;
    let settings = gpui_agent_infrastructure::Settings::discover(repo, Some(root.path().join("home")))?;
    std::fs::create_dir_all(settings.home.join("codex"))?;
    let legacy = b"[ ]\n";
    std::fs::write(settings.home.join("codex/mcp.json"), legacy)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let token = "pluginsfixturetoken123456789012345678";
    let api = gpui_agent_api::Api::new(settings.application()?, token.into())?.with_mcp_read_token("pluginreadonlytoken123456789012345678".into())?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let bundle = json!({"scope":"user","files":{
        "plugin.json":br#"{"name":"review","description":"Review tools"}"#.to_vec(),
        "skills/check/SKILL.md":b"---\ndescription: Check changes\n---\nRun checks".to_vec(),
        "skills/check/references/check.md":b"Check everything".to_vec()
    }});
    assert_eq!(client.post(format!("{base}/v1/plugins/import")).json(&bundle).send().await?.status(), 401);
    assert_eq!(
        client
            .get(format!("{base}/v1/plugins"))
            .bearer_auth("pluginreadonlytoken123456789012345678")
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        client
            .post(format!("{base}/v1/plugins/import"))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(r#"{"scope":"user","files":{"plugin.json":[],"plugin.json":[]}}"#)
            .send()
            .await?
            .status(),
        422
    );
    let response = client.post(format!("{base}/v1/plugins/import")).bearer_auth(token).json(&bundle).send().await?;
    assert_eq!(response.status(), 200);
    let imported: Value = response.json().await?;
    assert_eq!(imported["plugins"][0]["id"], "user:review");
    assert_eq!(imported["plugins"][0]["enabled"], false);
    assert_eq!(
        client
            .post(format!("{base}/v1/plugins/import"))
            .bearer_auth(token)
            .json(&bundle)
            .send()
            .await?
            .status(),
        400
    );
    for enabled in [true, false] {
        let catalog: Value = client
            .put(format!("{base}/v1/plugins/enabled"))
            .bearer_auth(token)
            .json(&json!({"id":"user:review","enabled":enabled}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(catalog["plugins"][0]["enabled"], enabled);
        let guidance: Value = client
            .get(format!("{base}/v1/guidance"))
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(guidance["skills"].as_array().unwrap().len(), usize::from(enabled));
        if enabled {
            let id = guidance["skills"][0]["id"].as_str().unwrap();
            let resource: Value = client
                .post(format!("{base}/v1/skills/read"))
                .bearer_auth(token)
                .json(&json!({"id":id,"resource":"references/check.md"}))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            assert_eq!(resource["text"], "Check everything");
        }
    }
    let removed: Value = client
        .post(format!("{base}/v1/plugins/remove"))
        .bearer_auth(token)
        .json(&json!({"id":"user:review"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(removed["plugins"].as_array().unwrap().is_empty());
    assert_eq!(std::fs::read(settings.home.join("codex/mcp.json"))?, legacy);
    let servers = json!([{"name":"review","url":"https://review.invalid/mcp"}]);
    let manifest = json!({"name":"review","description":"Review fixture","mcp_servers":servers});
    client
        .post(format!("{base}/v1/plugins/import"))
        .bearer_auth(token)
        .json(&json!({"scope":"repo","files":{"plugin.json":serde_json::to_vec(&manifest)?}}))
        .send()
        .await?
        .error_for_status()?;
    let enable = |body: Value| client.put(format!("{base}/v1/plugins/enabled")).bearer_auth(token).json(&body).send();
    assert_eq!(enable(json!({"id":"repo:review","enabled":true})).await?.status(), 400);
    let updated = json!([{"name":"review","url":"https://updated.invalid/mcp"}]);
    let mut manifest = manifest;
    manifest["mcp_servers"] = updated.clone();
    std::fs::write(settings.cwd.join(".agents/plugins/review/plugin.json"), serde_json::to_vec(&manifest)?)?;
    assert_eq!(enable(json!({"id":"repo:review","enabled":true,"mcp_servers":servers})).await?.status(), 400);
    let catalog: Value = enable(json!({"id":"repo:review","enabled":true,"mcp_servers":updated}))
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(catalog["plugins"][0]["enabled"], true);
    let _ = stop.send(());
    host.await??;
    Ok(())
}

#[tokio::test]
async fn given_claude_installed_plugins_when_synced_over_http_then_they_are_imported_disabled() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    std::fs::create_dir(&repo)?;
    let settings = gpui_agent_infrastructure::Settings::discover(repo, Some(root.path().join("home")))?;
    let install = settings.guidance_home.join("claude/plugins/cache/market/review/1.0.0");
    std::fs::create_dir_all(install.join(".claude-plugin"))?;
    std::fs::create_dir_all(install.join("skills/check"))?;
    std::fs::write(install.join(".claude-plugin/plugin.json"), r#"{"name":"review","description":"Review tools"}"#)?;
    std::fs::write(install.join("skills/check/SKILL.md"), "---\ndescription: Check\n---\nCheck")?;
    std::fs::write(
        settings.guidance_home.join("claude/plugins/installed_plugins.json"),
        serde_json::to_vec(&json!({"version":2,"plugins":{"review@market":[{"installPath":install}]}}))?,
    )?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let token = "pluginsfixturetoken123456789012345678";
    let api = gpui_agent_api::Api::new(settings.application()?, token.into())?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let url = format!("{base}/v1/plugins/claude");
    assert_eq!(client.post(&url).json(&json!({"scope":"user"})).send().await?.status(), 401);
    assert_eq!(client.post(&url).bearer_auth(token).json(&json!({"scope":"other"})).send().await?.status(), 400);
    let result: Value = client.post(&url).bearer_auth(token).json(&json!({"scope":"user"})).send().await?.json().await?;
    assert_eq!(result["imported"], json!(["user:review"]));
    assert_eq!(result["catalog"]["plugins"][0]["enabled"], false);
    let _ = stop.send(());
    host.await??;
    Ok(())
}
