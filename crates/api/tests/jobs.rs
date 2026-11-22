extern crate gpui_agent_contracts as gpui_agent_domain;
#[path = "support/client_fixture.rs"]
mod client_fixture;
#[path = "support/computer.rs"]
mod computer;
#[path = "support/guidance.rs"]
mod guidance;
use guidance::shared_guidance_jobs;
#[path = "support/live_computer.rs"]
mod live_computer;
#[allow(dead_code)]
#[path = "../../agent/tests/protocols.rs"]
mod native;
#[path = "support/profile_contention.rs"]
mod profile_contention;
use anyhow::{Context,
             Result,
             ensure};
use gpui_agent_application::{Application,
                             ports::JobRepository};
use gpui_agent_contracts::*;
use gpui_agent_infrastructure::{JobStore,
                                Projects,
                                Settings};
use reqwest::{Method,
              StatusCode};
use serde_json::json;
use std::{fs,
          path::Path,
          sync::Arc,
          time::Duration};
const TOKEN: &str = "jobintegrationtokenlongenough1234567890123456789";
const MCP_READ_TOKEN: &str = "mcpreadonlyintegrationtoken1234567890123456789";
fn application_with_mcp(
    home: &Path,
    cwd: &Path,
    provider: Provider,
    sessions: Option<(&Arc<gpui_agent_infrastructure::McpSessions>, &str)>,
) -> Result<Application> {
    let config = gpui_agent::Config {
        mcp_home: Some(home.to_path_buf()),
        mcp_selection: None,
        plugin_mcp: vec![],
        provider,
        home: home.join(provider_name(provider)),
        cwd: cwd.into(),
        executable: std::env::current_exe()?,
        harness: Some(std::env::current_exe()?),
        browser_helper: Some(std::env::current_exe()?),
        timeout: if std::env::args().any(|arg| arg == "--computer-fixture-host") {
            Duration::from_secs(600)
        } else if fs::read_to_string(cwd.join("fixture.json")).is_ok_and(|spec| spec.contains("\"timeout\"")) {
            Duration::from_millis(300)
        } else {
            Duration::from_secs(30)
        },
    };
    gpui_agent_infrastructure::application_from_agents(
        PROVIDERS.map(|candidate| {
            let agent = if cwd.join("all-providers").exists() {
                gpui_agent::Agent::new(gpui_agent::Config {
                    provider: candidate,
                    home: home.join(provider_name(candidate)),
                    ..config.clone()
                })
            } else if candidate == provider {
                gpui_agent::Agent::new(config.clone())
            } else {
                Err(anyhow::anyhow!("fixture unavailable"))
            };
            agent.map(|agent| {
                if let Some((sessions, account)) = sessions {
                    agent.with_mcp_sessions(sessions.for_scope(
                        McpScope {
                            provider: candidate,
                            account_id: account.into(),
                            workspace_path: cwd.to_string_lossy().into_owned(),
                        },
                        false,
                    ))
                } else {
                    agent
                }
            })
        }),
        &home.join("history.sqlite3"),
        cwd,
    )
}
struct Server {
    diagnostics: Arc<std::sync::Mutex<Vec<McpDiagnostic>>>,
    sessions: Arc<gpui_agent_infrastructure::McpSessions>,
    root: tempfile::TempDir,
    url: String,
    client: reqwest::Client,
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl Server {
    async fn open(provider: Provider) -> Result<Self> { Self::open_scenario(provider, "interactive-allow").await }

    async fn open_scenario(provider: Provider, scenario: &str) -> Result<Self> {
        Self::open_controls(provider, scenario, Arc::new(client_fixture::NoControls)).await
    }

    async fn open_controls(provider: Provider, scenario: &str, controls: Arc<dyn gpui_agent_application::ports::HostControl>) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let cwd = root.path().join("project 한글");
        fs::create_dir(&cwd)?;
        fs::write(
            cwd.join("fixture.json"),
            json!({"provider":if scenario=="workflow"{"auto"}else{provider_name(provider)},"scenario":if matches!(scenario,"workflow"|"mcp-session"){"interactive-allow"}else{scenario},"probeMcp":scenario=="mcp-session"}).to_string(),
        )?;
        if scenario == "workflow" {
            fs::write(cwd.join("all-providers"), "")?;
        }
        let settings = Settings::discover(cwd.clone(), Some(root.path().join("home")))?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let sessions = Arc::new(gpui_agent_infrastructure::McpSessions::new(format!("{url}/mcp"))?);
        let automatic_computer = controls.automatic_computer_enabled();
        let app = application_with_mcp(&settings.home, &cwd, provider, Some((&sessions, "default")))?.with_automatic_computer(automatic_computer);
        let other = settings.clone();
        let workspace_sessions = sessions.clone();
        let workspaces = Projects::open(settings.clone(), app.clone())?
            .with_application_factory(move |cwd| application_with_mcp(&other.home, cwd, provider, Some((&workspace_sessions, "default"))))
            .service();
        let factory = settings.clone();
        let account_sessions = sessions.clone();
        let jobs = JobStore::open(settings, workspaces.clone())?
            .with_application_factory(move |scope| {
                let home = if scope.account_id == "default" {
                    factory.home.clone()
                } else {
                    factory.home.join("accounts").join(&scope.account_id)
                };
                Ok(
                    application_with_mcp(&home, Path::new(&scope.workspace_path), provider, Some((&account_sessions, &scope.account_id)))?
                        .with_automatic_computer(automatic_computer),
                )
            })
            .service(workspaces.clone());
        let api = gpui_agent_api::Api::new(app, TOKEN.into())?
            .with_controls(controls)
            .with_mcp_sessions(sessions.clone())
            .with_mcp_read_token(MCP_READ_TOKEN.into())?
            .with_workspaces(workspaces)
            .with_jobs(jobs);
        let diagnostics = Arc::new(std::sync::Mutex::new(Vec::new()));
        struct Sink(Arc<std::sync::Mutex<Vec<McpDiagnostic>>>);
        impl gpui_agent_application::ports::McpDiagnosticSink for Sink {
            fn record(&self, record: &McpDiagnostic) { self.0.lock().unwrap().push(record.clone()); }
        }
        let api = api.with_mcp_diagnostics(Arc::new(Sink(diagnostics.clone())));
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
            let _ = rx.await;
        }));
        Ok(Self {
            diagnostics,
            sessions,
            root,
            url,
            client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(15)).build()?,
            shutdown: tx,
            task,
        })
    }

    fn request(&self, method: Method, path: &str, account: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.url))
            .bearer_auth(TOKEN)
            .header("X-KND-Account", account)
    }

    async fn enqueue(&self, provider: Provider, account: &str, text: &str, conversation: Option<i64>) -> Result<Job> {
        Ok(self
            .request(Method::POST, &format!("/v1/providers/{}/jobs", provider_name(provider)), account)
            .json(&json!({"text":text,"conversation_id":conversation}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn mcp(&self, name: &str, arguments: serde_json::Value, token: &str) -> Result<serde_json::Value> {
        let response = self
            .client
            .post(format!("{}/mcp", self.url))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&json!({"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":name,"arguments":arguments}}))
            .send()
            .await?
            .error_for_status()?;
        let request_id = response
            .headers()
            .get("x-request-id")
            .context("Missing correlation header")?
            .to_str()?
            .to_owned();
        let reply: serde_json::Value = response.json().await?;
        if reply["result"].get("isError").is_some() {
            ensure!(self.diagnostics.lock().unwrap().iter().any(|r| r.request_id == request_id && r.tool == name));
        }
        Ok(reply)
    }

    async fn job(&self, id: i64) -> Result<JobDetail> {
        Ok(self
            .request(Method::GET, &format!("/v1/jobs/{id}"), "default")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn wait(&self, id: i64, state: JobState) -> Result<JobDetail> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let detail = self.job(id).await?;
                if detail.job.state == state {
                    return Ok(detail);
                }
                ensure!(!detail.job.state.terminal(), "job {id}: expected {state:?}, got {:?}", detail.job);
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        })
        .await?
    }

    async fn login(&self, provider: Provider, account: &str) -> Result<()> {
        let response = self
            .request(Method::POST, &format!("/v1/providers/{}/login", provider_name(provider)), account)
            .json(&json!({}))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        ensure!(response.contains("authenticated"), "{response}");
        Ok(())
    }

    async fn pending(&self, id: i64, after: i64) -> Result<(u64, i64)> {
        let mut response = self
            .request(Method::GET, &format!("/v1/jobs/{id}/events?after={after}"), "default")
            .send()
            .await?
            .error_for_status()?;
        let mut buffer = Vec::new();
        loop {
            while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                let event: JobEvent = serde_json::from_slice(&buffer.drain(..= end).collect::<Vec<_>>())?;
                ensure!(event.sequence > after);
                if let StreamEvent::Interaction {
                    request_id, ..
                } = event.event
                {
                    return Ok((request_id, event.sequence));
                }
            }
            let Some(chunk) = response.chunk().await? else {
                let job = self.job(id).await?.job;
                anyhow::bail!(
                    "Job event stream ended before an interaction: job={id} provider={:?} state={:?} after={after} last_sequence={} buffered={}",
                    job.scope.provider,
                    job.state,
                    job.last_sequence,
                    buffer.len()
                );
            };
            buffer.extend(chunk);
        }
    }

    async fn respond(&self, id: i64, request: u64) -> Result<()> {
        self.request(Method::POST, &format!("/v1/jobs/{id}/interactions/{request}"), "default")
            .json(&json!({"type":"approval","allow":true}))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn stop(self) -> Result<()> {
        let _ = self.shutdown.send(());
        self.task.await??;
        drop(self.root);
        Ok(())
    }
}

