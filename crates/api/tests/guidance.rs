use gpui_agent_contracts::*;
use gpui_agent_infrastructure::{McpSessions,
                                Settings};
use serde_json::{Value,
                 json};
use std::{fs,
          sync::Arc};

#[tokio::test]
async fn given_an_operator_when_guidance_is_created_and_edited_then_the_composer_reads_the_saved_file_and_stale_writes_fail() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo)?;
    let settings = Settings::discover(repo, Some(root.path().join("home")))?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let token = "guidanceeditorfixturetoken123456789012";
    let api = gpui_agent_api::Api::new(settings.application()?, token.into())?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    // Given a folder bundle, importing preserves resources and refuses
    // replacement.
    let bundle = json!({"id":"user:skills/copied/SKILL.md","files":{
        "SKILL.md":b"---\ndescription: Copied skill\n---\nRead references/check.md".to_vec(),
        "references/check.md":b"Run checks".to_vec(), "assets/icon.bin":vec![0u8,255,1]
    }});
    let import = || client.post(format!("{base}/v1/skills/import")).bearer_auth(token).json(&bundle).send();
    assert_eq!(import().await?.status(), 200);
    assert_eq!(import().await?.status(), 400);
    let resource: SkillResource = client
        .post(format!("{base}/v1/skills/read"))
        .bearer_auth(token)
        .json(&json!({"id":"user:skills/copied/SKILL.md","resource":"references/check.md"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(resource.text, "Run checks");
    assert_eq!(fs::read(root.path().join("home/skills/copied/assets/icon.bin"))?, [0, 255, 1]);
    let write = |text: &str, revision: Option<&str>| {
        client
            .put(format!("{base}/v1/guidance/files"))
            .bearer_auth(token)
            .json(&json!({"id":"repo:AGENTS.md","text":text,"expected_revision":revision}))
            .send()
    };
    let saved: Value = write("Use public tests", None).await?.error_for_status()?.json().await?;
    let snapshot: GuidanceSnapshot = client
        .get(format!("{base}/v1/guidance"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(snapshot.instructions[0].content, "Use public tests");
    let revision = saved["revision"].as_str().unwrap();
    write("Use BDD tests", Some(revision)).await?.error_for_status()?;
    assert_eq!(write("stale", Some(revision)).await?.status(), 400);
    let loaded: Value = client
        .post(format!("{base}/v1/guidance/files/read"))
        .bearer_auth(token)
        .json(&json!({"id":"repo:AGENTS.md"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(loaded["text"], "Use BDD tests");
    // Given an uploaded manifest, when saved through the same editor endpoint,
    // then the existing composer catalog and selection preparation see it.
    let skill_id = "repo:.agents/skills/uploaded/SKILL.md";
    let manifest = "---\nname: uploaded\ndescription: Verify uploaded guidance\n---\nRun public tests";
    client
        .put(format!("{base}/v1/guidance/files"))
        .bearer_auth(token)
        .json(&json!({"id":skill_id,"text":manifest,"expected_revision":null}))
        .send()
        .await?
        .error_for_status()?;
    let snapshot: GuidanceSnapshot = client
        .get(format!("{base}/v1/guidance"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(snapshot.select_skills("$uploaded", &[]).unwrap().len(), 1);
    let files: GuidanceFileCatalog = client
        .get(format!("{base}/v1/guidance/files"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(files.files, [skill_id, "repo:AGENTS.md", "user:skills/copied/SKILL.md"]);
    for (id, text) in [
        ("repo:../AGENTS.md", "escape"),
        ("repo:other/AGENTS.md", "outside cwd"),
        ("user:skills/broken/SKILL.md", "invalid YAML"),
        ("repo:.git/AGENTS.md", "git metadata"),
    ] {
        assert_eq!(
            client
                .put(format!("{base}/v1/guidance/files"))
                .bearer_auth(token)
                .json(&json!({"id":id,"text":text,"expected_revision":null}))
                .send()
                .await?
                .status(),
            400
        );
    }
    assert_eq!(
        client
            .put(format!("{base}/v1/guidance/files"))
            .json(&json!({"id":"user:AGENTS.md","text":"unauthorized"}))
            .send()
            .await?
            .status(),
        401
    );
    let _ = stop.send(());
    host.await??;
    Ok(())
}

#[tokio::test]
async fn given_no_provider_installation_when_clients_read_shared_guidance_then_http_and_scoped_mcp_use_the_same_host_policy() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    fs::create_dir_all(repo.join(".agents/skills/verify/references"))?;
    fs::write(repo.join("AGENTS.md"), "Use public tests")?;
    fs::write(repo.join(".agents/skills/verify/SKILL.md"), "---\ndescription: Verify changes\n---\nRun tests")?;
    fs::write(repo.join(".agents/skills/verify/references/check.txt"), "Supporting material")?;
    let settings = Settings::discover(repo.clone(), Some(root.path().join("home")))?;
    let app = settings.application()?;
    let workspaces = settings.workspaces(app.clone())?;
    let workspace = workspaces.catalog().await?.projects[0].workspaces[0].clone();
    let jobs = settings.jobs(workspaces.clone())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let sessions = Arc::new(McpSessions::new(format!("{base}/mcp"))?);
    let grant = sessions.issue_in_conversation(
        McpScope {
            provider: Provider::Antigravity,
            account_id: "default".into(),
            workspace_path: workspace.path,
        },
        false,
        None,
    )?;
    let token = "guidanceoperatorfixturetoken123456789012";
    let api = gpui_agent_api::Api::new(app, token.into())?
        .with_workspaces(workspaces)
        .with_jobs(jobs)
        .with_mcp_sessions(sessions);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    assert_eq!(client.get(format!("{base}/v1/skills")).send().await?.status(), 401);
    for provider in ["claude", "codex", "antigravity"] {
        let snapshot: GuidanceSnapshot = client
            .get(format!("{base}/v1/guidance"))
            .bearer_auth(token)
            .header("X-KND-Provider", provider)
            .header("X-KND-Workspace", workspace.id)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(snapshot.instructions[0].content, "Use public tests");
        assert_eq!(snapshot.skills.len(), 1);
    }
    let catalog: Value = client
        .get(format!("{base}/v1/skills"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let id = catalog["skills"][0]["id"].as_str().unwrap();
    let read = |arguments| {
        client
            .post(format!("{base}/mcp"))
            .header("Accept", "application/json, text/event-stream")
            .bearer_auth(&grant.connection().token)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"host_skills_read","arguments":arguments}}))
            .send()
    };
    let reply: Value = read(json!({"id":id,"resource":"references/check.txt"})).await?.json().await?;
    assert_eq!(reply["result"]["structuredContent"]["text"], "Supporting material");
    let denied: Value = read(json!({"id":id,"resource":"../../AGENTS.md"})).await?.json().await?;
    assert_eq!(denied["result"]["isError"], true);
    let unknown: Value = read(json!({"id":"repo:.agents/skills/private/SKILL.md"})).await?.json().await?;
    assert_eq!(unknown["result"]["isError"], true);
    assert_eq!(
        client
            .put(format!("{base}/v1/skills/enabled"))
            .bearer_auth(&grant.connection().token)
            .json(&json!({"id":id,"enabled":false}))
            .send()
            .await?
            .status(),
        401
    );
    for route in ["/v1/guidance/files", "/v1/guidance/files/read"] {
        assert_eq!(
            client
                .post(format!("{base}{route}"))
                .bearer_auth(&grant.connection().token)
                .json(&json!({"id":"repo:AGENTS.md"}))
                .send()
                .await?
                .status(),
            401
        );
    }
    client
        .put(format!("{base}/v1/skills/enabled"))
        .bearer_auth(token)
        .json(&json!({"id":id,"enabled":false}))
        .send()
        .await?
        .error_for_status()?;
    let denied: Value = read(json!({"id":id})).await?.json().await?;
    assert_eq!(denied["result"]["isError"], true);
    let _ = stop.send(());
    host.await??;
    Ok(())
}

#[tokio::test]
async fn given_an_agent_skill_proposal_when_the_operator_decides_then_only_an_approved_workspace_skill_is_installed() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo)?;
    let settings = Settings::discover(repo.clone(), Some(root.path().join("home")))?;
    let app = settings.application()?;
    let workspaces = settings.workspaces(app.clone())?;
    let workspace = workspaces.catalog().await?.projects[0].workspaces[0].clone();
    let jobs = settings.jobs(workspaces.clone())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let sessions = Arc::new(McpSessions::new(format!("{base}/mcp"))?);
    let grant = sessions.issue_in_conversation(
        McpScope {
            provider: Provider::Antigravity,
            account_id: "default".into(),
            workspace_path: workspace.path.clone(),
        },
        true,
        None,
    )?;
    let readonly = sessions.issue_in_conversation(
        McpScope {
            provider: Provider::Antigravity,
            account_id: "default".into(),
            workspace_path: workspace.path.clone(),
        },
        false,
        None,
    )?;
    let token = "skillproposaloperatortoken1234567890123";
    let api = gpui_agent_api::Api::new(app, token.into())?
        .with_workspaces(workspaces)
        .with_jobs(jobs)
        .with_mcp_sessions(sessions);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let call = |name: &'static str, arguments: Value| {
        let client = client.clone();
        let url = format!("{base}/mcp");
        let bearer = grant.connection().token.clone();
        async move {
            let reply: Value = client
                .post(url)
                .header("Accept", "application/json, text/event-stream")
                .bearer_auth(bearer)
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}))
                .send()
                .await?
                .json()
                .await?;
            anyhow::Ok(reply["result"].clone())
        }
    };
    let decide = |id: &str, allow: bool| {
        client
            .post(format!("{base}/v1/skill-proposals/{id}"))
            .bearer_auth(token)
            .json(&json!({"allow":allow}))
            .send()
    };
    // A read-only session cannot even create a proposal.
    let denied: Value = client
        .post(format!("{base}/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .bearer_auth(&readonly.connection().token)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"host_skill_propose","arguments":{"name":"nope","content":"---\ndescription: No\n---\nBody"}}}))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(denied["error"]["code"], -32001, "{denied}");
    let manifest = "---\nname: review\ndescription: Review a change\n---\nRead references/checks.md first";
    let proposal = json!({"name":"review","content":manifest,"resources":{"references/checks.md":"Run public tests"}});

    // Given a proposal, when nothing has been approved, then no skill exists.
    let proposed = call("host_skill_propose", proposal.clone()).await?;
    assert_eq!(proposed["structuredContent"]["status"], "pending");
    let id = proposed["structuredContent"]["id"].as_str().unwrap().to_owned();
    assert!(!repo.join(".agents/skills/review").exists());
    let listed = call("host_skills_list", json!({})).await?;
    assert_eq!(listed["structuredContent"]["skills"].as_array().unwrap().len(), 0);

    // The agent session cannot approve its own proposal.
    let forged = client
        .post(format!("{base}/v1/skill-proposals/{id}"))
        .bearer_auth(&grant.connection().token)
        .json(&json!({"allow":true}))
        .send()
        .await?;
    assert_eq!(forged.status(), 401);

    // When the operator approves, then the workspace skill is installed and
    // readable.
    assert_eq!(decide(&id, true).await?.status(), 200);
    assert_eq!(decide(&id, true).await?.status(), 409);
    let status = call("host_skill_proposal_status", json!({"id":id})).await?;
    assert_eq!(status["structuredContent"]["status"], "applied");
    let listed = call("host_skills_list", json!({})).await?;
    let skill = listed["structuredContent"]["skills"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(skill, "repo:.agents/skills/review/SKILL.md");
    let resource = call("host_skills_read", json!({"id":skill,"resource":"references/checks.md"})).await?;
    assert_eq!(resource["structuredContent"]["text"], "Run public tests");

    // Given a rejected proposal, then nothing is written.
    let other = json!({"name":"other","content":"---\ndescription: Other\n---\nBody"});
    let rejected = call("host_skill_propose", other).await?;
    let other_id = rejected["structuredContent"]["id"].as_str().unwrap().to_owned();
    assert_eq!(decide(&other_id, false).await?.status(), 200);
    let status = call("host_skill_proposal_status", json!({"id":other_id})).await?;
    assert_eq!(status["structuredContent"]["status"], "rejected");
    assert!(!repo.join(".agents/skills/other").exists());

    // Unsafe names, an existing skill and an invalid manifest never become
    // proposals.
    for bad in [
        json!({"name":"../escape","content":manifest}),
        json!({"name":"nested/name","content":manifest}),
        json!({"name":"broken","content":"no frontmatter"}),
        json!({"name":"res","content":manifest,"resources":{"../AGENTS.md":"x"}}),
    ] {
        assert_eq!(call("host_skill_propose", bad).await?["isError"], true);
    }
    let duplicate = call("host_skill_propose", proposal).await?;
    let duplicate_id = duplicate["structuredContent"]["id"].as_str().unwrap().to_owned();
    assert_eq!(decide(&duplicate_id, true).await?.status(), 400);
    let status = call("host_skill_proposal_status", json!({"id":duplicate_id})).await?;
    assert_eq!(status["structuredContent"]["status"], "failed");

    let _ = stop.send(());
    host.await??;
    Ok(())
}

#[tokio::test]
async fn given_an_agent_mcp_proposal_when_the_operator_decides_then_only_an_approved_server_becomes_an_enabled_workspace_plugin() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo)?;
    let settings = Settings::discover(repo.clone(), Some(root.path().join("home")))?;
    let app = settings.application()?;
    let workspaces = settings.workspaces(app.clone())?;
    let workspace = workspaces.catalog().await?.projects[0].workspaces[0].clone();
    let jobs = settings.jobs(workspaces.clone())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let sessions = Arc::new(McpSessions::new(format!("{base}/mcp"))?);
    let grant = sessions.issue_in_conversation(
        McpScope {
            provider: Provider::Antigravity,
            account_id: "default".into(),
            workspace_path: workspace.path,
        },
        true,
        None,
    )?;
    let token = "mcpproposaloperatortoken12345678901234";
    let api = gpui_agent_api::Api::new(app, token.into())?
        .with_workspaces(workspaces)
        .with_jobs(jobs)
        .with_mcp_sessions(sessions);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let call = |name: &'static str, arguments: Value| {
        let client = client.clone();
        let url = format!("{base}/mcp");
        let bearer = grant.connection().token.clone();
        async move {
            let reply: Value = client
                .post(url)
                .header("Accept", "application/json, text/event-stream")
                .bearer_auth(bearer)
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}))
                .send()
                .await?
                .json()
                .await?;
            anyhow::Ok(reply["result"].clone())
        }
    };
    let decide = |id: &str, allow: bool| {
        client
            .post(format!("{base}/v1/mcp-proposals/{id}"))
            .bearer_auth(token)
            .json(&json!({"allow":allow}))
            .send()
    };
    let plugins = || async {
        let catalog: PluginCatalog = client
            .get(format!("{base}/v1/plugins"))
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        anyhow::Ok(catalog)
    };
    let executable = std::env::current_exe()?.to_string_lossy().into_owned();
    let server = json!({"name":"fixture_tools","command":executable,"args":["--serve"]});
    let proposal = json!({"server":server,"rationale":"Needed to inspect fixtures"});

    // Given a proposal, when nothing has been approved, then no plugin or MCP
    // server exists.
    let proposed = call("host_mcp_propose", proposal.clone()).await?;
    assert_eq!(proposed["structuredContent"]["status"], "pending");
    assert_eq!(proposed["structuredContent"]["plugin_id"], "repo:mcp-fixture-tools");
    let id = proposed["structuredContent"]["id"].as_str().unwrap().to_owned();
    assert!(plugins().await?.plugins.is_empty());
    assert!(!repo.join(".agents/plugins").exists());

    // The agent session cannot approve its own proposal.
    let forged = client
        .post(format!("{base}/v1/mcp-proposals/{id}"))
        .bearer_auth(&grant.connection().token)
        .json(&json!({"allow":true}))
        .send()
        .await?;
    assert_eq!(forged.status(), 401);

    // When the operator approves, then the server is an enabled plugin in this
    // workspace only.
    assert_eq!(decide(&id, true).await?.status(), 200);
    assert_eq!(decide(&id, true).await?.status(), 409);
    let status = call("host_mcp_proposal_status", json!({"id":id})).await?;
    assert_eq!(status["structuredContent"]["status"], "applied");
    let catalog = plugins().await?;
    assert_eq!(catalog.plugins.len(), 1);
    let plugin = &catalog.plugins[0];
    assert_eq!(plugin.id, "repo:mcp-fixture-tools");
    assert!(plugin.enabled);
    assert_eq!(plugin.manifest.mcp_servers.len(), 1);
    assert_eq!(plugin.manifest.mcp_servers[0].name(), "fixture_tools");
    assert!(repo.join(".agents/plugins/mcp-fixture-tools/plugin.json").is_file());

    // Given a rejected proposal, then nothing is written.
    let other = json!({"server":{"name":"other","command":executable},"rationale":"Maybe"});
    let rejected = call("host_mcp_propose", other).await?;
    let other_id = rejected["structuredContent"]["id"].as_str().unwrap().to_owned();
    assert_eq!(decide(&other_id, false).await?.status(), 200);
    let status = call("host_mcp_proposal_status", json!({"id":other_id})).await?;
    assert_eq!(status["structuredContent"]["status"], "rejected");
    assert!(!repo.join(".agents/plugins/mcp-other").exists());

    // Servers that could not run, or that need credentials or OAuth setup,
    // never become proposals.
    for bad in [
        json!({"server":{"name":"relative","command":"tool"},"rationale":"x"}),
        json!({"server":{"name":"missing","command":"/definitely/not/installed"},"rationale":"x"}),
        json!({"server":{"name":"bad name","command":executable},"rationale":"x"}),
        json!({"server":{"name":"plain","url":"http://example.com/mcp"},"rationale":"x"}),
        json!({"server":{"name":"oauth","url":"https://example.com/mcp","oauth":{"client_id":"id"}},"rationale":"x"}),
        json!({"server":{"name":"blank","command":executable},"rationale":" "}),
    ] {
        assert_eq!(call("host_mcp_propose", bad).await?["isError"], true);
    }

    // A name that is already installed fails on approval without changing the
    // installed plugin.
    let duplicate = call("host_mcp_propose", proposal).await?;
    let duplicate_id = duplicate["structuredContent"]["id"].as_str().unwrap().to_owned();
    assert_eq!(decide(&duplicate_id, true).await?.status(), 400);
    let status = call("host_mcp_proposal_status", json!({"id":duplicate_id})).await?;
    assert_eq!(status["structuredContent"]["status"], "failed");
    assert_eq!(plugins().await?.plugins.len(), 1);

    let _ = stop.send(());
    host.await??;
    Ok(())
}

