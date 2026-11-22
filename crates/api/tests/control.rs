use gpui_agent_application::ports::{HostControl,
                                    RuntimeFuture};
use gpui_agent_contracts::*;
use std::sync::{Arc,
                atomic::{AtomicBool,
                         Ordering}};
struct Mock(AtomicBool);
impl HostControl for Mock {
    fn automate(&self, scope: Option<McpScope>, command: BrowserCommand) -> RuntimeFuture<'_, BrowserReply> {
        Box::pin(async move {
            if matches!(command, BrowserCommand::Diagnostics {}) {
                return Ok(BrowserReply::Diagnostics(BrowserDiagnostics {
                    entries: vec![BrowserDiagnostic {
                        kind: "exception".into(),
                        level: "error".into(),
                        text: scope.map_or("operator".into(), |scope| scope.workspace_path),
                        url: "about:srcdoc".into(),
                        line: Some(3),
                    }],
                    truncated: false,
                }));
            }
            if matches!(command, BrowserCommand::Capture {}) {
                return Ok(BrowserReply::Capture(ControlFrame {
                    width: 1,
                    height: 1,
                    url: Some("https://fixture.invalid/".into()),
                    image: Attachment::Image {
                        name: "capture.png".into(),
                        mime_type: "image/png".into(),
                        data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
                    },
                }));
            }
            if matches!(command, BrowserCommand::Close {}) {
                return Ok(BrowserReply::Closed);
            }
            Ok(BrowserReply::Snapshot(BrowserSnapshot {
                revision: "fixture:1".into(),
                url: format!("https://fixture.invalid/{}", scope.map_or("operator".into(), |scope| scope.workspace_path)),
                title: "fixture".into(),
                text: "fixture DOM".into(),
                text_truncated: false,
                targets: vec![],
                targets_truncated: false,
            }))
        })
    }

    fn status(&self) -> RuntimeFuture<'_, ControlStatus> {
        Box::pin(async {
            Ok(ControlStatus {
                browser_available: true,
                browser_running: false,
                desktop_available: true,
                desktop_enabled_until: if self.0.load(Ordering::SeqCst) { 600 } else { 0 },
            })
        })
    }

    fn browser(&self, _: ControlCommand) -> RuntimeFuture<'_, Option<ControlFrame>> { Box::pin(async { Ok(None) }) }

    fn desktop(&self, _: ControlCommand) -> RuntimeFuture<'_, Option<ControlFrame>> {
        Box::pin(async {
            anyhow::ensure!(self.0.load(Ordering::SeqCst), "grant required");
            Ok(None)
        })
    }

    fn enable_desktop(&self, enabled: bool) -> RuntimeFuture<'_, ()> {
        Box::pin(async move {
            self.0.store(enabled, Ordering::SeqCst);
            Ok(())
        })
    }

    fn shutdown(&self) { self.0.store(false, Ordering::SeqCst); }
}