// Given scoped history and durable turns, when an agent queries or controls a
// conversation, then paging, queue semantics and scope isolation are preserved.
async fn finish_attachment_job(server: &Server, id: i64) -> Result<JobDetail> {
    let (approval, _) = server.pending(id, 0).await?;
    server
        .request(Method::POST, &format!("/v1/jobs/{id}/interactions/{approval}"), "default")
        .json(&json!({"type":"approval","allow":false}))
        .send()
        .await?
        .error_for_status()?;
    server.wait(id, JobState::Completed).await
}
async fn mcp_inline_attachments() -> Result<()> {
    for provider in PROVIDERS {
        let server = Server::open_scenario(provider, "attachments").await?;
        server.login(provider, "default").await?;
        let catalog: ProjectCatalog = server
            .request(Method::GET, "/v1/projects", "default")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let scope = McpScope {
            provider,
            account_id: "default".into(),
            workspace_path: catalog.projects[0].workspaces[0].path.clone(),
        };
        let writer = server.sessions.issue(scope.clone(), true)?;
        let reader = server.sessions.issue(scope, false)?;
        let attachments = json!([
            {"type":"text","name":"sample.rs","text":"fn main() { println!(\"첨부 🦀\"); }\n","source":null},
            {"type":"image","name":"pixel.png","mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="}
        ]);
        let command = json!({"provider":provider,"text":"","attachments":attachments,"command_id":"inline-attachments"});
        let denied = server.mcp("conversation_send", command.clone(), &reader.connection().token).await?;
        ensure!(denied["error"]["code"] == -32001);
        // Given an attachment-only MCP message, when accepted, then all native
        // adapters receive its bytes.
        let accepted = server.mcp("conversation_send", command.clone(), &writer.connection().token).await?;
        ensure!(accepted["result"]["isError"] == false, "Attachment message rejected: {accepted}");
        let job: Job = serde_json::from_value(accepted["result"]["structuredContent"].clone())?;
        let done = finish_attachment_job(&server, job.id).await?;
        ensure!(json!(done.request.attachments) == attachments);
        let replay = server.mcp("conversation_send", command.clone(), &writer.connection().token).await?;
        ensure!(replay["result"]["structuredContent"]["id"] == job.id);
        let saved: SavedConversation = server
            .request(
                Method::GET,
                &format!("/v1/providers/{}/conversations/{}", provider_name(provider), done.job.conversation_id.unwrap()),
                "default",
            )
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(json!(saved.messages[0].attachments) == attachments);
        let mut changed = command.clone();
        changed["attachments"][0]["text"] = json!("different bytes");
        ensure!(server.mcp("conversation_send", changed, &writer.connection().token).await?["result"]["isError"] == true);
        let mut queued = command;
        queued["command_id"] = json!("inline-enqueue");
        let accepted = server.mcp("enqueue_job", queued, &writer.connection().token).await?;
        ensure!(accepted["result"]["isError"] == false, "enqueue attachments rejected: {accepted}");
        finish_attachment_job(&server, accepted["result"]["structuredContent"]["id"].as_i64().unwrap()).await?;
        {
            let records = server.diagnostics.lock().unwrap();
            ensure!(!json!(*records).to_string().contains("iVBOR") && !json!(*records).to_string().contains("println"));
        }
        if provider == Provider::Codex {
            let mut raw = vec![0; attachment::MAX_IMAGE_BYTES];
            raw[.. 8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
            let large = Attachment::from_bytes("large.png".into(), &raw).map_err(anyhow::Error::msg)?;
            for invalid in [
                json!([{"type":"text","name":"../escape","text":"bad"}]),
                json!([{"type":"text","name":"large.txt","text":"x".repeat(262145)}]),
                json!([{"type":"image","name":"bad.png","mime_type":"image/png","data":"not-base64!"}]),
                json!([{"type":"file","name":"doc.pdf","data":"JVBERi0="}]),
                json!(vec![json!({"type":"text","name":"a.txt","text":"x"}); 9]),
                json!([
                    large.clone(),
                    large,
                    Attachment::Text {
                        name: "x.txt".into(),
                        text: "x".into(),
                        source: None
                    }
                ]),
            ] {
                for name in ["conversation_send", "enqueue_job"] {
                    ensure!(
                        server
                            .mcp(
                                name,
                                json!({"provider":provider,"text":"","attachments":invalid,"command_id":"invalid-attachments"}),
                                &writer.connection().token
                            )
                            .await?["result"]["isError"]
                            == true
                    );
                }
            }
        }
        server.stop().await?;
    }
    println!("ok: scoped attachment-only MCP delivery, three native payloads, durable bytes/history, retries and secret-free diagnostics");
    Ok(())
}

async fn mcp_uploaded_attachments() -> Result<()> {
    for provider in PROVIDERS {
        let server = Server::open_scenario(provider, "attachments").await?;
        server.login(provider, "default").await?;
        let catalog: ProjectCatalog = server
            .request(Method::GET, "/v1/projects", "default")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let path = catalog.projects[0].workspaces[0].path.clone();
        let scope = McpScope {
            provider,
            account_id: "default".into(),
            workspace_path: path.clone(),
        };
        let writer = server.sessions.issue(scope.clone(), true)?;
        let reader = server.sessions.issue(scope.clone(), false)?;
        let token = &writer.connection().token;
        fs::write(Path::new(&path).join("sample.rs"), "skip\nfn main() { println!(\"첨부 🦀\"); }\nskip\n")?;
        let text = server
            .mcp(
                "attachment_from_workspace",
                json!({"provider":provider,"path":"sample.rs","start_line":2,"end_line":2}),
                token,
            )
            .await?;
        ensure!(text["result"]["isError"] == false, "Workspace upload rejected: {text}");
        let text_id = text["result"]["structuredContent"]["id"].as_i64().unwrap();
        let image = json!({"type":"image","name":"pixel.png","mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="});
        let image_attachment: Attachment = serde_json::from_value(image.clone())?;
        fs::write(Path::new(&path).join("pixel.png"), image_attachment.image_bytes().map_err(anyhow::Error::msg)?)?;
        let from_disk = server
            .mcp("attachment_from_workspace", json!({"provider":provider,"path":"pixel.png"}), token)
            .await?;
        ensure!(
            from_disk["result"]["isError"] == false && from_disk["result"]["structuredContent"]["kind"] == "image",
            "Workspace image rejected: {from_disk}"
        );
        let upload = json!({"provider":provider,"attachment":image});
        ensure!(server.mcp("attachment_upload", upload.clone(), &reader.connection().token).await?["error"]["code"] == -32001);
        let image = server.mcp("attachment_upload", upload.clone(), token).await?;
        ensure!(image["result"]["isError"] == false, "Image upload rejected: {image}");
        let image_id = image["result"]["structuredContent"]["id"].as_i64().unwrap();
        ensure!(from_disk["result"]["structuredContent"]["id"] == image_id);
        ensure!(server.mcp("attachment_upload", upload, token).await?["result"]["structuredContent"]["id"] == image_id);
        let listed = server.mcp("attachment_list", json!({"provider":provider}), &reader.connection().token).await?;
        ensure!(listed["result"]["structuredContent"]["items"].as_array().unwrap().len() == 2);
        ensure!(!listed.to_string().contains("iVBOR") && !listed.to_string().contains("println"));
        for request in [
            json!({"path":"../outside"}),
            json!({"path":".git/config"}),
            json!({"path":path}),
            json!({"path":"pixel.png","start_line":1}),
            json!({"path":"sample.rs","start_line":0}),
        ] {
            let mut request = request;
            request["provider"] = json!(provider);
            ensure!(server.mcp("attachment_from_workspace", request, token).await?["result"]["isError"] == true);
        }
        ensure!(
            server
                .mcp(
                    "attachment_from_workspace",
                    json!({"provider":provider,"path":"sample.rs"}),
                    &reader.connection().token
                )
                .await?["error"]["code"]
                == -32001
        );
        ensure!(
            server
                .mcp("attachment_delete", json!({"provider":provider,"id":text_id}), &reader.connection().token)
                .await?["error"]["code"]
                == -32001
        );
        let other = server.root.path().join("other workspace");
        fs::create_dir(&other)?;
        let registered: ProjectCatalog = server
            .request(Method::POST, "/v1/projects", "default")
            .json(&json!({"path":other}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let other = registered
            .projects
            .iter()
            .flat_map(|project| &project.workspaces)
            .find(|workspace| Some(workspace.id) == registered.registered_workspace_id)
            .unwrap();
        let foreign = server.sessions.issue(
            McpScope {
                workspace_path: other.path.clone(),
                ..scope.clone()
            },
            true,
        )?;
        let foreign_token = &foreign.connection().token;
        ensure!(server.mcp("attachment_list", json!({"provider":provider}), foreign_token).await?["result"]["structuredContent"]["items"] == json!([]));
        ensure!(
            server
                .mcp("attachment_delete", json!({"provider":provider,"id":text_id}), foreign_token)
                .await?["result"]["isError"]
                == true
        );
        ensure!(
            server
                .mcp(
                    "conversation_send",
                    json!({"provider":provider,"text":"","attachment_ids":[text_id],"command_id":"foreign-upload"}),
                    foreign_token
                )
                .await?["result"]["isError"]
                == true
        );
        for forged in [
            json!({"provider":provider,"workspace":other.id}),
            json!({"provider":provider,"account":"a".repeat(32)}),
            json!({"provider":if provider==Provider::Claude {Provider::Codex} else {Provider::Claude}}),
        ] {
            ensure!(server.mcp("attachment_list", forged, token).await?["result"]["isError"] == true);
        }
        fs::write(Path::new(&path).join("sample.rs"), "changed after upload")?;
        let command = json!({"provider":provider,"text":"","attachment_ids":[text_id,image_id],"command_id":"uploaded-message"});
        let sent = server.mcp("conversation_send", command.clone(), token).await?;
        ensure!(sent["result"]["isError"] == false, "Uploaded message rejected: {sent}");
        let id = sent["result"]["structuredContent"]["id"].as_i64().unwrap();
        let done = finish_attachment_job(&server, id).await?;
        ensure!(done.request.attachments.len() == 2 && done.request.attachments[0].text_context().unwrap().contains("첨부 🦀"));
        // Deleting staged uploads cannot alter admitted jobs or break an
        // identical command retry.
        for upload_id in [text_id, image_id] {
            ensure!(server.mcp("attachment_delete", json!({"provider":provider,"id":upload_id}), token).await?["result"]["isError"] == false);
        }
        ensure!(server.mcp("conversation_send", command, token).await?["result"]["structuredContent"]["id"] == id);
        ensure!(server.job(id).await?.request.attachments == done.request.attachments);
        server.stop().await?;
    }
    println!(
        "ok: scoped workspace/text/image uploads, duplicate reuse, metadata-only listing, native delivery, immutable admitted bytes and retries after upload deletion"
    );
    Ok(())
}

async fn mcp_conversations() -> Result<()> {
    let server = Server::open(Provider::Codex).await?;
    server.login(Provider::Codex, "default").await?;
    let first = server.enqueue(Provider::Codex, "default", "needle 세계%_", None).await?;
    let (approval, _) = server.pending(first.id, 0).await?;
    server.respond(first.id, approval).await?;
    let done = server.wait(first.id, JobState::Completed).await?;
    let id = done.job.conversation_id.context("Missing conversation")?;
    let session = server.sessions.issue(
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: done.job.scope.workspace_path.clone(),
        },
        true,
    )?;
    let token = &session.connection().token;
    let listed = server.mcp("conversation_list", json!({"provider":"codex"}), token).await?;
    ensure!(listed["result"]["isError"] == false, "list failed: {listed}");
    ensure!(listed["result"]["structuredContent"]["items"][0]["id"] == id);
    let found = server.mcp("conversation_search", json!({"provider":"codex","query":"세계%_"}), token).await?;
    ensure!(
        found["result"]["structuredContent"]["items"][0]["matched_message"]["preview"]
            .as_str()
            .is_some_and(|text| text.contains("세계%_"))
    );
    let read = server.mcp("conversation_read", json!({"provider":"codex","id":id}), token).await?;
    ensure!(read["result"]["isError"] == false, "read failed: {read}");
    let page = &read["result"]["structuredContent"];
    ensure!(page["messages"][0]["text"] == "needle 세계%_");
    ensure!(page["messages"][1]["text"].as_str().is_some_and(|text| text.contains("Hello")));
    ensure!(page.get("session_id").is_none() && page["next_cursor"].is_null());
    let first_page = server.mcp("conversation_read", json!({"provider":"codex","id":id,"limit":1}), token).await?;
    ensure!(first_page["result"]["structuredContent"]["messages"].as_array().unwrap().len() == 1);
    let next = server
        .mcp(
            "conversation_read",
            json!({"provider":"codex","id":id,"cursor":first_page["result"]["structuredContent"]["next_cursor"]}),
            token,
        )
        .await?;
    ensure!(next["result"]["structuredContent"]["messages"][0]["user"] == false);
    for (tool, args) in [
        ("conversation_search", json!({"provider":"codex","query":" "})),
        ("conversation_search", json!({"provider":"codex","query":"🦀".repeat(257)})),
        ("conversation_list", json!({"provider":"codex","limit":0})),
        ("conversation_read", json!({"provider":"codex","id":id,"cursor":{"message":0,"text":999}})),
        ("conversation_read", json!({"provider":"codex","id":id,"session_id":"untrusted-session"})),
        ("conversation_read", json!({"provider":"claude","id":id})),
        (
            "conversation_read",
            json!({"provider":"codex","id":id,"account":"0123456789abcdef0123456789abcdef"}),
        ),
        ("conversation_read", json!({"provider":"codex","id":id,"workspace":i64::MAX})),
        ("conversation_wait", json!({"provider":"codex","id":id,"timeout_ms":10001})),
    ] {
        ensure!(
            server.mcp(tool, args, token).await?["result"]["isError"] == true,
            "invalid {tool} request was accepted"
        );
    }
    let reader = server.sessions.issue(
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: done.job.scope.workspace_path.clone(),
        },
        false,
    )?;
    ensure!(
        server
            .mcp("conversation_read", json!({"provider":"codex","id":id}), &reader.connection().token)
            .await?["result"]["isError"]
            == false
    );
    for (tool, args) in [
        ("conversation_rename", json!({"provider":"codex","id":id,"title":"denied"})),
        ("conversation_organize", json!({"provider":"codex","id":id,"archived":true})),
        (
            "conversation_send",
            json!({"provider":"codex","id":id,"text":"denied","command_id":"denied-conversation-send"}),
        ),
        ("conversation_interrupt", json!({"provider":"codex","id":id})),
    ] {
        ensure!(server.mcp(tool, args, &reader.connection().token).await?["error"]["code"] == -32001);
    }
    let renamed = server
        .mcp("conversation_rename", json!({"provider":"codex","id":id,"title":"Renamed conversation"}), token)
        .await?;
    ensure!(renamed["result"]["isError"] == false, "rename failed: {renamed}");
    let found = server.mcp("conversation_search", json!({"provider":"codex","query":"needle"}), token).await?;
    ensure!(found["result"]["structuredContent"]["items"][0]["title"] == "Renamed conversation");
    let organized = server
        .mcp(
            "conversation_organize",
            json!({"provider":"codex","id":id,"pinned":true,"archived":true}),
            token,
        )
        .await?;
    ensure!(organized["result"]["isError"] == false, "organize failed: {organized}");
    ensure!(server.mcp("conversation_list", json!({"provider":"codex"}), token).await?["result"]["structuredContent"]["items"] == json!([]));
    let archived = server
        .mcp("conversation_list", json!({"provider":"codex","archived":"archived"}), token)
        .await?;
    ensure!(archived["result"]["structuredContent"]["items"][0]["pinned"] == true);
    ensure!(
        server
            .mcp(
                "conversation_organize",
                json!({"provider":"codex","id":id,"pinned":false,"archived":false}),
                token
            )
            .await?["result"]["isError"]
            == false
    );
    let input = json!({"provider":"codex","id":id,"text":"follow-up message","model":"first","command_id":"conversation-follow-up"});
    let sent = server.mcp("conversation_send", input.clone(), token).await?;
    ensure!(sent["result"]["isError"] == false, "send failed: {sent}");
    let follow: Job = serde_json::from_value(sent["result"]["structuredContent"].clone())?;
    ensure!(follow.conversation_id == Some(id));
    ensure!(server.mcp("conversation_send", input, token).await?["result"]["structuredContent"]["id"] == follow.id);
    let (approval, _) = server.pending(follow.id, 0).await?;
    ensure!(server.job(follow.id).await?.request.model.as_deref() == Some("first"));
    ensure!(
        server
            .mcp("conversation_rename", json!({"provider":"codex","id":id,"title":"busy"}), token)
            .await?["result"]["isError"]
            == true
    );
    server.respond(follow.id, approval).await?;
    server.wait(follow.id, JobState::Completed).await?;
    let settled = server
        .mcp("conversation_wait", json!({"provider":"codex","id":id,"timeout_ms":0}), token)
        .await?;
    ensure!(settled["result"]["isError"] == false, "wait failed: {settled}");
    ensure!(settled["result"]["structuredContent"]["job"]["id"] == follow.id && settled["result"]["structuredContent"]["timed_out"] == false);
    let sent = server
        .mcp(
            "conversation_send",
            json!({"provider":"codex","id":id,"text":"interrupt this run","command_id":"conversation-interrupt"}),
            token,
        )
        .await?;
    let active: Job = serde_json::from_value(sent["result"]["structuredContent"].clone())?;
    server.pending(active.id, 0).await?;
    let queued = server
        .mcp(
            "conversation_send",
            json!({"provider":"codex","id":id,"text":"queued follow-up","command_id":"queued-conversation-follow"}),
            token,
        )
        .await?;
    let queued: Job = serde_json::from_value(queued["result"]["structuredContent"].clone())?;
    ensure!(queued.state == JobState::Queued);
    let listed = server.mcp("conversation_list", json!({"provider":"codex"}), token).await?;
    ensure!(
        listed["result"]["structuredContent"]["items"][0]["execution_state"] == "waiting",
        "a newer queued message must not hide the active approval wait"
    );
    let waiting = server
        .mcp("conversation_wait", json!({"provider":"codex","id":id,"timeout_ms":0}), token)
        .await?;
    ensure!(waiting["result"]["structuredContent"]["timed_out"] == true);
    let interrupted = server.mcp("conversation_interrupt", json!({"provider":"codex","id":id}), token).await?;
    ensure!(
        interrupted["result"]["structuredContent"]["job"]["id"] == active.id,
        "running work must be selected ahead of a newer queued message"
    );
    ensure!(
        server
            .mcp("conversation_interrupt", json!({"provider":"codex","id":id,"job_id":queued.id}), token)
            .await?["result"]["isError"]
            == false
    );
    ensure!(
        server
            .mcp("conversation_interrupt", json!({"provider":"codex","id":id,"job_id":active.id}), token)
            .await?["result"]["isError"]
            == false
    );
    let ended = server
        .mcp(
            "conversation_wait",
            json!({"provider":"codex","id":id,"job_id":active.id,"timeout_ms":10000}),
            token,
        )
        .await?;
    ensure!(ended["result"]["structuredContent"]["job"]["state"] == "cancelled" && ended["result"]["structuredContent"]["timed_out"] == false);
    let long_text = format!("{}끝", "🦀".repeat(16000));
    let created = server
        .mcp(
            "conversation_send",
            json!({"provider":"codex","text":long_text,"command_id":"new-long-conversation"}),
            token,
        )
        .await?;
    let created: Job = serde_json::from_value(created["result"]["structuredContent"].clone())?;
    let (approval, _) = server.pending(created.id, 0).await?;
    server.respond(created.id, approval).await?;
    let created = server.wait(created.id, JobState::Completed).await?;
    let new_id = created.job.conversation_id.context("Missing new conversation")?;
    let first = server.mcp("conversation_read", json!({"provider":"codex","id":new_id}), token).await?;
    ensure!(first["result"]["structuredContent"]["messages"][0]["text"] == "🦀".repeat(16000));
    let rest = server
        .mcp(
            "conversation_read",
            json!({"provider":"codex","id":new_id,"cursor":first["result"]["structuredContent"]["next_cursor"]}),
            token,
        )
        .await?;
    ensure!(rest["result"]["structuredContent"]["messages"][0]["text"] == "끝");
    ensure!(rest["result"]["structuredContent"]["messages"][1]["user"] == false);
    let page = server.mcp("conversation_list", json!({"provider":"codex","limit":1}), token).await?;
    ensure!(page["result"]["structuredContent"]["next_offset"] == 1);
    let page = server.mcp("conversation_list", json!({"provider":"codex","limit":1,"offset":1}), token).await?;
    ensure!(page["result"]["structuredContent"]["items"][0]["id"] == id && page["result"]["structuredContent"]["next_offset"].is_null());
    for tool in ["conversation_wait", "conversation_interrupt"] {
        ensure!(server.mcp(tool, json!({"provider":"codex","id":id,"job_id":created.job.id}), token).await?["result"]["isError"] == true);
    }
    let other_path = server.root.path().join("other conversation workspace");
    fs::create_dir(&other_path)?;
    fs::copy(server.root.path().join("project 한글/fixture.json"), other_path.join("fixture.json"))?;
    let catalog: ProjectCatalog = server
        .request(Method::POST, "/v1/projects", "default")
        .json(&json!({"path":other_path}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let canonical = fs::canonicalize(&other_path)?;
    let workspace = catalog
        .projects
        .iter()
        .flat_map(|project| &project.workspaces)
        .find(|workspace| fs::canonicalize(&workspace.path).ok().as_ref() == Some(&canonical))
        .context("Missing other workspace")?;
    let account: AccountProfile = server
        .request(Method::POST, "/v1/providers/codex/accounts", "default")
        .json(&json!({"label":"Private conversation profile"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    server.login(Provider::Codex, &account.id).await?;
    let private: Job = server
        .request(Method::POST, "/v1/providers/codex/jobs", &account.id)
        .header("X-KND-Workspace", workspace.id)
        .json(&json!({"text":"private-profile-message"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let (approval, _) = server.pending(private.id, 0).await?;
    server.respond(private.id, approval).await?;
    let private = server.wait(private.id, JobState::Completed).await?;
    let private_id = private.job.conversation_id.context("Missing private conversation")?;
    let private_session = server.sessions.issue(
        McpScope {
            provider: Provider::Codex,
            account_id: account.id.clone(),
            workspace_path: workspace.path.clone(),
        },
        false,
    )?;
    let own = server
        .mcp(
            "conversation_read",
            json!({"provider":"codex","id":private_id}),
            &private_session.connection().token,
        )
        .await?;
    ensure!(
        own["result"]["structuredContent"]["messages"][0]["text"] == "private-profile-message",
        "omitted account/workspace must inherit the session"
    );
    ensure!(
        server
            .mcp("conversation_search", json!({"provider":"codex","query":"private-profile-message"}), token)
            .await?["result"]["structuredContent"]["items"]
            == json!([])
    );
    ensure!(
        server
            .mcp(
                "conversation_read",
                json!({"provider":"codex","id":private_id,"account":account.id,"workspace":workspace.id}),
                token
            )
            .await?["result"]["isError"]
            == true
    );
    let operator = server
        .mcp(
            "conversation_read",
            json!({"provider":"codex","id":private_id,"account":account.id,"workspace":workspace.id}),
            MCP_READ_TOKEN,
        )
        .await?;
    ensure!(operator["result"]["structuredContent"]["messages"][0]["text"] == "private-profile-message");
    server.stop().await?;
    println!("ok: scoped conversation search/read/paging, metadata updates, durable send, run wait/interrupt and account/workspace isolation");
    Ok(())
}

// Given a native question, when a user channel replies through MCP, then the
// provider resumes once; scoped agents cannot answer on the user's behalf.
// Given an MCP form, invalid or repeated HTTP replies must not consume another
// request, and the accepted request must remain visible in saved conversation
// history.
async fn native_tools() -> Result<()> {
    let server = Server::open_scenario(Provider::Codex, "elicitation-accept").await?;
    server.login(Provider::Codex, "default").await?;
    let job = server.enqueue(Provider::Codex, "default", "MCP form", None).await?;
    let (request, after) = server.pending(job.id, 0).await?;
    let route = format!("/v1/jobs/{}/interactions/{request}", job.id);
    for response in [
        json!({"type":"approval","allow":true}),
        json!({"type":"elicitation","action":"accept","content":{"count":"two"}}),
    ] {
        ensure!(server.request(Method::POST, &route, "default").json(&response).send().await?.status() == StatusCode::BAD_REQUEST);
    }
    ensure!(server.job(job.id).await?.job.pending_request_id == Some(request));
    let response = json!({"type":"elicitation","action":"accept","content":{"name":"fixture 🦀","count":2,"enabled":false}});
    server
        .request(Method::POST, &route, "default")
        .json(&response)
        .send()
        .await?
        .error_for_status()?;
    ensure!(
        !server
            .request(Method::POST, &route, "default")
            .json(&response)
            .send()
            .await?
            .status()
            .is_success()
    );
    let (command, _) = server.pending(job.id, after).await?;
    server
        .request(Method::POST, &format!("/v1/jobs/{}/interactions/{command}", job.id), "default")
        .json(&json!({"type":"approval","allow":false}))
        .send()
        .await?
        .error_for_status()?;
    let done = server.wait(job.id, JobState::Completed).await?;
    let saved: SavedConversation = server
        .request(
            Method::GET,
            &format!("/v1/providers/codex/conversations/{}", done.job.conversation_id.unwrap()),
            "default",
        )
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        saved.timeline.iter().any(|entry| matches!(entry.event, Event::Elicitation { .. })),
        "MCP requests must survive history reload"
    );
    server.stop().await?;
    for scenario in ["permissions-accept", "elicitation-url-accept", "progress-events"] {
        let server = Server::open_scenario(Provider::Codex, scenario).await?;
        server.login(Provider::Codex, "default").await?;
        let job = server.enqueue(Provider::Codex, "default", "Native tool progress", None).await?;
        let (request, after) = server.pending(job.id, 0).await?;
        let response = match scenario {
            | "permissions-accept" => json!({"type":"approval","allow":true}),
            | "elicitation-url-accept" => json!({"type":"elicitation","action":"accept","content":null}),
            | _ => json!({"type":"approval","allow":false}),
        };
        server
            .request(Method::POST, &format!("/v1/jobs/{}/interactions/{request}", job.id), "default")
            .json(&response)
            .send()
            .await?
            .error_for_status()?;
        if scenario != "progress-events" {
            let (command, _) = server.pending(job.id, after).await?;
            server
                .request(Method::POST, &format!("/v1/jobs/{}/interactions/{command}", job.id), "default")
                .json(&json!({"type":"approval","allow":false}))
                .send()
                .await?
                .error_for_status()?;
        }
        let done = server.wait(job.id, JobState::Completed).await?;
        let saved: SavedConversation = server
            .request(
                Method::GET,
                &format!("/v1/providers/codex/conversations/{}", done.job.conversation_id.unwrap()),
                "default",
            )
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if scenario == "progress-events" {
            let replay = server
                .request(Method::GET, &format!("/v1/jobs/{}/events?after=0", job.id), "default")
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let replay: Vec<JobEvent> = replay.lines().map(serde_json::from_str).collect::<std::result::Result<_, _>>()?;
            for (id, expected) in [
                (
                    "mcp-progress",
                    vec![
                        json!({"type":"progress","message":"자료 검색 중"}),
                        json!({"type":"progress","message":"자료 검증 중"}),
                    ],
                ),
                (
                    "terminal-progress",
                    vec![
                        json!({"type":"terminal_input","process_id":"process-42","stdin":"계속\nnext line\n"}),
                        json!({"type":"terminal_input","process_id":"process-42","stdin":""}),
                    ],
                ),
            ] {
                let activity: Vec<_> = saved
                    .timeline
                    .iter()
                    .filter_map(|entry| match &entry.event {
                        | Event::Tool {
                            call,
                        } if call.id == id => Some(&call.activity),
                        | _ => None,
                    })
                    .flatten()
                    .collect();
                ensure!(
                    serde_json::to_value(activity)? == json!(expected),
                    "Tool activity must survive conversation history reload"
                );
            }
            let saved_patches: Vec<_> = saved
                .timeline
                .iter()
                .filter_map(|entry| match &entry.event {
                    | Event::Tool {
                        call,
                    } if call.id == "patch-progress" => Some(&call.input),
                    | _ => None,
                })
                .collect();
            ensure!(
                serde_json::to_value(saved_patches)?
                    == json!([[],[{"path":"src/fixture.rs","kind":{"type":"update","move_path":null},"diff":"-before\n+after\n"}],[],[]]),
                "Patch snapshots must remain in order, including empty replacements"
            );
            for id in ["mcp-progress", "terminal-progress", "patch-progress"] {
                let saved_calls: Vec<_> = saved
                    .timeline
                    .iter()
                    .filter_map(|entry| match &entry.event {
                        | Event::Tool {
                            call,
                        } if call.id == id => Some(call),
                        | _ => None,
                    })
                    .collect();
                let replayed_calls: Vec<_> = replay
                    .iter()
                    .filter_map(|entry| match &entry.event {
                        | StreamEvent::Agent(Event::Tool {
                            call,
                        }) if call.id == id => Some(call),
                        | _ => None,
                    })
                    .collect();
                ensure!(
                    serde_json::to_value(saved_calls)? == serde_json::to_value(replayed_calls)?,
                    "Job replay must match recorded tool activity"
                );
            }
            ensure!(saved.timeline.iter().any(|entry| matches!(&entry.event, Event::Tool { call } if call.title.as_deref() == Some("update_plan") && call.status.as_deref() == Some("completed"))));
            ensure!(saved.timeline.iter().any(|entry| matches!(&entry.event, Event::Tool { call } if call.id == "collab-1" && call.output.as_ref().unwrap()["receiverThreadIds"] == json!(["child-1"]))));
            for (id, item) in [
                (
                    "image-view-1",
                    json!({"type":"imageView","id":"image-view-1","path":"C:/fixture/이미지 preview.png"}),
                ),
                ("compaction-1", json!({"type":"contextCompaction","id":"compaction-1"})),
            ] {
                let calls: Vec<_> = saved
                    .timeline
                    .iter()
                    .filter_map(|entry| match &entry.event {
                        | Event::Tool {
                            call,
                        } if call.id == id => Some(call),
                        | _ => None,
                    })
                    .collect();
                ensure!(calls.len() == 2, "Image/compaction progress must survive a conversation history reload");
                ensure!(calls[0].status.as_deref() == Some("in_progress") && calls[1].status.as_deref() == Some("completed"));
                ensure!(
                    calls.iter().all(|call| call.output.as_ref() == Some(&item)),
                    "Native metadata must be preserved in persisted progress"
                );
                let replayed: Vec<_> = replay
                    .iter()
                    .filter_map(|entry| match &entry.event {
                        | StreamEvent::Agent(Event::Tool {
                            call,
                        }) if call.id == id => Some(call),
                        | _ => None,
                    })
                    .collect();
                ensure!(
                    serde_json::to_value(&replayed)? == serde_json::to_value(&calls)?,
                    "Replayed HTTP events must match persisted conversation progress"
                );
            }
            ensure!(
                !saved
                    .timeline
                    .iter()
                    .any(|entry| matches!(&entry.event,Event::Tool{call} if call.id=="foreign-compaction"))
            );
        }
        server.stop().await?;
    }
    let server = Server::open_scenario(Provider::Codex, "elicitation-cancel").await?;
    server.login(Provider::Codex, "default").await?;
    let job = server.enqueue(Provider::Codex, "default", "Cancel pending MCP input", None).await?;
    let (request, _) = server.pending(job.id, 0).await?;
    server
        .request(Method::DELETE, &format!("/v1/jobs/{}", job.id), "default")
        .send()
        .await?
        .error_for_status()?;
    server.wait(job.id, JobState::Cancelled).await?;
    ensure!(
        !server
            .request(Method::POST, &format!("/v1/jobs/{}/interactions/{request}", job.id), "default")
            .json(&json!({"type":"cancelled"}))
            .send()
            .await?
            .status()
            .is_success()
    );
    server.stop().await?;
    println!(
        "ok: authenticated permissions, MCP form/URL, invalid/duplicate/stale rejection, cancellation and persisted/replayed plan, collaboration, image-view and compaction progress"
    );
    Ok(())
}

// Given HTTP MCP settings, saving and reloading must preserve the account's
// configuration; rejected updates must leave the previous settings intact.
async fn remote_mcp_configuration() -> Result<()> {
    let server = Server::open(Provider::Codex).await?;
    let route = "/v1/mcp";
    let legacy = br#"[{"name":"legacy","url":"https://legacy.invalid/mcp"}]"#;
    for profile in ["claude", "codex", "accounts/work/antigravity"] {
        let directory = server.root.path().join("home").join(profile);
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("mcp.json"), legacy)?;
    }
    let imported: serde_json::Value = server.request(Method::GET, route, "default").send().await?.error_for_status()?.json().await?;
    ensure!(
        imported == serde_json::from_slice::<serde_json::Value>(legacy)?,
        "Identical legacy servers must be merged without duplicates"
    );
    ensure!(!server.root.path().join("home/mcp.json").exists(), "Reading must not migrate files implicitly");
    fs::write(
        server.root.path().join("home/codex/mcp.json"),
        br#"[{"name":"legacy","url":"https://other.invalid/mcp"}]"#,
    )?;
    ensure!(
        !server.request(Method::GET, route, "default").send().await?.status().is_success(),
        "Conflicting names must not silently choose a provider"
    );
    fs::write(server.root.path().join("home/codex/mcp.json"), legacy)?;
    let providers: serde_json::Value = server
        .request(Method::GET, "/v1/providers", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    for provider in providers.as_array().unwrap() {
        ensure!(
            provider["capabilities"]["mcp_http"] == (provider["provider"] != "antigravity"),
            "HTTP MCP capability must distinguish native support from runtime capability checks"
        );
    }
    let config = json!([
        {"name":"fixture","command":std::env::current_exe()?,"args":["--stdio"]},
        {"name":"remote","url":"https://mcp.example.invalid/tools","bearer_token_env_var":"MCP_ACCESS_TOKEN","env_http_headers":{"X-Tenant":"MCP_TENANT"}}
    ]);
    ensure!(server.client.put(format!("{}{route}", server.url)).json(&config).send().await?.status() == StatusCode::UNAUTHORIZED);
    let saved: serde_json::Value = server
        .request(Method::PUT, route, "default")
        .json(&config)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(saved == config, "Both MCP transports must be saved without resolving credentials");
    for url in ["http://outside.invalid/mcp", "https://user:password@example.invalid/mcp"] {
        ensure!(
            server
                .request(Method::PUT, route, "default")
                .json(&json!([{"name":"unsafe","url":url}]))
                .send()
                .await?
                .status()
                == StatusCode::BAD_REQUEST
        );
    }
    let loaded: serde_json::Value = server.request(Method::GET, route, "default").send().await?.error_for_status()?.json().await?;
    ensure!(loaded == config);
    let foreign: serde_json::Value = server
        .request(Method::GET, "/v1/mcp/claude", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(foreign == config, "Given a common registration, Claude must see the same servers as Codex");
    let third: serde_json::Value = server
        .request(Method::GET, "/v1/mcp/antigravity", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(third == config, "All three providers must share MCP registration");
    server
        .request(Method::PUT, "/v1/mcp/claude", "default")
        .json(&json!([]))
        .send()
        .await?
        .error_for_status()?;
    for provider in ["claude", "codex", "antigravity"] {
        let loaded: serde_json::Value = server
            .request(Method::GET, &format!("/v1/mcp/{provider}"), "default")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(loaded == json!([]), "An empty common catalog must not revive legacy servers");
    }
    for profile in ["claude", "codex", "accounts/work/antigravity"] {
        ensure!(
            fs::read(server.root.path().join("home").join(profile).join("mcp.json"))? == legacy,
            "Original profile bytes must be preserved"
        );
    }
    server.stop().await?;
    println!("ok: authenticated common MCP settings, mixed transports, environment references, invalid-update rollback and all-provider visibility");
    Ok(())
}

async fn codex_policy_configuration() -> Result<()> {
    let server = Server::open_scenario(Provider::Codex, "codex-policy").await?;
    server.login(Provider::Codex, "default").await?;
    let mut conversation = None;
    for policy in [
        json!({"approval_policy":"on-request","sandbox":"workspace-write"}),
        json!({"approval_policy":"never","sandbox":"read-only"}),
    ] {
        fs::write(
            server.root.path().join("project 한글/fixture.json"),
            json!({"provider":"codex","scenario":"codex-policy","policy":policy}).to_string(),
        )?;
        let job: Job = server
            .request(Method::POST, "/v1/providers/codex/jobs", "default")
            .json(&json!({"text":"안녕 $() `test`\nsecond line","conversation_id":conversation,"codex_execution_policy":policy}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let (request, _) = server.pending(job.id, 0).await?;
        server
            .request(Method::POST, &format!("/v1/jobs/{}/interactions/{request}", job.id), "default")
            .json(&json!({"type":"approval","allow":false}))
            .send()
            .await?
            .error_for_status()?;
        let done = server.wait(job.id, JobState::Completed).await?;
        ensure!(
            serde_json::to_value(&done.request)?["codex_execution_policy"] == policy,
            "Queued/resumed jobs must preserve the selected execution policy"
        );
        conversation = done.job.conversation_id;
    }
    ensure!(
        server
            .request(Method::POST, "/v1/providers/claude/jobs", "default")
            .json(&json!({"text":"Wrong provider","codex_execution_policy":{}}))
            .send()
            .await?
            .status()
            == StatusCode::BAD_REQUEST
    );
    server.stop().await?;
    println!("ok: selected Codex policies persist through queued start/resume and are rejected for other providers");
    Ok(())
}

async fn oauth_mcp_configuration() -> Result<()> {
    let fixture = tokio::task::spawn_blocking(|| native::oauth_fixture::OAuthFixture::new("valid")).await??;
    let server = Server::open(Provider::Codex).await?;
    let route = "/v1/mcp/codex/remote/oauth";
    let config = serde_json::to_value(vec![fixture.server()])?;
    server
        .request(Method::PUT, "/v1/mcp/codex", "default")
        .json(&config)
        .send()
        .await?
        .error_for_status()?;
    ensure!(server.client.get(format!("{}{route}", server.url)).send().await?.status() == StatusCode::UNAUTHORIZED);
    let state: serde_json::Value = server.request(Method::GET, route, "default").send().await?.error_for_status()?.json().await?;
    ensure!(state["state"] == "signed_out");
    for command in ["cancel", "logout"] {
        let state: serde_json::Value = server
            .request(Method::POST, route, "default")
            .json(&command)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(state["state"] == if command == "cancel" { "cancelled" } else { "signed_out" });
        ensure!(state.get("access_token").is_none() && state.get("refresh_token").is_none());
    }
    let pending: McpOAuthStatus = server
        .request(Method::POST, route, "default")
        .json(&McpOAuthCommand::Login)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(pending.state == McpOAuthState::Pending);
    let callback = native::oauth_fixture::callback(pending.authorization_url.as_deref().context("Missing browser URL")?, "fixture-code")?;
    ensure!(server.client.get(callback).send().await?.status() == StatusCode::OK);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let state: McpOAuthStatus = server.request(Method::GET, route, "default").send().await?.error_for_status()?.json().await?;
        ensure!(!serde_json::to_string(&state)?.contains("fixture-oauth"));
        if state.state == McpOAuthState::Connected {
            break;
        }
        ensure!(tokio::time::Instant::now() < deadline, "OAuth API callback did not complete");
        tokio::task::yield_now().await;
    }
    server
        .request(Method::POST, route, "default")
        .json(&McpOAuthCommand::Logout)
        .send()
        .await?
        .error_for_status()?;
    let other: McpOAuthStatus = server
        .request(Method::GET, "/v1/mcp/claude/remote/oauth", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        other.state == McpOAuthState::SignedOut,
        "Common registration must preserve separate provider credentials"
    );
    let loaded: serde_json::Value = server
        .request(Method::GET, "/v1/mcp/codex", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(loaded == config);
    server
        .request(Method::PUT, "/v1/mcp/codex", "default")
        .json(&json!([]))
        .send()
        .await?
        .error_for_status()?;
    let plugin = json!({"scope":"user","files":{
        "plugin.json":serde_json::to_vec(&json!({"name":"oauth-fixture","description":"Package OAuth fixture","mcp_servers":[fixture.server()]}))?
    }});
    server
        .request(Method::POST, "/v1/plugins/import", "default")
        .json(&plugin)
        .send()
        .await?
        .error_for_status()?;
    server
        .request(Method::PUT, "/v1/plugins/enabled", "default")
        .json(&json!({"id":"user:oauth-fixture","enabled":true}))
        .send()
        .await?
        .error_for_status()?;
    let state: McpOAuthStatus = server.request(Method::GET, route, "default").send().await?.error_for_status()?.json().await?;
    ensure!(state.state == McpOAuthState::SignedOut);
    server
        .request(Method::POST, route, "default")
        .json(&McpOAuthCommand::Logout)
        .send()
        .await?
        .error_for_status()?;
    let standalone: serde_json::Value = server
        .request(Method::GET, "/v1/mcp/codex", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(standalone == json!([]), "Plugin servers must not enter the standalone MCP editor");
    server
        .request(Method::POST, "/v1/plugins/remove", "default")
        .json(&json!({"id":"user:oauth-fixture"}))
        .send()
        .await?
        .error_for_status()?;
    ensure!(server.request(Method::GET, route, "default").send().await?.status() == StatusCode::NOT_FOUND);
    server.stop().await?;
    println!("ok: operator-only standalone and plugin MCP OAuth lookup without rewriting legacy settings or returning credentials");
    Ok(())
}

async fn mcp_questions() -> Result<()> {
    for provider in [Provider::Claude, Provider::Codex] {
        for cancel in [false, true] {
            let server = Server::open_scenario(provider, if cancel { "interactive-question-cancel" } else { "interactive-question" }).await?;
            server.login(provider, "default").await?;
            let job = server.enqueue(provider, "default", "ask the user", None).await?;
            let (approval, after) = server.pending(job.id, 0).await?;
            ensure!(server.mcp("get_pending_questions", json!({"id":job.id}), TOKEN).await?["result"]["isError"] == false);
            ensure!(server.mcp("get_pending_questions", json!({"id":job.id}), TOKEN).await?["result"]["structuredContent"]["pending"].is_null());
            ensure!(
                server
                    .mcp(
                        "answer_questions",
                        json!({"id":job.id,"request_id":approval,"response":{"type":"cancelled"}}),
                        TOKEN
                    )
                    .await?["result"]["isError"]
                    == true
            );
            server.respond(job.id, approval).await?;
            let (question, after) = server.pending(job.id, after).await?;
            let scope = McpScope {
                provider,
                account_id: "default".into(),
                workspace_path: job.scope.workspace_path.clone(),
            };
            let agent = server.sessions.issue(scope.clone(), true)?;
            let frames = bridge_probe(agent.connection()).await?;
            let tools = frames[0]["result"]["tools"].as_array().context("scoped tools")?;
            ensure!(tools.iter().any(|tool| tool["name"] == "get_pending_questions"));
            ensure!(!tools.iter().any(|tool| tool["name"] == "answer_questions"));
            let reader = server.sessions.issue(scope.clone(), false)?;
            let foreign = server.sessions.issue(
                McpScope {
                    account_id: "0123456789abcdef0123456789abcdef".into(),
                    ..scope
                },
                true,
            )?;
            let pending = server.mcp("get_pending_questions", json!({"id":job.id}), &reader.connection().token).await?;
            let pending = &pending["result"]["structuredContent"]["pending"];
            ensure!(pending["request_id"] == question && pending["questions"][0]["question"] == "Choose?");
            ensure!(pending["questions"][0]["options"] == json!(["A", "B"]));
            let question_id = pending["questions"][0]["id"].as_str().context("question ID")?;
            let answer = json!({"id":job.id,"request_id":question,"response":if cancel { json!({"type":"cancelled"}) } else { json!({"type":"answers","answers":{question_id:["B"]}}) }});
            ensure!(server.mcp("get_pending_questions", json!({"id":job.id}), &foreign.connection().token).await?["result"]["isError"] == true);
            for token in [&agent.connection().token, &reader.connection().token] {
                ensure!(server.mcp("answer_questions", answer.clone(), token).await?["error"]["code"] == -32001);
            }
            for invalid in [
                json!({"id":job.id,"request_id":question + 1,"response":{"type":"cancelled"}}),
                json!({"id":job.id,"request_id":question,"response":{"type":"approval","allow":true}}),
                json!({"id":job.id,"request_id":question,"response":{"type":"answers","answers":{}}}),
                json!({"id":job.id,"request_id":question,"response":{"type":"answers","answers":{question_id:[""]}}}),
                json!({"id":job.id,"request_id":question,"response":{"type":"answers","answers":{question_id:["A","B"]}}}),
            ] {
                ensure!(server.mcp("answer_questions", invalid, TOKEN).await?["result"]["isError"] == true);
            }
            ensure!(
                server.mcp("get_pending_questions", json!({"id":job.id}), TOKEN).await?["result"]["structuredContent"]["pending"]["request_id"] == question
            );
            let (one, two) = tokio::try_join!(
                server.mcp("answer_questions", answer.clone(), TOKEN),
                server.mcp("answer_questions", answer.clone(), TOKEN)
            )?;
            ensure!([one, two].iter().filter(|reply| reply["result"]["isError"] == false).count() == 1);
            ensure!(server.mcp("answer_questions", answer, TOKEN).await?["result"]["isError"] == true);
            server.wait(job.id, JobState::Completed).await?;
            ensure!(server.mcp("get_pending_questions", json!({"id":job.id}), TOKEN).await?["result"]["structuredContent"]["pending"].is_null());
            let events = server.mcp("get_job_events", json!({"id":job.id,"after":after}), TOKEN).await?;
            ensure!(
                events["result"]["structuredContent"]["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|event| event["event"]["type"] == "interaction_resolved" && event["event"]["request_id"] == question)
            );
            server.stop().await?;
        }
    }
    let server = Server::open_scenario(Provider::Codex, "interactive-question").await?;
    server.login(Provider::Codex, "default").await?;
    let job = server.enqueue(Provider::Codex, "default", "cancel while asking", None).await?;
    let (approval, after) = server.pending(job.id, 0).await?;
    server.respond(job.id, approval).await?;
    let (question, _) = server.pending(job.id, after).await?;
    ensure!(server.mcp("cancel_job", json!({"id":job.id}), TOKEN).await?["result"]["isError"] == false);
    server.wait(job.id, JobState::Cancelled).await?;
    ensure!(server.mcp("get_pending_questions", json!({"id":job.id}), TOKEN).await?["result"]["structuredContent"]["pending"].is_null());
    ensure!(
        server
            .mcp(
                "answer_questions",
                json!({"id":job.id,"request_id":question,"response":{"type":"cancelled"}}),
                TOKEN
            )
            .await?["result"]["isError"]
            == true
    );
    server.stop().await?;
    println!("ok: native Claude/Codex questions, scoped reads, operator-only answers/skip, stale/invalid rejection and once-only response");
    Ok(())
}

// Given a completed source, a fork preserves its prefix in an independent
// native session.
async fn mcp_context_transfers() -> Result<()> {
    let server = Server::open(Provider::Codex).await?;
    server.login(Provider::Codex, "default").await?;
    let job: Job = server
        .request(Method::POST, "/v1/providers/codex/jobs", "default")
        .json(&json!({"text":"branch source marker","model":"first"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let (approval, _) = server.pending(job.id, 0).await?;
    server.respond(job.id, approval).await?;
    let completed = server.wait(job.id, JobState::Completed).await?;
    let source = completed.job.conversation_id.context("Missing source")?;
    let saved: SavedConversation = server
        .request(Method::GET, &format!("/v1/providers/codex/conversations/{source}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let boundary = saved.messages.last().and_then(|message| message.id).context("Missing boundary")?;
    let session = server.sessions.issue(
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: completed.job.scope.workspace_path.clone(),
        },
        true,
    )?;
    let token = &session.connection().token;
    let input = json!({"provider":"codex","id":source,"through_message_id":boundary,"title":"Independent branch","command_id":"fork-context-contract"});
    let forked = server.mcp("conversation_fork", input.clone(), token).await?;
    ensure!(forked["result"]["isError"] == false, "fork failed: {forked}");
    let transfer = &forked["result"]["structuredContent"];
    let branch = transfer["target_id"].as_i64().context("Missing branch")?;
    ensure!(branch != source && transfer["state"] == "pending" && transfer["through_message_id"] == boundary);
    let pending = server.mcp("conversation_transfers", json!({"provider":"codex","id":branch}), token).await?;
    ensure!(pending["result"]["structuredContent"]["items"][0]["state"] == "pending");
    ensure!(pending["result"]["structuredContent"]["items"][0].get("context").is_none());
    let retry = server.mcp("conversation_fork", input, token).await?;
    ensure!(retry["result"]["structuredContent"]["id"] == transfer["id"]);
    let saved: SavedConversation = server
        .request(Method::GET, &format!("/v1/providers/codex/conversations/{branch}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(saved.session_id.is_none() && saved.messages.len() == 2 && saved.messages[0].text == "branch source marker");
    ensure!(saved.model.as_deref() == Some("first"));
    let sent = server
        .mcp(
            "conversation_send",
            json!({"provider":"codex","id":branch,"text":"branch work marker","command_id":"branch-first-turn"}),
            token,
        )
        .await?;
    let sent: Job = serde_json::from_value(sent["result"]["structuredContent"].clone())?;
    let (approval, _) = server.pending(sent.id, 0).await?;
    let attached = server.mcp("conversation_transfers", json!({"provider":"codex","id":branch}), token).await?;
    ensure!(
        attached["result"]["structuredContent"]["items"][0]["state"] == "attached",
        "attachment is recorded at turn admission, before provider completion"
    );
    server.respond(sent.id, approval).await?;
    server.wait(sent.id, JobState::Completed).await?;
    let saved: SavedConversation = server
        .request(Method::GET, &format!("/v1/providers/codex/conversations/{branch}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        saved.model.as_deref() == Some("first"),
        "the first independent branch turn must inherit the saved model"
    );
    let branch_boundary = saved.messages.last().and_then(|message| message.id).context("Missing branch boundary")?;
    let merged_input =
        json!({"provider":"codex","source_id":branch,"target_id":source,"through_message_id":branch_boundary,"command_id":"merge-context-contract"});
    let (merged, retry) = tokio::try_join!(
        server.mcp("conversation_merge_back", merged_input.clone(), token),
        server.mcp("conversation_merge_back", merged_input.clone(), token)
    )?;
    ensure!(merged["result"]["isError"] == false, "merge failed: {merged}");
    let merge_id = merged["result"]["structuredContent"]["id"].as_i64().context("Missing merge ID")?;
    ensure!(retry["result"]["structuredContent"]["id"] == merge_id);
    ensure!(merged["result"]["structuredContent"]["state"] == "pending");
    let unchanged: SavedConversation = server
        .request(Method::GET, &format!("/v1/providers/codex/conversations/{source}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        unchanged.messages.len() == 2 && unchanged.messages[0].text == "branch source marker",
        "saving a merge must not rewrite or run the parent"
    );
    let parent_turn = server
        .mcp(
            "conversation_send",
            json!({"provider":"codex","id":source,"text":"consume branch context","command_id":"consume-context-contract"}),
            token,
        )
        .await?;
    let parent_turn: Job = serde_json::from_value(parent_turn["result"]["structuredContent"].clone())?;
    let (approval, _) = server.pending(parent_turn.id, 0).await?;
    let parent: SavedConversation = server
        .request(Method::GET, &format!("/v1/providers/codex/conversations/{source}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let contexts: Vec<_> = parent.messages[2].attachments.iter().filter_map(Attachment::text_context).collect();
    ensure!(
        contexts.len() == 1
            && contexts[0].contains("branch work marker")
            && !contexts[0].contains("branch source marker")
            && !contexts[0].contains("codex-session")
    );
    let transfers = server
        .mcp("conversation_transfers", json!({"provider":"codex","id":source,"after":merge_id-1}), token)
        .await?;
    ensure!(
        transfers["result"]["structuredContent"]["items"][0]["id"] == merge_id && transfers["result"]["structuredContent"]["items"][0]["state"] == "attached"
    );
    server.respond(parent_turn.id, approval).await?;
    server.wait(parent_turn.id, JobState::Completed).await?;
    let again = server
        .mcp(
            "conversation_send",
            json!({"provider":"codex","id":source,"text":"no repeated context","command_id":"no-repeat-context-contract"}),
            token,
        )
        .await?;
    let again: Job = serde_json::from_value(again["result"]["structuredContent"].clone())?;
    let (approval, _) = server.pending(again.id, 0).await?;
    let parent: SavedConversation = server
        .request(Method::GET, &format!("/v1/providers/codex/conversations/{source}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(parent.messages[4].attachments.is_empty(), "accepted merge context must attach only once");
    server.respond(again.id, approval).await?;
    server.wait(again.id, JobState::Completed).await?;
    let reader = server.sessions.issue(
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: completed.job.scope.workspace_path.clone(),
        },
        false,
    )?;
    ensure!(
        server
            .mcp(
                "conversation_fork",
                json!({"provider":"codex","id":source,"command_id":"denied-fork"}),
                &reader.connection().token
            )
            .await?["error"]["code"]
            == -32001
    );
    ensure!(server.mcp("conversation_merge_back", merged_input.clone(), &reader.connection().token).await?["error"]["code"] == -32001);
    ensure!(
        server
            .mcp("conversation_transfers", json!({"provider":"codex","id":branch}), &reader.connection().token)
            .await?["result"]["isError"]
            == false
    );
    for (tool, arguments) in [
        (
            "conversation_fork",
            json!({"provider":"codex","id":source,"through_message_id":boundary,"title":"changed","command_id":"fork-context-contract"}),
        ),
        (
            "conversation_fork",
            json!({"provider":"codex","id":source,"through_message_id":unchanged.messages[0].id,"command_id":"user-boundary"}),
        ),
        ("conversation_fork", json!({"provider":"codex","id":source,"command_id":""})),
        ("conversation_fork", json!({"provider":"claude","id":source,"command_id":"wrong-provider"})),
        (
            "conversation_fork",
            json!({"provider":"codex","id":source,"account":"0123456789abcdef0123456789abcdef","command_id":"wrong-account"}),
        ),
        (
            "conversation_fork",
            json!({"provider":"codex","id":source,"workspace":i64::MAX,"command_id":"wrong-workspace"}),
        ),
        (
            "conversation_merge_back",
            json!({"provider":"codex","source_id":source,"target_id":branch,"command_id":"wrong-direction"}),
        ),
        (
            "conversation_merge_back",
            json!({"provider":"codex","source_id":source,"target_id":source,"command_id":"self-merge"}),
        ),
        (
            "conversation_merge_back",
            json!({"provider":"codex","source_id":branch,"target_id":source,"through_message_id":branch_boundary,"command_id":"fork-context-contract"}),
        ),
    ] {
        ensure!(
            server.mcp(tool, arguments, token).await?["result"]["isError"] == true,
            "invalid {tool} accepted"
        );
    }
    server.stop().await?;
    println!("ok: scoped durable fork, model inheritance, branch-only merge, transfer states, once-only context delivery and permission boundaries");
    Ok(())
}

async fn lifecycle(provider: Provider) -> Result<()> {
    let server = Server::open(provider).await?;
    let catalog: ProjectCatalog = server
        .request(Method::GET, "/v1/projects", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let workspace = catalog.default_workspace_id.unwrap();
    git_features(&server, workspace).await?;
    let route = format!("/v1/providers/{}", provider_name(provider));
    ensure!(server.client.get(format!("{}/v1/jobs", server.url)).send().await?.status() == StatusCode::UNAUTHORIZED);
    ensure!(
        server
            .request(Method::GET, "/v1/jobs", "default")
            .header("Origin", "https://example.invalid")
            .send()
            .await?
            .status()
            == StatusCode::UNAUTHORIZED
    );
    let account: AccountProfile = server
        .request(Method::POST, &format!("{route}/accounts"), "default")
        .json(&json!({"label":"Second account 한글"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(account.id != "default" && account.label == "Second account 한글");
    ensure!(
        server
            .request(Method::POST, &format!("{route}/jobs"), "../escape")
            .json(&json!({"text":"bad"}))
            .send()
            .await?
            .status()
            == StatusCode::BAD_REQUEST
    );
    server.login(provider, "default").await?;
    server.login(provider, &account.id).await?;
    let first = server.enqueue(provider, "default", "first-account-A", None).await?;
    let first = server.wait(first.id, JobState::Waiting).await?;
    let conversation = first.job.conversation_id.context("Missing conversation")?;
    let catalog: ProjectCatalog = server
        .request(Method::GET, "/v1/projects", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let workspace = catalog.default_workspace_id.unwrap();
    ensure!(
        server
            .request(Method::DELETE, &format!("/v1/projects/{}", catalog.projects[0].id), "default")
            .send()
            .await?
            .status()
            == StatusCode::CONFLICT
    );
    // Given a real Git repository and an active native job, inspection and
    // proposal save intent without staging or committing the agent's changes.
    let git_route = format!("/v1/workspaces/{workspace}/git");
    let inspected: GitOverview = server
        .request(Method::POST, &git_route, "default")
        .header("x-knd-workspace", workspace)
        .json(&json!({"type":"inspect"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(inspected.files.iter().any(|file| file.path == "file.txt"));
    let proposed: GitOverview = server
        .request(Method::POST, &git_route, "default")
        .header("x-knd-workspace", workspace)
        .json(&json!({"type":"propose_commit","message":"Agent proposal during active work","paths":["file.txt"]}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(proposed.version == inspected.version && proposed.commit.is_none());
    let proposal = proposed.proposal.context("Missing active-job commit proposal")?;
    ensure!(proposal.expected == inspected.version && proposal.paths == ["file.txt"]);
    ensure!(proposal.message == "Agent proposal during active work");
    // When any Git mutation is requested, then the active job still prevents
    // it.
    for command in [
        json!({"type":"preview_pr","remote":"origin","base":"main"}),
        json!({"type":"create_pr","remote":"origin","base":"main","title":"blocked","body":"body","draft":true,"push":true,"expected":{"head":null,"index":"x","worktree":"x"},"expected_base":"base","expected_branch":"feature/test","expected_repository":"owner/repo"}),
        json!({"type":"checkpoint","label":"busy"}),
        json!({"type":"stage","path":"file.txt","staged":true,"expected":inspected.version}),
        json!({"type":"preview_restore","checkpoint":"invalid"}),
        json!({"type":"restore","checkpoint":"invalid","expected":{"head":null,"index":"x","worktree":"x"}}),
        json!({"type":"commit","message":"busy","expected":inspected.version}),
        json!({"type":"commit_selected","message":"busy","paths":["file.txt"],"expected":inspected.version}),
    ] {
        ensure!(
            server
                .request(Method::POST, &format!("/v1/workspaces/{workspace}/git"), "default")
                .json(&command)
                .send()
                .await?
                .status()
                == StatusCode::CONFLICT
        );
    }

    ensure!(
        server
            .request(Method::POST, &format!("/v1/runtimes/{}", provider_name(provider)), "default")
            .json(&json!({"type":"install","version":"1.0.0","sha256":"0".repeat(64)}))
            .send()
            .await?
            .status()
            == StatusCode::CONFLICT
    );
    ensure!(
        server
            .request(Method::POST, &format!("/v1/runtimes/{}", provider_name(provider)), "default")
            .json(&json!({"type":"install","version":"../bad","sha256":"0".repeat(64)}))
            .send()
            .await?
            .status()
            == StatusCode::BAD_REQUEST
    );
    let (approval, cursor) = server.pending(first.job.id, 0).await?;
    ensure!(first.job.pending_request_id == Some(approval));
    ensure!(
        server
            .request(Method::DELETE, &format!("{route}/conversations/{conversation}"), "default")
            .send()
            .await?
            .status()
            == StatusCode::CONFLICT
    );
    // Dropping the stream must not cancel a queued/background operation.
    ensure!(server.job(first.job.id).await?.job.state == JobState::Waiting);
    let second = server.enqueue(provider, "default", "second-A", Some(conversation)).await?;
    let third = server.enqueue(provider, "default", "third-A", Some(conversation)).await?;
    let removed = server.enqueue(provider, "default", "cancel queued", None).await?;
    ensure!(server.job(second.id).await?.job.state == JobState::Queued);
    server
        .request(Method::DELETE, &format!("/v1/jobs/{}", removed.id), "default")
        .send()
        .await?
        .error_for_status()?;
    ensure!(server.job(removed.id).await?.job.state == JobState::Cancelled);
    ensure!(
        server
            .request(Method::PATCH, &format!("/v1/jobs/{}", second.id), "default")
            .json(&json!({"text":"edited"}))
            .send()
            .await?
            .status()
            == StatusCode::METHOD_NOT_ALLOWED,
        "Queued jobs can no longer be edited"
    );
    let other = server.enqueue(provider, &account.id, "first-account-B", None).await?;
    let other = server.wait(other.id, JobState::Waiting).await?;
    ensure!(
        server.job(first.job.id).await?.job.state == JobState::Waiting,
        "Accounts did not run concurrently"
    );
    let a: SavedConversation = server
        .request(Method::GET, &format!("{route}/conversations/{conversation}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let b: SavedConversation = server
        .request(
            Method::GET,
            &format!("{route}/conversations/{}", other.job.conversation_id.unwrap()),
            &account.id,
        )
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(a.messages[0].text == "first-account-A" && b.messages[0].text == "first-account-B");
    let summaries: Vec<ConversationSummary> = server
        .request(Method::GET, &format!("{route}/conversations"), "default")
        .send()
        .await?
        .json()
        .await?;
    ensure!(summaries[0].execution_state == Some(JobState::Waiting));
    let state_route = format!("{route}/conversations/{conversation}/state");
    server
        .request(Method::PATCH, &state_route, "default")
        .json(&json!({"pinned":true,"archived":true}))
        .send()
        .await?
        .error_for_status()?;
    let other_summaries: Vec<ConversationSummary> = server
        .request(Method::GET, &format!("{route}/conversations"), &account.id)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(!other_summaries[0].pinned && !other_summaries[0].archived, "Metadata leaked across profiles");
    server
        .request(
            Method::PATCH,
            &format!("{route}/conversations/{}/state", other.job.conversation_id.unwrap()),
            &account.id,
        )
        .json(&json!({"pinned":true,"archived":true}))
        .send()
        .await?
        .error_for_status()?;
    let hidden: Vec<ConversationSummary> = server
        .request(Method::GET, &format!("{route}/conversations"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(hidden.is_empty());
    let archived: Vec<ConversationSummary> = server
        .request(Method::GET, &format!("{route}/conversations?archived=archived&q=first-account"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(archived.len() == 1 && archived[0].pinned && archived[0].archived && archived[0].matched_message.is_some());
    ensure!(
        server
            .request(Method::GET, &format!("{route}/conversations?archived=bad"), "default")
            .send()
            .await?
            .status()
            == StatusCode::BAD_REQUEST
    );
    ensure!(
        server
            .request(Method::PATCH, &state_route, "default")
            .json(&json!({"pinned":"true"}))
            .send()
            .await?
            .status()
            == StatusCode::UNPROCESSABLE_ENTITY
    );

    let events = server
        .request(Method::GET, &format!("/v1/jobs/{}/events?after={cursor}", first.job.id), "default")
        .send()
        .await?;
    drop(events);
    server.respond(first.job.id, approval).await?;
    server.wait(first.job.id, JobState::Completed).await?;
    server.wait(second.id, JobState::Waiting).await?;
    ensure!(server.job(third.id).await?.job.state == JobState::Queued, "Account ordering was not serialized");
    let (approval, _) = server.pending(second.id, 0).await?;
    server.respond(second.id, approval).await?;
    server.wait(second.id, JobState::Completed).await?;
    server.wait(third.id, JobState::Waiting).await?;
    let (approval, _) = server.pending(third.id, 0).await?;
    server.respond(third.id, approval).await?;
    server.wait(third.id, JobState::Completed).await?;
    let saved: SavedConversation = server
        .request(Method::GET, &format!("{route}/conversations/{conversation}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let user: Vec<_> = saved
        .messages
        .iter()
        .filter(|message| message.user)
        .map(|message| message.text.as_str())
        .collect();
    ensure!(user == ["first-account-A", "second-A", "third-A"], "Wrong queue/resume order: {user:?}");
    let report: UsageReport = server
        .request(Method::GET, &format!("{route}/conversations/{conversation}/usage"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if provider == Provider::Antigravity {
        ensure!(report.usage.is_none() && report.missing_turns == 3);
    } else {
        ensure!(report.reported_turns == 3 && report.missing_turns == 0);
        ensure!(report.usage.as_ref().unwrap().total_tokens == Some(if provider == Provider::Claude { 48 } else { 42 }));
    }
    ensure!(
        server
            .request(Method::GET, &format!("{route}/conversations/{conversation}/usage"), "../bad")
            .send()
            .await?
            .status()
            == StatusCode::BAD_REQUEST
    );

    let isolated: UsageReport = server
        .request(
            Method::GET,
            &format!("{route}/conversations/{}/usage", other.job.conversation_id.unwrap()),
            &account.id,
        )
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(isolated.usage.is_none(), "Usage leaked across account profiles");
    let resumed: Vec<ConversationSummary> = server
        .request(Method::GET, &format!("{route}/conversations"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(resumed.len() == 1 && resumed[0].pinned && !resumed[0].archived);

    server
        .request(Method::DELETE, &format!("/v1/jobs/{}", other.job.id), "default")
        .send()
        .await?
        .error_for_status()?;
    server.wait(other.job.id, JobState::Cancelled).await?;
    let (stale, _) = server.pending(other.job.id, 0).await?;
    ensure!(
        server
            .request(Method::POST, &format!("/v1/jobs/{}/interactions/{stale}", other.job.id), "default")
            .json(&json!({"type":"approval","allow":true}))
            .send()
            .await?
            .status()
            == StatusCode::NOT_FOUND
    );
    let replay = server
        .request(Method::GET, &format!("/v1/jobs/{}/events?after={cursor}", first.job.id), "default")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let events: Vec<JobEvent> = replay.lines().map(serde_json::from_str).collect::<std::result::Result<_, _>>()?;
    ensure!(events.iter().all(|event| event.sequence > cursor));
    ensure!(matches!(events.last().map(|e| &e.event), Some(StreamEvent::Completed { .. })));

    server
        .request(Method::DELETE, &format!("{route}/conversations/{conversation}"), "default")
        .send()
        .await?
        .error_for_status()?;
    for id in [first.job.id, second.id, third.id] {
        ensure!(server.request(Method::GET, &format!("/v1/jobs/{id}"), "default").send().await?.status() == StatusCode::NOT_FOUND);
    }
    ensure!(
        server.job(other.job.id).await?.job.state == JobState::Cancelled,
        "Deleting account A leaked into account B"
    );
    server.stop().await?;
    println!("ok: {provider:?} account isolation/concurrency, queue ordering/edit/cancel, status, cursor replay and native approvals");
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    if gpui_agent_api::run_mcp_bridge().await? {
        return Ok(());
    }
    if gpui_agent::run_browser_helper() {
        return Ok(());
    }
    if let Ok(spec) = fs::read_to_string("fixture.json") {
        return native::fixture(serde_json::from_str(&spec)?);
    }
    native::test_arguments(&[
        "--guidance-only",
        "--profile-contention-only",
        "--client-fixture",
        "--client-fixture-host",
        "--computer-fixture",
        "--computer-fixture-host",
        "--live-computer",
        "--live-computer-host",
        "--live-computer-automatic",
        "--mcp-only",
        "--computer-only",
        "--native-tools-only",
        "--codex-policy-only",
        "--remote-mcp-only",
        "--oauth-mcp-only",
        "--conversations-only",
        "--attachments-only",
        "--uploads-only",
        "--context-transfers-only",
        "--questions-only",
        "--recovery-only",
        "--long-stream-only",
        "--mcp-sessions-only",
        "--proposals-only",
    ])?;
    if std::env::args().any(|a| a == "--client-fixture") {
        return client_fixture::supervise();
    }
    if std::env::args().any(|a| a == "--client-fixture-host") {
        return client_fixture::run().await;
    }
    if std::env::args().any(|a| a == "--computer-fixture") {
        return client_fixture::supervise();
    }
    if std::env::args().any(|a| a == "--computer-fixture-host") {
        return computer::serve_fixture().await;
    }
    if std::env::args().any(|a| a == "--live-computer") {
        return client_fixture::supervise();
    }
    if std::env::args().any(|a| a == "--live-computer-host") {
        return live_computer::run().await;
    }
    if std::env::args().any(|a| a == "--live-computer-automatic") {
        return live_computer::automatic().await;
    }
    if std::env::args().any(|a| a == "--guidance-only") {
        return shared_guidance_jobs().await;
    }
    if std::env::args().any(|a| a == "--profile-contention-only") {
        return profile_contention::run().await;
    }
    if std::env::args().any(|a| a == "--mcp-only") {
        return mcp_contract().await;
    }
    if std::env::args().any(|a| a == "--computer-only") {
        return computer::run().await;
    }
    if std::env::args().any(|a| a == "--native-tools-only") {
        return native_tools().await;
    }
    if std::env::args().any(|a| a == "--codex-policy-only") {
        return codex_policy_configuration().await;
    }
    if std::env::args().any(|a| a == "--remote-mcp-only") {
        return remote_mcp_configuration().await;
    }
    if std::env::args().any(|a| a == "--oauth-mcp-only") {
        return oauth_mcp_configuration().await;
    }
    if std::env::args().any(|a| a == "--conversations-only") {
        return mcp_conversations().await;
    }
    if std::env::args().any(|a| a == "--attachments-only") {
        mcp_inline_attachments().await?;
        return mcp_uploaded_attachments().await;
    }
    if std::env::args().any(|a| a == "--uploads-only") {
        return mcp_uploaded_attachments().await;
    }
    if std::env::args().any(|a| a == "--context-transfers-only") {
        return mcp_context_transfers().await;
    }
    if std::env::args().any(|a| a == "--questions-only") {
        return mcp_questions().await;
    }
    if std::env::args().any(|a| a == "--recovery-only") {
        return recovery().await;
    }
    if std::env::args().any(|a| a == "--long-stream-only") {
        return long_stream().await;
    }
    if std::env::args().any(|a| a == "--mcp-sessions-only") {
        return mcp_session_lifecycle().await;
    }
    if std::env::args().any(|a| a == "--proposals-only") {
        return proposal_continuation().await;
    }
    mcp_session_lifecycle().await?;
    proposal_continuation().await?;
    computer::run().await?;
    mcp_contract().await?;
    profile_contention::run().await?;
    shared_guidance_jobs().await?;
    mcp_conversations().await?;
    mcp_inline_attachments().await?;
    mcp_uploaded_attachments().await?;
    mcp_context_transfers().await?;
    mcp_questions().await?;
    native_tools().await?;
    codex_policy_configuration().await?;
    remote_mcp_configuration().await?;
    oauth_mcp_configuration().await?;
    for provider in PROVIDERS {
        lifecycle(provider).await?;
        mcp_profile_settings(provider).await?;
        revised_background_job(provider).await?;
    }
    recovery().await?;
    long_stream().await?;
    multiple_projects().await?;
    Ok(())
}

// Given an agent that proposed its own skill or MCP server, when the operator
// approves, then the same conversation continues in a new queued turn so the
// next turn starts with the new configuration; rejection continues nothing.
async fn proposal_continuation() -> Result<()> {
    let server = Server::open(Provider::Codex).await?;
    server.login(Provider::Codex, "default").await?;
    let first = server.enqueue(Provider::Codex, "default", "Set up what you need", None).await?;
    let (approval, _) = server.pending(first.id, 0).await?;
    server.respond(first.id, approval).await?;
    let done = server.wait(first.id, JobState::Completed).await?;
    let conversation = done.job.conversation_id.context("Missing conversation")?;
    let session = server.sessions.issue_in_conversation(
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: done.job.scope.workspace_path.clone(),
        },
        true,
        Some(conversation),
    )?;
    let token = &session.connection().token;
    let decide = |kind: &'static str, id: String, allow: bool| {
        let request = server.request(Method::POST, &format!("/v1/{kind}-proposals/{id}"), "default");
        async move { anyhow::Ok(request.json(&json!({"allow":allow})).send().await?.error_for_status()?) }
    };
    let follow_ups = || async {
        let jobs: Vec<Job> = server
            .request(Method::GET, "/v1/jobs", "default")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        anyhow::Ok(
            jobs.into_iter()
                .filter(|job| job.id != first.id && job.conversation_id == Some(conversation))
                .collect::<Vec<_>>(),
        )
    };

    // Rejecting changes nothing, so no turn is queued.
    let rejected = server
        .mcp(
            "host_skill_propose",
            json!({"name":"declined","content":"---\ndescription: Declined\n---\nBody"}),
            token,
        )
        .await?;
    let rejected = rejected["result"]["structuredContent"]["id"]
        .as_str()
        .context("Missing proposal id")?
        .to_owned();
    decide("skill", rejected, false).await?;
    ensure!(follow_ups().await?.is_empty(), "A rejected proposal must not continue the conversation");

    // Approving queues exactly one continuation in the proposing conversation.
    let proposed = server
        .mcp(
            "host_skill_propose",
            json!({"name":"helper","content":"---\ndescription: Help with the task\n---\nBody"}),
            token,
        )
        .await?;
    let id = proposed["result"]["structuredContent"]["id"]
        .as_str()
        .context("Missing proposal id")?
        .to_owned();
    // The operator sees what the user last asked, recorded by the host rather
    // than supplied by the agent.
    let pending: serde_json::Value = server
        .request(Method::GET, "/v1/skill-proposals", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        pending[0]["requested_during"] == "Set up what you need",
        "The operator must see the user's latest request: {pending}"
    );
    decide("skill", id.clone(), true).await?;
    let queued = follow_ups().await?;
    ensure!(queued.len() == 1, "Expected one continuation, got {queued:?}");
    let detail = server.wait(queued[0].id, JobState::Waiting).await?;
    ensure!(
        detail.request.text.contains(&id) && detail.request.text.contains("host_skills_list"),
        "{:?}",
        detail.request.text
    );
    ensure!(detail.request.conversation_id == Some(conversation));
    let (approval, _) = server.pending(queued[0].id, 0).await?;
    server.respond(queued[0].id, approval).await?;
    let finished = server.wait(queued[0].id, JobState::Completed).await?;
    ensure!(
        finished.job.conversation_id == Some(conversation),
        "The continuation must reuse the conversation"
    );

    // MCP servers use the same path, naming the plugin that was enabled.
    let executable = std::env::current_exe()?.to_string_lossy().into_owned();
    let proposed = server
        .mcp(
            "host_mcp_propose",
            json!({"server":{"name":"fixture_tools","command":executable},"rationale":"Needed to finish the task"}),
            token,
        )
        .await?;
    let id = proposed["result"]["structuredContent"]["id"]
        .as_str()
        .context("Missing proposal id")?
        .to_owned();
    decide("mcp", id.clone(), true).await?;
    let queued = follow_ups().await?;
    ensure!(queued.len() == 2, "Expected a second continuation, got {queued:?}");
    let newest = queued.iter().max_by_key(|job| job.id).context("Missing continuation")?;
    let detail = server.job(newest.id).await?;
    ensure!(
        detail.request.text.contains(&id) && detail.request.text.contains("repo:mcp-fixture-tools"),
        "{:?}",
        detail.request.text
    );
    server.stop().await
}

async fn mcp_session_lifecycle() -> Result<()> {
    use gpui_agent_application::ports::McpSessionAuthorizer;
    for provider in PROVIDERS {
        let server = Server::open_scenario(provider, "mcp-session").await?;
        ensure!(server.sessions.active_sessions()? == 0);
        server.login(provider, "default").await?;
        ensure!(
            server.sessions.active_sessions()? == 0,
            "Authentication must not receive orchestration credentials"
        );
        let job = server.enqueue(provider, "default", "session work", None).await?;
        let waiting = server.wait(job.id, JobState::Waiting).await?;
        ensure!(server.sessions.active_sessions()? == 1, "Provider prompt must own one transient session");
        server.respond(job.id, waiting.job.pending_request_id.context("approval")?).await?;
        let done = server.wait(job.id, JobState::Completed).await?;
        ensure!(server.sessions.active_sessions()? == 0, "Completion must revoke credentials");
        let resumed = server.enqueue(provider, "default", "resume session", done.job.conversation_id).await?;
        server.wait(resumed.id, JobState::Waiting).await?;
        ensure!(server.sessions.active_sessions()? == 1, "Resume must mint a new session");
        server
            .request(Method::DELETE, &format!("/v1/jobs/{}", resumed.id), "default")
            .send()
            .await?
            .error_for_status()?;
        server.wait(resumed.id, JobState::Cancelled).await?;
        ensure!(server.sessions.active_sessions()? == 0, "Cancellation must revoke credentials");
        let active = server.enqueue(provider, "default", "shutdown session", None).await?;
        let waiting = server.wait(active.id, JobState::Waiting).await?;
        let sessions = server.sessions.clone();
        let scope = McpScope {
            provider,
            account_id: "default".into(),
            workspace_path: waiting.job.scope.workspace_path,
        };
        let explicit = sessions.issue(scope.clone(), false)?;
        let bearer = explicit.connection().token.clone();
        server.stop().await?;
        ensure!(sessions.resolve(&bearer)?.is_none() && sessions.active_sessions()? == 0);
        ensure!(sessions.issue(scope, false).is_err());
    }
    println!("ok: three providers automatically mint/revoke scoped MCP on prompt, resume, cancellation and shutdown");
    for (provider, scenario) in [
        (Provider::Codex, "failed"),
        (Provider::Codex, "timeout"),
        (Provider::Antigravity, "mcp-unsupported"),
    ] {
        let server = Server::open_scenario(provider, scenario).await?;
        if scenario == "timeout" {
            fs::write(server.root.path().join("home/codex/fixture-auth"), "fixture")?;
        } else {
            server.login(provider, "default").await?;
        }
        let job = server.enqueue(provider, "default", "failure session", None).await?;
        if scenario == "failed" {
            let waiting = server.wait(job.id, JobState::Waiting).await?;
            server
                .request(
                    Method::POST,
                    &format!("/v1/jobs/{}/interactions/{}", job.id, waiting.job.pending_request_id.context("approval")?),
                    "default",
                )
                .json(&json!({"type":"approval","allow":false}))
                .send()
                .await?
                .error_for_status()?;
        }
        server.wait(job.id, JobState::Failed).await?;
        // Given a runtime that fails before approval, when its stream closes,
        // then the assertion identifies the failed job without exposing its
        // input.
        if scenario != "failed" {
            let error = server
                .pending(job.id, 0)
                .await
                .expect_err("failed runtime must not request approval")
                .to_string();
            ensure!(
                error.contains(&format!("job={}", job.id)) && error.contains(&format!("provider={provider:?}")) && error.contains("state=Failed"),
                "Missing safe failure context: {error}"
            );
            ensure!(!error.contains("failure session") && !error.contains(TOKEN));
        }
        ensure!(server.sessions.active_sessions()? == 0, "Failure/timeout must revoke credentials");
        server.stop().await?;
    }
    Ok(())
}

async fn mcp_contract() -> Result<()> {
    let server = Server::open(Provider::Codex).await?;
    let response = server
        .request(Method::POST, "/mcp", "default")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}
        }}))
        .send()
        .await?;
    ensure!(response.status() == StatusCode::OK, "MCP initialization returned {}", response.status());
    let initialized: serde_json::Value = response.json().await?;
    ensure!(initialized["id"] == 1 && initialized["result"]["protocolVersion"] == "2025-11-25");
    let listed: serde_json::Value = server
        .request(Method::POST, "/mcp", "default")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        listed["result"]["tools"]
            .as_array()
            .context("Missing tools")?
            .iter()
            .any(|tool| tool["name"] == "list_jobs")
    );
    for name in ["get_pending_questions", "answer_questions"] {
        ensure!(listed["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == name));
    }
    ensure!(
        !listed["result"]["tools"].as_array().unwrap().iter().any(|tool| {
            let name = tool["name"].as_str().unwrap_or_default();
            name.starts_with("automation_") || name.contains("secret")
        }),
        "removed automation and secret-request tools must not be advertised"
    );
    let jobs: serde_json::Value = server
        .request(Method::POST, "/mcp", "default")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_jobs","arguments":{}}}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(jobs["result"]["isError"] == false && jobs["result"]["structuredContent"]["jobs"] == json!([]));
    ensure!(server.client.post(format!("{}/mcp", server.url)).json(&json!({})).send().await?.status() == StatusCode::UNAUTHORIZED);
    ensure!(
        server
            .request(Method::POST, "/mcp", "default")
            .header("Origin", "https://evil.invalid")
            .json(&json!({}))
            .send()
            .await?
            .status()
            == StatusCode::FORBIDDEN
    );
    let readonly: serde_json::Value = server
        .client
        .post(format!("{}/mcp", server.url))
        .bearer_auth(MCP_READ_TOKEN)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        readonly["result"]["tools"]
            .as_array()
            .context("Missing readonly tools")?
            .iter()
            .all(|tool| tool["annotations"]["readOnlyHint"] == true)
    );
    ensure!(
        server
            .client
            .get(format!("{}/v1/jobs", server.url))
            .bearer_auth(MCP_READ_TOKEN)
            .send()
            .await?
            .status()
            == StatusCode::UNAUTHORIZED
    );
    server.login(Provider::Codex, "default").await?;
    let input = json!({"provider":"codex","text":"MCP work","command_id":"mcp-command-1"});
    let denied = server.mcp("enqueue_job", input.clone(), MCP_READ_TOKEN).await?;
    ensure!(denied["error"]["code"] == -32001, "Read-only token must not start work: {denied}");
    let (first, retry) = tokio::try_join!(server.mcp("enqueue_job", input.clone(), TOKEN), server.mcp("enqueue_job", input.clone(), TOKEN))?;
    let id = first["result"]["structuredContent"]["id"].as_i64().context(format!("Missing job: {first}"))?;
    ensure!(retry["result"]["structuredContent"]["id"] == id);
    let conflict = server
        .mcp("enqueue_job", json!({"provider":"codex","text":"changed","command_id":"mcp-command-1"}), TOKEN)
        .await?;
    ensure!(conflict["result"]["isError"] == true);
    ensure!(
        server.mcp("list_jobs", json!({}), MCP_READ_TOKEN).await?["result"]["structuredContent"]["jobs"]
            .as_array()
            .context("Missing jobs")?
            .len()
            == 1
    );
    let cancel = server.mcp("cancel_job", json!({"id":id}), TOKEN).await?;
    ensure!(cancel["result"]["isError"] == false);
    let terminal = server
        .request(Method::GET, &format!("/v1/jobs/{id}/events"), "default")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    ensure!(terminal.contains("cancelled"), "Missing durable cancellation: {terminal}");
    let saved = server.mcp("get_job", json!({"id":id}), MCP_READ_TOKEN).await?;
    ensure!(saved["result"]["structuredContent"]["job"]["state"] == "cancelled");
    {
        // Given concurrent retries, diagnostics must link each call to the same
        // durable job.
        let records = server.diagnostics.lock().unwrap();
        let submissions: Vec<_> = records.iter().filter(|r| r.tool == "enqueue_job" && r.ok).collect();
        ensure!(submissions.len() == 2 && submissions.iter().all(|r| r.job_id == Some(id)));
        ensure!(submissions[0].request_id != submissions[1].request_id);
        let read = records.iter().rev().find(|r| r.tool == "get_job").context("Missing read diagnostic")?;
        ensure!(read.job_id == Some(id));
        ensure!(serde_json::to_value(read.operation_id)? == saved["result"]["structuredContent"]["job"]["operation_id"]);
        let encoded = serde_json::to_string(&*records)?;
        ensure!(!encoded.contains(TOKEN) && !encoded.contains(MCP_READ_TOKEN) && !encoded.contains("MCP work"));
    }
    let replay = server.mcp("get_job_events", json!({"id":id}), MCP_READ_TOKEN).await?;
    let events = replay["result"]["structuredContent"]["events"].as_array().context("Missing persisted events")?;
    let cursor = events.last().context("Missing terminal event")?["sequence"]
        .as_i64()
        .context("Missing cursor")?;
    ensure!(events.last().context("Missing terminal event")?["event"]["type"] == "cancelled");
    ensure!(server.mcp("get_job_events", json!({"id":id,"after":cursor}), MCP_READ_TOKEN).await?["result"]["structuredContent"]["events"] == json!([]));
    let replayed: Job = server
        .request(Method::POST, "/v1/providers/codex/jobs", "default")
        .header("Idempotency-Key", "mcp-command-1")
        .json(&json!({"text":"MCP work"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(replayed.id == id, "HTTP and MCP retries must share the same durable receipt");
    let scope = McpScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_path: replayed.scope.workspace_path.clone(),
    };
    let session = server.sessions.issue(scope.clone(), false)?;
    let bearer = session.connection().token.clone();
    let frames = bridge_probe(session.connection()).await?;
    ensure!(frames.len() == 2 && frames[0]["result"]["tools"].is_array() && frames[1]["result"]["isError"] == false);
    ensure!(server.mcp("get_job", json!({"id":id}), &bearer).await?["result"]["isError"] == false);
    ensure!(server.mcp("cancel_job", json!({"id":id}), &bearer).await?["error"]["code"] == -32001);
    let alien = server.sessions.issue(
        McpScope {
            account_id: "a".repeat(32),
            ..scope.clone()
        },
        true,
    )?;
    ensure!(server.mcp("list_jobs", json!({}), &alien.connection().token).await?["result"]["structuredContent"]["jobs"] == json!([]));
    ensure!(server.mcp("get_job", json!({"id":id}), &alien.connection().token).await?["result"]["isError"] == true);
    ensure!(server.mcp("cancel_job", json!({"id":id}), &alien.connection().token).await?["result"]["isError"] == true);
    let writer = server.sessions.issue(scope.clone(), true)?;
    ensure!(
        server
            .mcp(
                "enqueue_job",
                json!({"provider":"claude","text":"escape","command_id":"escape"}),
                &writer.connection().token
            )
            .await?["result"]["isError"]
            == true
    );
    let scoped_retry = server.mcp("enqueue_job", input.clone(), &writer.connection().token).await?;
    ensure!(
        scoped_retry["result"]["structuredContent"]["id"] == id,
        "Session writes must resolve their own workspace: {scoped_retry}"
    );
    drop(session);
    ensure!(
        server
            .client
            .post(format!("{}/mcp", server.url))
            .bearer_auth(&bearer)
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":5,"method":"ping"}))
            .send()
            .await?
            .status()
            == StatusCode::UNAUTHORIZED
    );
    for input in [json!({"id":0}), json!({"id":id,"extra":true})] {
        ensure!(server.mcp("get_job", input, TOKEN).await?["result"]["isError"] == true);
    }
    ensure!(server.mcp("missing_tool", json!({}), TOKEN).await?["error"]["code"] == -32602);
    let notification = server
        .request(Method::POST, "/mcp", "default")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send()
        .await?;
    ensure!(notification.status() == StatusCode::ACCEPTED && notification.bytes().await?.is_empty());
    ensure!(server.request(Method::GET, "/mcp", "default").send().await?.status() == StatusCode::METHOD_NOT_ALLOWED);
    ensure!(
        server
            .request(Method::POST, "/mcp", "default")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "unsupported")
            .json(&json!({"jsonrpc":"2.0","id":9,"method":"ping"}))
            .send()
            .await?
            .status()
            == StatusCode::BAD_REQUEST
    );
    server.stop().await?;
    println!("ok: authenticated MCP initialization, discovery and application-service reads");
    Ok(())
}

async fn bridge_probe(connection: &gpui_agent_application::ports::McpConnection) -> Result<Vec<serde_json::Value>> {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new(std::env::current_exe()?)
        .arg("--mcp-stdio")
        .env("GPUI_AGENT_MCP_URL", &connection.url)
        .env("GPUI_AGENT_MCP_TOKEN", &connection.token)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut input = child.stdin.take().context("bridge stdin")?;
    input.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"list_jobs\",\"arguments\":{}}}\n").await?;
    drop(input);
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output()).await??;
    ensure!(output.status.success(), "bridge failed: {}", String::from_utf8_lossy(&output.stderr));
    ensure!(!String::from_utf8_lossy(&output.stdout).contains(&connection.token));
    ensure!(!String::from_utf8_lossy(&output.stderr).contains(&connection.token));
    String::from_utf8(output.stdout)?
        .lines()
        .map(|line| serde_json::from_str(line).map_err(Into::into))
        .collect()
}

async fn revised_background_job(provider: Provider) -> Result<()> {
    let server = Server::open(provider).await?;
    server.login(provider, "default").await?;
    let route = format!("/v1/providers/{}", provider_name(provider));
    let original = server.enqueue(provider, "default", "Original question", None).await?;
    let waiting = server.wait(original.id, JobState::Waiting).await?;
    server.respond(original.id, waiting.job.pending_request_id.context("Missing approval")?).await?;
    let done = server.wait(original.id, JobState::Completed).await?;
    let id = done.job.conversation_id.context("Missing conversation")?;
    let source: SavedConversation = server
        .request(Method::GET, &format!("{route}/conversations/{id}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let prepared: PreparedRevision = server
        .request(Method::POST, &format!("{route}/conversations/{id}/revisions"), "default")
        .json(&ReviseRequest {
            extensions: None,
            skills: None,
            message_id: source.messages[0].id.unwrap(),
            text: "Revised question".into(),
            model: None,
            claude_permission_mode: None,
            codex_execution_policy: None,
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(prepared.conversation.messages.is_empty() && prepared.request.session_id.is_none());
    let job: Job = server
        .request(Method::POST, &format!("{route}/jobs"), "default")
        .json(&prepared.request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let waiting = server.wait(job.id, JobState::Waiting).await?;
    server
        .respond(job.id, waiting.job.pending_request_id.context("Missing revised approval")?)
        .await?;
    let revised = server.wait(job.id, JobState::Completed).await?;
    ensure!(revised.job.conversation_id == Some(prepared.conversation.id) && revised.job.conversation_id != Some(id));
    let saved: SavedConversation = server
        .request(Method::GET, &format!("{route}/conversations/{}", prepared.conversation.id), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(saved.messages.len() == 2 && saved.messages[0].text == "Revised question" && !saved.messages[1].incomplete);
    let source: SavedConversation = server
        .request(Method::GET, &format!("{route}/conversations/{id}"), "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(source.messages.len() == 2 && source.messages[0].text == "Original question");
    server.stop().await?;
    println!("ok: {provider:?} revised first question runs in a separate background conversation and preserves its original");
    Ok(())
}
async fn multiple_projects() -> Result<()> {
    let server = Server::open(Provider::Codex).await?;
    let catalog: ProjectCatalog = server
        .request(Method::GET, "/v1/projects", "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let a = catalog.default_workspace_id.context("Missing default workspace")?;
    let path_b = server.root.path().join("second project 한글");
    fs::create_dir(&path_b)?;
    fs::write(
        path_b.join("fixture.json"),
        json!({"provider":"codex","scenario":"interactive-allow"}).to_string(),
    )?;
    let catalog: ProjectCatalog = server
        .request(Method::POST, "/v1/projects", "default")
        .json(&json!({"path":path_b}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let b = catalog.registered_workspace_id.context("Missing registered workspace")?;
    let account: AccountProfile = server
        .request(Method::POST, "/v1/providers/codex/accounts", "default")
        .json(&json!({"label":"Parallel profile"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    server.login(Provider::Codex, "default").await?;
    server.login(Provider::Codex, &account.id).await?;
    let enqueue = |workspace, account: &str, text: &str| {
        server
            .request(Method::POST, "/v1/providers/codex/jobs", account)
            .header("X-KND-Workspace", workspace)
            .json(&json!({"text":text}))
    };
    let first: Job = enqueue(a, "default", "project A").send().await?.error_for_status()?.json().await?;
    server.wait(first.id, JobState::Waiting).await?;
    // Closing the event subscription models leaving the first tab.
    let (approval_a, _) = server.pending(first.id, 0).await?;
    let queued: Job = enqueue(b, "default", "project B same profile").send().await?.error_for_status()?.json().await?;
    let parallel: Job = enqueue(b, &account.id, "project B parallel").send().await?.error_for_status()?.json().await?;
    server.wait(parallel.id, JobState::Waiting).await?;
    ensure!(server.job(first.id).await?.job.state == JobState::Waiting);
    ensure!(
        server.job(queued.id).await?.job.state == JobState::Queued,
        "Same profile must retain its runtime lock across projects"
    );
    let jobs: Vec<Job> = server
        .request(Method::GET, "/v1/jobs", "default")
        .header("X-KND-Workspace", b)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        jobs.iter()
            .any(|job| job.id == first.id && job.scope.workspace_id == Some(a) && job.state == JobState::Waiting)
    );
    ensure!(
        jobs.iter()
            .any(|job| job.id == parallel.id && job.scope.workspace_id == Some(b) && job.state == JobState::Waiting)
    );
    let (approval_b, _) = server.pending(parallel.id, 0).await?;
    server.respond(parallel.id, approval_b).await?;
    server.wait(parallel.id, JobState::Completed).await?;
    ensure!(
        server.job(first.id).await?.job.state == JobState::Waiting,
        "Completing B must not resolve A's approval"
    );
    // Job IDs retain their original scope even while the client is viewing B.
    server
        .request(Method::POST, &format!("/v1/jobs/{}/interactions/{approval_a}", first.id), "default")
        .header("X-KND-Workspace", b)
        .json(&json!({"type":"approval","allow":true}))
        .send()
        .await?
        .error_for_status()?;
    let first = server.wait(first.id, JobState::Completed).await?;
    let queued = server.wait(queued.id, JobState::Waiting).await?;
    ensure!(first.job.scope.workspace_id == Some(a));
    ensure!(queued.job.scope.workspace_id == Some(b));
    ensure!(fs::canonicalize(&queued.job.scope.workspace_path)? == fs::canonicalize(&path_b)?);
    let (approval, _) = server.pending(queued.job.id, 0).await?;
    server.respond(queued.job.id, approval).await?;
    server.wait(queued.job.id, JobState::Completed).await?;
    for (workspace, expected) in [(a, first.job.conversation_id), (b, queued.job.conversation_id)] {
        let conversations: Vec<ConversationSummary> = server
            .request(Method::GET, "/v1/providers/codex/conversations", "default")
            .header("X-KND-Workspace", workspace)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(
            conversations.len() == 1 && Some(conversations[0].id) == expected,
            "Conversation history crossed projects"
        );
    }
    server.stop().await?;
    println!("ok: shared server, two projects, disconnected subscriber, scoped approvals/history and concurrent profiles");
    Ok(())
}
async fn recovery() -> Result<()> {
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let app = Application::new(
        std::array::from_fn(|_| Err(anyhow::anyhow!("unavailable"))),
        Arc::new(gpui_agent_infrastructure::History::open(&settings.home.join("history.sqlite3"), root.path())?),
    );
    let workspaces = settings.workspaces(app)?;
    let scope = JobScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_id: Some(workspaces.default_workspace().unwrap()),
        workspace_path: root.path().display().to_string(),
    };
    let store = JobStore::open(settings.clone(), workspaces.clone())?;
    ensure!(
        JobStore::open(settings.clone(), workspaces.clone()).is_err(),
        "A second host must not recover another host's running jobs"
    );
    let request = ChatRequest {
        extensions: None,
        skills: vec![],
        computer_use: false,
        claude_permission_mode: None,
        codex_execution_policy: None,
        text: "persist queue".into(),
        attachments: Vec::new(),
        model: None,
        session_id: None,
        conversation_id: None,
    };
    let first = store.enqueue_once(&scope, &request, "recovery-command")?;
    ensure!(store.enqueue_once(&scope, &request, "recovery-command")?.id == first.id);
    let mut different = request.clone();
    different.text = "a different command".into();
    ensure!(store.enqueue_once(&scope, &different, "recovery-command").is_err());
    let mut other_scope = scope.clone();
    other_scope.provider = Provider::Claude;
    ensure!(store.enqueue_once(&other_scope, &request, "recovery-command").is_err());
    for key in ["", "contains spaces", "contains/slash"] {
        ensure!(store.enqueue_once(&scope, &request, key).is_err());
    }
    let second = store.enqueue(&scope, &request)?;
    ensure!(store.claim(&scope)?.unwrap().job.id == first.id);
    ensure!(!store.cancel_queued(first.id)?);
    ensure!(store.get(first.id)?.job.cancel_requested);
    drop(store);
    let store = JobStore::open(settings, workspaces)?;
    ensure!(store.enqueue_once(&scope, &request, "recovery-command")?.id == first.id);
    ensure!(store.list()?.len() == 2, "A retried command must not create another job");
    ensure!(store.get(first.id)?.job.state == JobState::Interrupted);
    ensure!(store.get(second.id)?.job.state == JobState::Queued);
    ensure!(
        store
            .events(first.id, 0)?
            .last()
            .is_some_and(|event| matches!(event.event, StreamEvent::Error { .. }))
    );
    ensure!(store.claim(&scope)?.unwrap().job.id == second.id);
    println!("ok: durable queued jobs, exclusive host ownership, cancellation-before-start and interrupted-turn recovery");
    Ok(())
}

async fn long_stream() -> Result<()> {
    let server = Server::open_scenario(Provider::Antigravity, "long-stream").await?;
    server.login(Provider::Antigravity, "default").await?;
    let job = server.enqueue(Provider::Antigravity, "default", "long stream", None).await?;
    server.wait(job.id, JobState::Waiting).await?;
    let (request, _) = server.pending(job.id, 0).await?;
    server
        .request(Method::POST, &format!("/v1/jobs/{}/interactions/{request}", job.id), "default")
        .json(&json!({"type":"approval","allow":false}))
        .send()
        .await?
        .error_for_status()?;
    server.wait(job.id, JobState::Completed).await?;
    let journal = server
        .request(Method::GET, &format!("/v1/jobs/{}/events", job.id), "default")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let events: Vec<JobEvent> = journal.lines().map(serde_json::from_str).collect::<std::result::Result<_, _>>()?;
    ensure!(
        events
            .iter()
            .filter(|event| matches!(event.event, StreamEvent::Agent(Event::Text { .. })))
            .count()
            == 1500
    );
    server.stop().await?;
    println!("ok: 1500 queued streaming chunks are journaled without loss");
    Ok(())
}

async fn git_features(server: &Server, workspace: i64) -> Result<()> {
    let repo = server.root.path().join("project 한글");
    let git = |args: &[&str]| -> Result<()> {
        let out = std::process::Command::new("git").arg("-C").arg(&repo).args(args).output()?;
        ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        Ok(())
    };
    git(&["init", "-b", "main"])?;
    git(&["config", "user.name", "API Test"])?;
    git(&["config", "user.email", "api@example.invalid"])?;
    fs::write(repo.join("file.txt"), "original\n")?;
    git(&["add", "."])?;
    git(&["-c", "core.hooksPath=disabled-hooks", "-c", "commit.gpgSign=false", "commit", "-m", "initial"])?;
    let route = format!("/v1/workspaces/{workspace}/git");
    ensure!(server.client.get(format!("{}{route}", server.url)).send().await?.status() == StatusCode::UNAUTHORIZED);
    ensure!(
        server
            .request(Method::POST, &route, "default")
            .json(&json!({"type":"checkpoint","label":" "}))
            .send()
            .await?
            .status()
            == StatusCode::BAD_REQUEST
    );
    let saved: GitOverview = server
        .request(Method::POST, &route, "default")
        .json(&json!({"type":"checkpoint","label":"API snapshot 한글"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let key = &saved.checkpoints[0].id;
    fs::write(repo.join("file.txt"), "changed\n")?;
    let preview: GitOverview = server
        .request(Method::POST, &route, "default")
        .json(&json!({"type":"preview_restore","checkpoint":key}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(preview.diff.contains("original") && preview.diff.contains("changed"));
    ensure!(
        server
            .request(Method::POST, &route, "default")
            .json(&json!({"type":"restore","checkpoint":key,"expected":saved.version}))
            .send()
            .await?
            .status()
            == StatusCode::CONFLICT
    );
    let restored: GitOverview = server
        .request(Method::POST, &route, "default")
        .json(&json!({"type":"restore","checkpoint":key,"expected":preview.version}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(restored.backup.is_some() && fs::read_to_string(repo.join("file.txt"))? == "original\n");
    fs::write(repo.join("file.txt"), "staged\n")?;
    let before: GitOverview = server.request(Method::GET, &route, "default").send().await?.error_for_status()?.json().await?;
    let staged: GitOverview = server
        .request(Method::POST, &route, "default")
        .json(&json!({"type":"stage","path":"file.txt","staged":true,"expected":before.version}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(staged.diff.contains("staged"));
    let unstaged: GitOverview = server
        .request(Method::POST, &route, "default")
        .json(&json!({"type":"stage","path":"file.txt","staged":false,"expected":staged.version}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(unstaged.diff.is_empty());
    server
        .request(Method::POST, &route, "default")
        .json(&json!({"type":"stage","path":"file.txt","staged":true,"expected":unstaged.version}))
        .send()
        .await?
        .error_for_status()?;
    fs::write(repo.join("file.txt"), "unstaged\n")?;
    let inspect: GitOverview = server.request(Method::GET, &route, "default").send().await?.error_for_status()?.json().await?;
    ensure!(inspect.diff.contains("staged") && !inspect.diff.contains("unstaged"));
    let committed: GitOverview = server
        .request(Method::POST, &route, "default")
        .json(&json!({"type":"commit","message":"API commit","expected":inspect.version}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(committed.commit.is_some() && fs::read_to_string(repo.join("file.txt"))? == "unstaged\n");
    println!("ok: API checkpoints, preview/stale restore, recovery backup and staged-only commit");
    Ok(())
}

// Given a provider profile, MCP settings round-trip and cannot change while a
// job from that profile is waiting.
async fn mcp_profile_settings(provider: Provider) -> Result<()> {
    let server = Server::open(provider).await?;
    server.login(provider, "default").await?;
    let mcp_route = format!("/v1/mcp/{}", provider_name(provider));
    let servers = json!([{"name":"fixture","command":std::env::current_exe()?,"args":["--stdio"]}]);
    server
        .request(Method::PUT, &mcp_route, "default")
        .json(&servers)
        .send()
        .await?
        .error_for_status()?;
    let restored: serde_json::Value = server
        .request(Method::GET, &mcp_route, "default")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(restored == servers);
    let job = server.enqueue(provider, "default", "queued prompt", None).await?;
    server.wait(job.id, JobState::Waiting).await?;
    ensure!(server.request(Method::PUT, &mcp_route, "default").json(&json!([])).send().await?.status() == StatusCode::CONFLICT);
    let (approval, _) = server.pending(job.id, 0).await?;
    server
        .request(Method::POST, &format!("/v1/jobs/{}/interactions/{approval}", job.id), "default")
        .json(&json!({"type":"approval","allow":true}))
        .send()
        .await?
        .error_for_status()?;
    let completed = server.wait(job.id, JobState::Completed).await?;
    ensure!(completed.request.text == "queued prompt");
    server.stop().await?;
    println!("ok: {provider:?} MCP profile settings round-trip and busy-profile protection");
    Ok(())
}