#[tokio::test]
async fn given_an_http_mcp_proposal_when_its_environment_is_not_ready_then_it_is_refused_and_no_secret_value_is_ever_returned() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo)?;
    let settings = Settings::discover(repo.clone(), Some(root.path().join("home")))?;
    let app = settings.application()?;
    let workspaces = settings.workspaces(app.clone())?;
    let workspace = workspaces.catalog().await?.projects[0].workspaces[0].clone();
    let jobs = settings.jobs(workspaces.clone())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let sessions = Arc::new(McpSessions::new(format!("{base}/mcp"))?);
    let grant = sessions.issue_in_conversation(
        McpScope {
            provider: Provider::Antigravity,
            account_id: "default".into(),
            workspace_path: workspace.path,
        },
        true,
        None,
    )?;
    let token = "mcpenvironmentoperatortoken1234567890123";
    let api = gpui_agent_api::Api::new(app, token.into())?
        .with_workspaces(workspaces)
        .with_jobs(jobs)
        .with_mcp_sessions(sessions);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let call = |arguments: Value| {
        let client = client.clone();
        let url = format!("{base}/mcp");
        let bearer = grant.connection().token.clone();
        async move {
            let reply = client
                .post(url)
                .header("Accept", "application/json, text/event-stream")
                .bearer_auth(bearer)
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"host_mcp_propose","arguments":arguments}}))
                .send()
                .await?
                .text()
                .await?;
            anyhow::Ok(reply)
        }
    };
    // PATH is set on every test host; its value stands in for a secret that
    // must never be echoed.
    let secret = std::env::var("PATH")?;
    let ready = json!({"server":{"name":"remote","url":"https://example.com/mcp","bearer_token_env_var":"PATH"},"rationale":"Needs the API"});
    let unset = "GPUI_AGENT_PROPOSAL_TEST_UNSET";
    let mut transcript = String::new();

    // Given a variable the host does not have, then the proposal is refused and
    // names the variable, not a value.
    let refused = call(json!({"server":{"name":"remote","url":"https://example.com/mcp","bearer_token_env_var":unset},"rationale":"Needs the API"})).await?;
    transcript += &refused;
    let refused: Value = serde_json::from_str(&refused)?;
    assert_eq!(refused["result"]["isError"], true);
    let message = refused["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(message.contains(unset) && message.contains("operator"), "{message}");
    let header_refused =
        call(json!({"server":{"name":"remote","url":"https://example.com/mcp","env_http_headers":{"X-Key":unset}},"rationale":"Needs the API"})).await?;
    transcript += &header_refused;
    assert!(header_refused.contains("\"isError\":true"));

    // OAuth needs an interactive login, so the agent is told to leave it to the
    // operator.
    let oauth = call(json!({"server":{"name":"oauth","url":"https://example.com/mcp","oauth":{"scopes":["read"]}},"rationale":"Needs the API"})).await?;
    transcript += &oauth;
    let oauth: Value = serde_json::from_str(&oauth)?;
    assert_eq!(oauth["result"]["isError"], true);
    let message = oauth["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(message.contains("MCP settings") && message.contains("login"), "{message}");

    // Given a variable the host has, the proposal lists only readiness for the
    // operator.
    let proposed = call(ready).await?;
    transcript += &proposed;
    let proposed: Value = serde_json::from_str(&proposed)?;
    let id = proposed["result"]["structuredContent"]["id"].as_str().unwrap().to_owned();
    let pending = client
        .get(format!("{base}/v1/mcp-proposals"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    transcript += &pending;
    let pending: Value = serde_json::from_str(&pending)?;
    assert_eq!(pending[0]["environment"], json!([{"variable":"PATH","ready":true}]));
    let approved = client
        .post(format!("{base}/v1/mcp-proposals/{id}"))
        .bearer_auth(token)
        .json(&json!({"allow":true}))
        .send()
        .await?;
    assert_eq!(approved.status(), 200);
    transcript += &approved.text().await?;
    let manifest = fs::read_to_string(repo.join(".agents/plugins/mcp-remote/plugin.json"))?;
    transcript += &manifest;
    assert!(manifest.contains("\"bearer_token_env_var\": \"PATH\""));

    // Neither responses nor the installed manifest ever carry the variable's
    // value.
    assert!(!transcript.contains(&secret), "A secret environment value leaked");

    let _ = stop.send(());
    host.await??;
    Ok(())
}
