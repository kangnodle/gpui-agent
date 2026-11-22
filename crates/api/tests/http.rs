#[allow(dead_code)]
#[path = "../../agent/tests/protocols.rs"]
mod native;

use anyhow::{Context,
             Result,
             ensure};
use gpui_agent::{Agent,
                 Config,
                 Provider};
use gpui_agent_api::{Api,
                     StreamEvent};
use reqwest::StatusCode;
use serde_json::{Value,
                 json};
use std::{fs,
          time::Duration};

const TOKEN: &str = "testtokenislongenoughforalocalhttptest1234567890";

struct Harness {
    root: tempfile::TempDir,
    url: String,
    api: Api,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<()>>,
    client: reqwest::Client,
}

impl Harness {
    async fn new(scenario: &str) -> Result<Self> { Self::open(tempfile::tempdir()?, scenario).await }

    async fn open(root: tempfile::TempDir, scenario: &str) -> Result<Self> { Self::open_provider(root, scenario, Provider::Codex).await }

    async fn open_provider(root: tempfile::TempDir, scenario: &str, provider: Provider) -> Result<Self> {
        fs::write(
            root.path().join("fixture.json"),
            json!({"provider":gpui_agent_api::provider_name(provider),"scenario":scenario}).to_string(),
        )?;
        let config = Config {
            mcp_home: None,
            mcp_selection: None,
            plugin_mcp: vec![],
            provider,
            executable: std::env::current_exe()?,
            home: root.path().join("profile"),
            cwd: root.path().into(),
            harness: Some(std::env::current_exe()?),
            browser_helper: Some(std::env::current_exe()?),
            timeout: if scenario == "interactive-timeout" {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(30)
            },
        };
        let application = gpui_agent_infrastructure::application_from_agents(
            gpui_agent_api::PROVIDERS.map(|candidate| {
                if candidate == provider {
                    Agent::new(config.clone())
                } else {
                    Err(anyhow::anyhow!("Runtime unavailable"))
                }
            }),
            &root.path().join("history.sqlite3"),
            root.path(),
        )?;
        let api = Api::new(application, TOKEN.into())?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(gpui_agent_api::serve(listener, api.clone(), async {
            let _ = rx.await;
        }));
        Ok(Self {
            root,
            url,
            api,
            shutdown: Some(tx),
            task,
            client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(15)).build()?,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client.request(method, format!("{}{path}", self.url)).bearer_auth(TOKEN)
    }

    async fn stop(mut self) -> Result<tempfile::TempDir> {
        let _ = self.shutdown.take().unwrap().send(());
        self.task.await??;
        drop(self.api);
        Ok(self.root)
    }
}

async fn next_event(response: &mut reqwest::Response, buffer: &mut Vec<u8>) -> Result<Option<StreamEvent>> {
    loop {
        if let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<_> = buffer.drain(..= end).collect();
            return Ok(Some(serde_json::from_slice(&line)?));
        }
        let Some(chunk) = response.chunk().await? else {
            ensure!(buffer.is_empty(), "Truncated stream");
            return Ok(None);
        };
        buffer.extend_from_slice(&chunk);
        ensure!(buffer.len() < 16 * 1024 * 1024, "Event too large");
    }
}
async fn stream(harness: &Harness, mut response: reqwest::Response) -> Result<Vec<StreamEvent>> {
    ensure!(response.status().is_success());
    let mut events = Vec::new();
    let mut buffer = Vec::new();
    while let Some(event) = next_event(&mut response, &mut buffer).await? {
        if let StreamEvent::Interaction {
            operation_id,
            request_id,
            ..
        } = &event
        {
            harness
                .request(reqwest::Method::POST, &format!("/v1/operations/{operation_id}/interactions/{request_id}"))
                .json(&json!({"type":"approval","allow":false}))
                .send()
                .await?
                .error_for_status()?;
        }
        events.push(event);
    }
    Ok(events)
}