#[tokio::test]
async fn browser_auth_control_grants_and_public_shell() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let app = gpui_agent_infrastructure::application_from_agents(
        PROVIDERS.map(|_| Err(anyhow::anyhow!("not installed"))),
        &root.path().join("history.sqlite3"),
        root.path(),
    )?;
    let token = "testBearerWithAtLeast32Characters12345678";
    let backend = Arc::new(Mock(AtomicBool::new(false)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let sessions = Arc::new(gpui_agent_infrastructure::McpSessions::new(format!("{origin}/mcp"))?);
    let scope = McpScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_path: "scope-a".into(),
    };
    let writer = sessions.issue(scope.clone(), true)?;
    let reader = sessions.issue(scope, false)?;
    let foreign = sessions.issue(
        McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: "scope-b".into(),
        },
        true,
    )?;
    let api = gpui_agent_api::Api::new(app, token.into())?
        .with_controls(backend.clone())
        .with_mcp_sessions(sessions);
    let (stop, done) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = done.await;
    }));
    let client = reqwest::Client::builder().no_proxy().build()?;
    let response = client.get(&origin).send().await?;
    assert_eq!(response.status(), 200);
    assert!(response.headers()["content-security-policy"].to_str()?.contains("frame-ancestors 'none'"));
    assert!(!response.text().await?.contains(token));
    for asset in ["/app.js", "/style.css", "/manifest.webmanifest", "/sw.js", "/icon.svg"] {
        assert_eq!(client.get(format!("{origin}{asset}")).send().await?.status(), 200);
    }
    let tools: serde_json::Value = client
        .post(format!("{origin}/mcp"))
        .bearer_auth(token)
        .header("Accept", "application/json, text/event-stream")
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(
        tools["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == "browser_open"),
        "browser automation must be discoverable without a Jobs service"
    );
    for name in ["browser_render_html", "browser_diagnostics"] {
        assert!(
            tools["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == name),
            "Missing {name}"
        );
    }
    // Given authenticated MCP without Jobs, when tools are discovered, then
    // mobile device tools are present but operator grants are never agent
    // tools.
    for name in ["device_list", "device_open", "device_screenshot", "device_act", "device_close"] {
        assert!(
            tools["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == name),
            "Missing {name}"
        );
    }
    assert!(!tools["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == "device_grant"));
    async fn call(client: &reqwest::Client, origin: &str, token: &str, name: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Ok(client
            .post(format!("{origin}/mcp"))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .json(&serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":args}}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    let snapshot = call(
        &client,
        &origin,
        &writer.connection().token,
        "browser_open",
        serde_json::json!({"url":"https://fixture.invalid"}),
    )
    .await?;
    assert_eq!(snapshot["result"]["structuredContent"]["url"], "https://fixture.invalid/scope-a");
    let rendered = call(
        &client,
        &origin,
        &writer.connection().token,
        "browser_render_html",
        serde_json::json!({"html":"<h1>Result</h1>"}),
    )
    .await?;
    assert_eq!(rendered["result"]["structuredContent"]["url"], "https://fixture.invalid/scope-a");
    let diagnostics = call(&client, &origin, &reader.connection().token, "browser_diagnostics", serde_json::json!({})).await?;
    assert_eq!(diagnostics["result"]["structuredContent"]["entries"][0]["text"], "scope-a");
    assert_eq!(diagnostics["result"]["structuredContent"]["entries"][0]["level"], "error");
    assert_eq!(diagnostics["result"]["structuredContent"]["truncated"], false);
    let snapshot = call(&client, &origin, &foreign.connection().token, "browser_snapshot", serde_json::json!({})).await?;
    assert_eq!(snapshot["result"]["structuredContent"]["url"], "https://fixture.invalid/scope-b");
    assert_eq!(
        call(&client, &origin, token, "browser_snapshot", serde_json::json!({})).await?["result"]["structuredContent"]["url"],
        "https://fixture.invalid/operator"
    );
    for (name, args) in [
        ("browser_open", serde_json::json!({"url":"https://fixture.invalid"})),
        ("browser_act", serde_json::json!({"action":{"type":"key","key":"Enter"}})),
        ("browser_close", serde_json::json!({})),
        ("browser_render_html", serde_json::json!({"html":"<h1>Result</h1>"})),
    ] {
        assert_eq!(call(&client, &origin, &reader.connection().token, name, args).await?["error"]["code"], -32001);
    }
    assert_eq!(
        call(&client, &origin, &reader.connection().token, "browser_snapshot", serde_json::json!({})).await?["result"]["isError"],
        false
    );
    let capture = call(&client, &origin, &reader.connection().token, "browser_capture", serde_json::json!({})).await?;
    assert_eq!(capture["result"]["content"][1]["type"], "image");
    assert_eq!(capture["result"]["content"][1]["mimeType"], "image/png");
    assert_eq!(capture["result"]["structuredContent"]["width"], 1);
    assert!(!capture["result"]["structuredContent"].to_string().contains("iVBOR"));
    for (name, args) in [
        ("browser_snapshot", serde_json::json!({"scope":"forged"})),
        ("browser_open", serde_json::json!({"url":"https://fixture.invalid","type":"capture"})),
        (
            "browser_act",
            serde_json::json!({"action":{"type":"fill","revision":"","target":0,"text":"invalid"}}),
        ),
        (
            "browser_act",
            serde_json::json!({"action":{"type":"click","revision":"fixture:1","target":100}}),
        ),
        (
            "browser_act",
            serde_json::json!({"action":{"type":"fill","revision":"fixture:1","target":0,"text":"x".repeat(4097)}}),
        ),
        ("browser_act", serde_json::json!({"action":{"type":"key","key":"F24"}})),
        ("browser_wait", serde_json::json!({"text":"","timeout_ms":0})),
        ("browser_wait", serde_json::json!({"text":"ready","timeout_ms":10001})),
        ("browser_render_html", serde_json::json!({"html":" "})),
        ("browser_render_html", serde_json::json!({"html":"x".repeat(BROWSER_HTML_LIMIT+1)})),
        ("browser_render_html", serde_json::json!({"html":"<h1>\u{0}</h1>"})),
        ("browser_render_html", serde_json::json!({"html":"<h1>Result</h1>","url":"file:///secret"})),
        ("browser_diagnostics", serde_json::json!({"scope":"forged"})),
    ] {
        assert_eq!(
            call(&client, &origin, &writer.connection().token, name, args).await?["result"]["isError"],
            true,
            "accepted invalid {name}"
        );
    }
    assert_eq!(client.get(format!("{origin}/v1/control")).send().await?.status(), 401);
    assert_eq!(
        client
            .get(format!("{origin}/v1/providers"))
            .bearer_auth(token)
            .header("Origin", &origin)
            .send()
            .await?
            .status(),
        401
    );
    let web = |method, path: &str| {
        client
            .request(method, format!("{origin}{path}"))
            .bearer_auth(token)
            .header("Origin", &origin)
            .header("X-KND-Web", "1")
            .header("Sec-Fetch-Site", "same-origin")
    };
    // Given the web client's header, when a same-origin request authenticates,
    // then it succeeds, while duplicate client headers are rejected.
    assert_eq!(web(reqwest::Method::GET, "/v1/providers").send().await?.status(), 200);
    assert_eq!(web(reqwest::Method::GET, "/v1/providers").header("X-KND-Web", "1").send().await?.status(), 401);
    assert_eq!(
        web(reqwest::Method::POST, "/v1/control/browser")
            .header("Origin", "https://evil.example")
            .json(&serde_json::json!({"type":"capture"}))
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        web(reqwest::Method::POST, "/v1/control/browser")
            .header("Sec-Fetch-Site", "cross-site")
            .json(&serde_json::json!({"type":"capture"}))
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        client
            .request(reqwest::Method::OPTIONS, format!("{origin}/v1/control/browser"))
            .header("Origin", "https://evil.example")
            .header("Access-Control-Request-Headers", "authorization")
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        web(reqwest::Method::POST, "/v1/control/desktop")
            .json(&serde_json::json!({"type":"capture"}))
            .send()
            .await?
            .status(),
        502
    );
    assert_eq!(web(reqwest::Method::POST, "/v1/control/desktop/grant").send().await?.status(), 204);
    assert!(backend.0.load(Ordering::SeqCst));
    assert_eq!(
        web(reqwest::Method::POST, "/v1/control/desktop")
            .json(&serde_json::json!({"type":"capture"}))
            .send()
            .await?
            .status(),
        200
    );
    assert_eq!(
        web(reqwest::Method::POST, "/v1/control/browser")
            .json(&serde_json::json!({"type":"key","key":"arbitrary shell command"}))
            .send()
            .await?
            .status(),
        400
    );
    assert_eq!(
        web(reqwest::Method::POST, "/v1/control/browser")
            .json(&serde_json::json!({"type":"capture","extra":true}))
            .send()
            .await?
            .status(),
        422
    );
    assert_eq!(web(reqwest::Method::DELETE, "/v1/control/desktop").send().await?.status(), 204);
    assert!(!backend.0.load(Ordering::SeqCst));
    stop.send(()).unwrap();
    task.await??;
    Ok(())
}