#[tokio::main]
async fn main() -> Result<()> {
    if gpui_agent::run_browser_helper() {
        return Ok(());
    }
    if let Ok(spec) = fs::read_to_string("fixture.json") {
        return native::fixture(serde_json::from_str(&spec)?);
    }
    native::test_arguments(&["--codex-running-only"])?;
    if std::env::args().any(|arg| arg == "--codex-running-only") {
        return interactive(Provider::Codex, "interactive-running").await;
    }
    let mut harness = Harness::new("normal").await?;
    let response = harness.client.get(format!("{}/v1/providers", harness.url)).send().await?;
    ensure!(response.status() == StatusCode::UNAUTHORIZED);
    let response = harness
        .request(reqwest::Method::GET, "/v1/providers")
        .header("Origin", "https://example.invalid")
        .send()
        .await?;
    ensure!(response.status() == StatusCode::UNAUTHORIZED);
    let providers: Vec<Value> = harness.request(reqwest::Method::GET, "/v1/providers").send().await?.json().await?;
    ensure!(providers.len() == gpui_agent_api::PROVIDERS.len() && providers[1]["available"] == true);
    ensure!(harness.request(reqwest::Method::GET, "/v1/providers/claude/status").send().await?.status() == StatusCode::SERVICE_UNAVAILABLE);
    ensure!(
        harness
            .request(reqwest::Method::POST, "/v1/providers/codex/chat")
            .json(&json!({"text":" ","model":null,"session_id":null}))
            .send()
            .await?
            .status()
            == StatusCode::BAD_REQUEST
    );
    let login = stream(
        &harness,
        harness
            .request(reqwest::Method::POST, "/v1/providers/codex/login")
            .json(&json!({}))
            .send()
            .await?,
    )
    .await?;
    ensure!(
        login
            .iter()
            .any(|event| matches!(event, StreamEvent::Agent(gpui_agent::Event::Authorization { .. })))
    );
    ensure!(matches!(login.last(), Some(StreamEvent::Authenticated {status}) if status.authenticated));
    let mut conversation_id = None;
    for resumed in [false, true] {
        if resumed {
            for query in ["수정한 제목", "  hELLo 세계  "] {
                let results: Vec<Value> = harness
                    .request(reqwest::Method::GET, "/v1/providers/codex/conversations")
                    .query(&[("q", query)])
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                ensure!(results.len() == 1 && results[0]["id"] == conversation_id.unwrap());
                conversation_id = results[0]["id"].as_i64();
                let saved: Value = harness
                    .request(reqwest::Method::GET, &format!("/v1/providers/codex/conversations/{}", conversation_id.unwrap()))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if query == "수정한 제목" {
                    ensure!(results[0]["matched_message"].is_null());
                } else {
                    ensure!(results[0]["matched_message"]["message_id"] == saved["messages"][1]["id"]);
                    ensure!(results[0]["matched_message"]["user"] == false);
                    ensure!(results[0]["matched_message"]["preview"] == "Hello 세계");
                }
            }
        }
        let events = stream(
            &harness,
            harness
                .request(reqwest::Method::POST, "/v1/providers/codex/chat")
                .json(&json!({"text":"hello","model":"fixture-model","conversation_id":if resumed {conversation_id} else {None}}))
                .send()
                .await?,
        )
        .await?;
        let mut streamed = String::new();
        for event in &events {
            if let StreamEvent::Agent(gpui_agent::Event::Text {
                delta,
            }) = event
            {
                streamed.push_str(delta);
            }
            if let StreamEvent::Conversation {
                conversation_id: id,
            } = event
            {
                if resumed {
                    ensure!(conversation_id == Some(*id));
                }
                conversation_id = Some(*id);
            }
        }
        match events.last() {
            | Some(StreamEvent::Completed {
                session_id,
                text,
            }) => {
                ensure!(text == "Hello 세계" && *text == streamed);
                ensure!(session_id == "codex-session");
            },
            | other => anyhow::bail!("missing successful completion: {other:?}"),
        }
        let id = conversation_id.context("missing saved conversation id")?;
        let saved: Value = harness
            .request(reqwest::Method::GET, &format!("/v1/providers/codex/conversations/{id}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(saved["session_id"] == "codex-session" && saved["model"] == "fixture-model");
        ensure!(saved["messages"].as_array().unwrap().len() == if resumed { 4 } else { 2 });
        ensure!(saved["messages"][1]["text"] == "Hello 세계" && saved["messages"][1]["incomplete"] == false);
        let list: Vec<Value> = harness
            .request(reqwest::Method::GET, "/v1/providers/codex/conversations")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(list.len() == 1 && list[0]["id"] == id);
        for query in ["missing", "%_", "' OR 1=1 --", "+ & 🦀"] {
            let results: Vec<Value> = harness
                .request(reqwest::Method::GET, "/v1/providers/codex/conversations")
                .query(&[("q", query)])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            ensure!(results.is_empty());
        }
        let other: Vec<Value> = harness
            .request(reqwest::Method::GET, "/v1/providers/claude/conversations")
            .query(&[("q", "Hello 세계")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(other.is_empty());
        ensure!(
            harness
                .request(reqwest::Method::GET, "/v1/providers/codex/conversations")
                .query(&[("q", "a".repeat(257))])
                .send()
                .await?
                .status()
                == StatusCode::BAD_REQUEST
        );
        if resumed {
            ensure!(list[0]["title"] == "수정한 제목 🦀");
        }
        ensure!(
            harness
                .request(reqwest::Method::GET, &format!("/v1/providers/claude/conversations/{id}"))
                .send()
                .await?
                .status()
                == StatusCode::NOT_FOUND
        );
        if !resumed {
            let route = format!("/v1/providers/codex/conversations/{id}");
            ensure!(harness.client.delete(format!("{}{route}", harness.url)).send().await?.status() == StatusCode::UNAUTHORIZED);
            for title in [" ".to_owned(), "a".repeat(101), "bad\ntitle".to_owned()] {
                ensure!(
                    harness
                        .request(reqwest::Method::PATCH, &route)
                        .json(&json!({"title":title}))
                        .send()
                        .await?
                        .status()
                        == StatusCode::BAD_REQUEST
                );
            }
            let wrong = format!("/v1/providers/claude/conversations/{id}");
            ensure!(
                harness
                    .request(reqwest::Method::PATCH, &wrong)
                    .json(&json!({"title":"wrong"}))
                    .send()
                    .await?
                    .status()
                    == StatusCode::NOT_FOUND
            );
            ensure!(harness.request(reqwest::Method::DELETE, &wrong).send().await?.status() == StatusCode::NOT_FOUND);
            ensure!(
                harness
                    .request(reqwest::Method::PATCH, &route)
                    .json(&json!({"title":"  수정한 제목 🦀  "}))
                    .send()
                    .await?
                    .status()
                    == StatusCode::NO_CONTENT
            );
            let root = harness.stop().await?;
            harness = Harness::open(root, "normal").await?;
        }
    }
    let models: Vec<Value> = harness.request(reqwest::Method::GET, "/v1/providers/codex/models").send().await?.json().await?;
    ensure!(models.len() == 2);
    harness
        .request(reqwest::Method::POST, "/v1/providers/codex/logout")
        .send()
        .await?
        .error_for_status()?;
    let status: Value = harness.request(reqwest::Method::GET, "/v1/providers/codex/status").send().await?.json().await?;
    ensure!(status["authenticated"] == false);
    let id = conversation_id.unwrap();
    let route = format!("/v1/providers/codex/conversations/{id}");
    ensure!(harness.request(reqwest::Method::DELETE, &route).send().await?.status() == StatusCode::NO_CONTENT);
    let root = harness.stop().await?;
    let harness = Harness::open(root, "normal").await?;
    ensure!(harness.request(reqwest::Method::GET, &route).send().await?.status() == StatusCode::NOT_FOUND);
    let list: Vec<Value> = harness
        .request(reqwest::Method::GET, "/v1/providers/codex/conversations")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(list.is_empty());
    harness.stop().await?;
    println!("ok: HTTP auth/origin guard, login, streaming, resume, models, logout, rename/delete and restart persistence");

    let harness = Harness::new("cancel").await?;
    fs::write(harness.root.path().join("profile/fixture-auth"), "fixture")?;
    let mut response = harness
        .request(reqwest::Method::POST, "/v1/providers/codex/chat")
        .json(&json!({"text":"cancel me","model":null,"session_id":null}))
        .send()
        .await?;
    let mut received = String::new();
    let mut id = None;
    loop {
        received.push_str(std::str::from_utf8(
            &response.chunk().await?.context("stream closed before native session started")?,
        )?);
        for line in received.lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && let Some(value) = value["operation_id"].as_u64()
            {
                id = Some(value);
            }
        }
        if received.contains("\"type\":\"session\"") {
            break;
        }
    }
    let id = id.context("missing operation id")?;
    let conversation_id = received
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|value| value["conversation_id"].as_i64())
        .context("missing conversation id")?;
    let route = format!("/v1/providers/codex/conversations/{conversation_id}");
    ensure!(
        harness
            .request(reqwest::Method::PATCH, &route)
            .json(&json!({"title":"busy"}))
            .send()
            .await?
            .status()
            == StatusCode::CONFLICT
    );
    ensure!(harness.request(reqwest::Method::DELETE, &route).send().await?.status() == StatusCode::CONFLICT);
    ensure!(harness.request(reqwest::Method::DELETE, &format!("/v1/operations/{id}")).send().await?.status() == StatusCode::NO_CONTENT);
    let remaining = response.text().await?;
    ensure!(remaining.contains("cancelled"));
    let list: Vec<Value> = harness
        .request(reqwest::Method::GET, "/v1/providers/codex/conversations")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(list.len() == 1);
    let saved: Value = harness
        .request(reqwest::Method::GET, &format!("/v1/providers/codex/conversations/{}", list[0]["id"]))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(saved["session_id"] == "codex-session");
    ensure!(saved["messages"].as_array().unwrap().len() == 2 && saved["messages"][1]["incomplete"] == true);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(harness.root.path().join("profile/.operation.lock"))?;
    lock.try_lock()?;
    drop(lock);
    harness.api.cancel_all();
    harness.stop().await?;
    println!("ok: HTTP cancellation terminates the owned runtime and releases the profile lock");
    let root = tempfile::tempdir()?;
    let path = root.path().join("api.token");
    let token = gpui_agent_infrastructure::load_or_create_token(&path)?;
    ensure!(token.len() == 64 && token == gpui_agent_infrastructure::load_or_create_token(&path)?);
    println!("ok: API token is generated once and reused");
    revisions().await?;
    for provider in gpui_agent_api::PROVIDERS {
        for scenario in [
            "interactive-allow",
            "interactive-deny",
            "interactive-request-cancel",
            "interactive-failed",
            "interactive-cancel",
            "interactive-running",
            "interactive-question",
            "interactive-question-cancel",
        ] {
            if provider == Provider::Antigravity && scenario.starts_with("interactive-question") {
                continue;
            }
            interactive(provider, scenario).await?;
        }
    }
    pending_timeout().await?;
    for provider in gpui_agent_api::PROVIDERS {
        attachment_roundtrip(provider, "attachments").await?;
    }
    attachment_roundtrip(Provider::Antigravity, "attachments-fallback").await?;
    attachment_roundtrip(Provider::Antigravity, "attachments-no-images").await?;
    println!("ok: all providers tool results, allow/deny, questions where supported, pending/running cancellation and SQLite restart");
    Ok(())
}

async fn revisions() -> Result<()> {
    use gpui_agent_contracts::{ConversationVersion,
                               PreparedRevision,
                               SavedConversation};
    let harness = Harness::new("normal").await?;
    stream(
        &harness,
        harness
            .request(reqwest::Method::POST, "/v1/providers/codex/login")
            .json(&json!({}))
            .send()
            .await?,
    )
    .await?;
    let mut id = None;
    for text in ["Earlier question", "Last question"] {
        let events = stream(
            &harness,
            harness
                .request(reqwest::Method::POST, "/v1/providers/codex/chat")
                .json(&json!({"text":text,"conversation_id":id,"model":null,"session_id":null}))
                .send()
                .await?,
        )
        .await?;
        id = events.iter().find_map(|event| {
            if let StreamEvent::Conversation {
                conversation_id,
            } = event
            {
                Some(*conversation_id)
            } else {
                None
            }
        });
        ensure!(matches!(events.last(), Some(StreamEvent::Completed { .. })), "{events:?}");
    }
    let id = id.context("missing source")?;
    let source: SavedConversation = harness
        .request(reqwest::Method::GET, &format!("/v1/providers/codex/conversations/{id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        harness
            .request(reqwest::Method::POST, &format!("/v1/providers/codex/conversations/{id}/revisions"))
            .json(&json!({"message_id":source.messages[0].id,"text":"Stale edit"}))
            .send()
            .await?
            .status()
            == StatusCode::CONFLICT
    );
    let prepared: PreparedRevision = harness
        .request(reqwest::Method::POST, &format!("/v1/providers/codex/conversations/{id}/revisions"))
        .json(&json!({"message_id":source.messages[2].id,"text":"Edited question"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(prepared.conversation.id != id && prepared.conversation.session_id.is_none());
    ensure!(prepared.conversation.messages.len() == 2 && prepared.request.session_id.is_none());
    let next = prepared.conversation.id;
    let events = stream(
        &harness,
        harness
            .request(reqwest::Method::POST, "/v1/providers/codex/chat")
            .json(&prepared.request)
            .send()
            .await?,
    )
    .await?;
    ensure!(matches!(events.last(), Some(StreamEvent::Completed { .. })), "{events:?}");
    let saved: SavedConversation = harness
        .request(reqwest::Method::GET, &format!("/v1/providers/codex/conversations/{next}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(saved.messages.len() == 4 && saved.messages[0].text == "Earlier question" && saved.messages[2].text == "Edited question");
    let unchanged: SavedConversation = harness
        .request(reqwest::Method::GET, &format!("/v1/providers/codex/conversations/{id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(unchanged.messages[2].text == "Last question" && unchanged.messages.len() == 4);
    let root = harness.stop().await?;
    let harness = Harness::open(root, "normal").await?;
    let versions: Vec<ConversationVersion> = harness
        .request(reqwest::Method::GET, &format!("/v1/providers/codex/conversations/{next}/versions"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(versions.len() == 2 && versions[0].conversation_id == id && versions[1].conversation_id == next && versions[0].question_index == 2);
    ensure!(
        harness
            .request(reqwest::Method::GET, &format!("/v1/providers/claude/conversations/{next}/versions"))
            .send()
            .await?
            .status()
            == StatusCode::NOT_FOUND
    );
    harness.stop().await?;
    println!("ok: edited question uses a fresh session, keeps the prefix and original answer, and preserves versions across restart");
    Ok(())
}

async fn interactive(provider: Provider, scenario: &str) -> Result<()> {
    let harness = Harness::open_provider(tempfile::tempdir()?, scenario, provider).await?;
    let name = gpui_agent_api::provider_name(provider);
    let profile = harness.root.path().join("profile");
    let token = if provider == Provider::Antigravity {
        profile.join("antigravity-acp/acp_token.json")
    } else {
        profile.join("fixture-auth")
    };
    fs::create_dir_all(token.parent().unwrap())?;
    fs::write(token, "fixture")?;
    let capabilities: Value = harness.request(reqwest::Method::GET, "/v1/providers").send().await?.json().await?;
    let info = &capabilities[gpui_agent_api::provider_index(provider)];
    ensure!(info["capabilities"]["user_input"] == (provider != Provider::Antigravity));
    let mut response = harness
        .request(reqwest::Method::POST, &format!("/v1/providers/{name}/chat"))
        .json(&json!({"text":"interactive fixture","attachments":[{"type":"text","name":"cancel.txt","text":"retain through cancellation"}]}))
        .send()
        .await?
        .error_for_status()?;
    let mut buffer = Vec::new();
    let mut events = Vec::new();
    let mut conversation_id = None;
    let mut operation_id = None;
    let mut pending_route = None;
    let mut answered = false;
    let mut cancelled = false;
    while let Some(event) = next_event(&mut response, &mut buffer).await? {
        match &event {
            | StreamEvent::Started {
                operation_id: id,
            } => operation_id = Some(*id),
            | StreamEvent::Conversation {
                conversation_id: id,
            } => conversation_id = Some(*id),
            | StreamEvent::Interaction {
                operation_id: operation,
                request_id,
                event,
            } => {
                let route = format!("/v1/operations/{operation}/interactions/{request_id}");
                pending_route = Some(route.clone());
                ensure!(
                    harness
                        .client
                        .post(format!("{}{route}", harness.url))
                        .json(&json!({"type":"cancelled"}))
                        .send()
                        .await?
                        .status()
                        == StatusCode::UNAUTHORIZED
                );
                ensure!(
                    harness
                        .request(reqwest::Method::POST, &format!("/v1/operations/999999/interactions/{request_id}"))
                        .json(&json!({"type":"cancelled"}))
                        .send()
                        .await?
                        .status()
                        == StatusCode::NOT_FOUND
                );
                if scenario == "interactive-cancel" {
                    harness
                        .request(reqwest::Method::DELETE, &format!("/v1/operations/{operation}"))
                        .send()
                        .await?
                        .error_for_status()?;
                    cancelled = true;
                } else {
                    let answer = match event {
                        | gpui_agent_api::Event::UserInput {
                            questions,
                        } => {
                            let invalid = json!({"type":"answers","answers":{"wrong-id":["B"]}});
                            ensure!(harness.request(reqwest::Method::POST, &route).json(&invalid).send().await?.status() == StatusCode::BAD_REQUEST);
                            if scenario == "interactive-question-cancel" {
                                json!({"type":"cancelled"})
                            } else {
                                json!({"type":"answers","answers":{questions[0].id.clone():["B"]}})
                            }
                        },
                        | gpui_agent_api::Event::Permission {
                            ..
                        } => {
                            ensure!(
                                harness
                                    .request(reqwest::Method::POST, &route)
                                    .json(&json!({"type":"answers","answers":{}}))
                                    .send()
                                    .await?
                                    .status()
                                    == StatusCode::BAD_REQUEST
                            );
                            if scenario == "interactive-request-cancel" {
                                json!({"type":"cancelled"})
                            } else {
                                json!({"type":"approval","allow":scenario != "interactive-deny"})
                            }
                        },
                        | _ => anyhow::bail!("Unexpected interaction"),
                    };
                    ensure!(harness.request(reqwest::Method::POST, &route).json(&answer).send().await?.status() == StatusCode::NO_CONTENT);
                    answered = true;
                }
                ensure!(
                    harness
                        .request(reqwest::Method::POST, &route)
                        .json(&json!({"type":"approval","allow":true}))
                        .send()
                        .await?
                        .status()
                        == StatusCode::NOT_FOUND
                );
            },
            | StreamEvent::Agent(gpui_agent_api::Event::Tool {
                ..
            }) if scenario == "interactive-running" && answered && !cancelled => {
                harness
                    .request(reqwest::Method::DELETE, &format!("/v1/operations/{}", operation_id.unwrap()))
                    .send()
                    .await?
                    .error_for_status()?;
                cancelled = true;
            },
            | _ => {},
        }
        events.push(event);
    }
    ensure!(pending_route.is_some(), "{name}/{scenario}: no interaction");
    if cancelled {
        ensure!(matches!(events.last(), Some(StreamEvent::Cancelled)), "{name}/{scenario}: {events:?}");
        let lock = fs::OpenOptions::new().read(true).write(true).open(profile.join(".operation.lock"))?;
        lock.try_lock()?;
    } else {
        ensure!(matches!(events.last(), Some(StreamEvent::Completed { .. })), "{name}/{scenario}: {events:?}");
        ensure!(
            events
                .iter()
                .any(|event| matches!(event, StreamEvent::Agent(gpui_agent_api::Event::Tool { call }) if call.output.is_some())),
            "Missing tool output"
        );
        if scenario == "interactive-failed" {
            ensure!(
                events
                    .iter()
                    .any(|event| matches!(event, StreamEvent::Agent(gpui_agent_api::Event::Tool { call }) if call.status.as_deref()==Some("failed")))
            );
        }
    }
    let id = conversation_id.unwrap();
    let root = harness.stop().await?;
    let harness = Harness::open_provider(root, scenario, provider).await?;
    let route = format!("/v1/providers/{name}/conversations/{id}");
    let saved: gpui_agent_api::SavedConversation = harness.request(reqwest::Method::GET, &route).send().await?.error_for_status()?.json().await?;
    ensure!(!saved.timeline.is_empty());
    ensure!(matches!(&saved.messages[0].attachments[..], [gpui_agent_api::Attachment::Text { text, .. }] if text == "retain through cancellation"));
    ensure!(
        saved
            .timeline
            .iter()
            .any(|entry| matches!(entry.event,gpui_agent_api::Event::TurnEnd { completed } if completed != cancelled))
    );
    ensure!(saved.messages[1].incomplete == cancelled);
    if !cancelled {
        ensure!(
            saved
                .timeline
                .iter()
                .any(|entry| matches!(entry.event, gpui_agent_api::Event::InteractionResolved { .. }))
        );
    }
    harness.request(reqwest::Method::DELETE, &route).send().await?.error_for_status()?;
    harness.stop().await?;
    println!("ok: {name}/{scenario}");
    Ok(())
}

async fn pending_timeout() -> Result<()> {
    let harness = Harness::new("interactive-timeout").await?;
    fs::write(harness.root.path().join("profile/fixture-auth"), "fixture")?;
    let mut response = harness
        .request(reqwest::Method::POST, "/v1/providers/codex/chat")
        .json(&json!({"text":"timeout"}))
        .send()
        .await?;
    let mut buffer = Vec::new();
    let mut route = None;
    let mut terminal = None;
    while let Some(event) = next_event(&mut response, &mut buffer).await? {
        if let StreamEvent::Interaction {
            operation_id,
            request_id,
            ..
        } = &event
        {
            route = Some(format!("/v1/operations/{operation_id}/interactions/{request_id}"));
        }
        if event.is_terminal() {
            terminal = Some(event);
        }
    }
    ensure!(matches!(terminal, Some(StreamEvent::Error { .. })));
    ensure!(
        harness
            .request(reqwest::Method::POST, &route.context("No pending interaction")?)
            .json(&json!({"type":"approval","allow":true}))
            .send()
            .await?
            .status()
            == StatusCode::NOT_FOUND
    );
    harness.stop().await?;
    println!("ok: timeout revokes pending approval");
    Ok(())
}

async fn attachment_roundtrip(provider: Provider, scenario: &str) -> Result<()> {
    let harness = Harness::open_provider(tempfile::tempdir()?, scenario, provider).await?;
    let name = gpui_agent_api::provider_name(provider);
    let route = format!("/v1/providers/{name}");
    let profile = harness.root.path().join("profile");
    let token = if provider == Provider::Antigravity {
        profile.join("antigravity-acp/acp_token.json")
    } else {
        profile.join("fixture-auth")
    };
    fs::create_dir_all(token.parent().unwrap())?;
    fs::write(token, "fixture")?;
    let file = harness.root.path().join("sample.rs");
    fs::write(&file, "skip\nfn main() { println!(\"첨부 🦀\"); }\nskip\n")?;
    let context: Value = harness
        .request(reqwest::Method::POST, &format!("{route}/context"))
        .json(&json!({"path":"sample.rs","start_line":2,"end_line":2}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(context["text"] == "fn main() { println!(\"첨부 🦀\"); }\n");
    ensure!(context["source"]["start_line"] == 2);
    fs::write(&file, "changed after snapshot")?;
    for request in [
        json!({"path":"../outside"}),
        json!({"path":file}),
        json!({"path":"sample.rs","start_line":0}),
        json!({"path":"sample.rs","end_line":90}),
    ] {
        ensure!(
            harness
                .request(reqwest::Method::POST, &format!("{route}/context"))
                .json(&request)
                .send()
                .await?
                .status()
                == StatusCode::BAD_REQUEST
        );
    }
    let image = json!({"type":"image","name":"pixel.png","mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII="});
    for attachments in [
        json!([{"type":"image","name":"bad.png","mime_type":"image/png","data":"bad!"}]),
        json!([{"type":"text","name":"binary","text":"a\u{0}b"}]),
        json!([{"type":"text","name":"large","text":"x".repeat(256*1024+1)}]),
        Value::Array(vec![context.clone(); 9]),
    ] {
        ensure!(
            harness
                .request(reqwest::Method::POST, &format!("{route}/chat"))
                .json(&json!({"text":"hello","attachments":attachments}))
                .send()
                .await?
                .status()
                == StatusCode::BAD_REQUEST
        );
    }
    let empty: Vec<Value> = harness
        .request(reqwest::Method::GET, &format!("{route}/conversations"))
        .send()
        .await?
        .json()
        .await?;
    ensure!(empty.is_empty(), "Invalid attachments must not create conversations");
    let attachments = vec![context.clone(), image];
    let events = stream(
        &harness,
        harness
            .request(reqwest::Method::POST, &format!("{route}/chat"))
            .json(&json!({"text":"","attachments":attachments}))
            .send()
            .await?,
    )
    .await?;
    let id = events
        .iter()
        .find_map(|event| {
            if let StreamEvent::Conversation {
                conversation_id,
            } = event
            {
                Some(*conversation_id)
            } else {
                None
            }
        })
        .context("Missing saved chat")?;
    if scenario == "attachments-no-images" {
        ensure!(matches!(events.last(), Some(StreamEvent::Error { message }) if message.contains("does not advertise image")));
        let requests = fs::read_to_string(profile.join("requests.jsonl"))?;
        ensure!(!requests.contains("session/prompt"), "Unsupported images must not be sent to ACP");
    } else {
        ensure!(matches!(events.last(), Some(StreamEvent::Completed { .. })), "{name}: {events:?}");
    }
    let root = harness.stop().await?;
    let harness = Harness::open_provider(root, scenario, provider).await?;
    let saved: Value = harness
        .request(reqwest::Method::GET, &format!("{route}/conversations/{id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(saved["messages"][0]["attachments"] == json!(attachments));
    ensure!(saved["messages"][1]["attachments"] == json!([]));
    if scenario != "attachments-no-images" {
        let events = stream(
            &harness,
            harness
                .request(reqwest::Method::POST, &format!("{route}/chat"))
                .json(&json!({"text":"continue","conversation_id":id}))
                .send()
                .await?,
        )
        .await?;
        ensure!(matches!(events.last(), Some(StreamEvent::Completed { .. })));
        let saved: Value = harness
            .request(reqwest::Method::GET, &format!("{route}/conversations/{id}"))
            .send()
            .await?
            .json()
            .await?;
        ensure!(saved["messages"].as_array().unwrap().len() == 4);
        ensure!(saved["messages"][2]["attachments"] == json!([]));
    }
    harness
        .request(reqwest::Method::DELETE, &format!("{route}/conversations/{id}"))
        .send()
        .await?
        .error_for_status()?;
    harness.stop().await?;
    println!("ok: {name}/{scenario} context, native text/image payload, validation, SQLite restart/resume/delete");
    Ok(())
}
