use anyhow::{Result,
             ensure};
use gpui_agent_application::ports::McpSessionLease;
use gpui_agent_contracts::{attachment::MAX_IMAGE_BYTES,
                           *};
use serde_json::{Value,
                 json};
use std::{io::Write,
          sync::Arc,
          time::Duration};

const TOKEN: &str = "deviceFixtureOperatorCredential1234567890";
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218,
    99, 100, 248, 15, 0, 1, 5, 1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn fixture(args: &[String]) -> Result<()> {
    ensure!(args.get(1).map(String::as_str) == Some("127.0.0.1"), "Only the local ADB server is allowed");
    let root = std::env::current_exe()?.parent().unwrap().to_path_buf();
    let args = &args[2 ..];
    if args == ["devices", "-l"] {
        println!("List of devices attached\nemulator-5554 device product:sdk model:Fixture_Phone transport_id:1\nUSB-123 unauthorized model:Private_Phone\n");
        return Ok(());
    }
    ensure!(
        args.first().map(String::as_str) == Some("-s") && args.get(1).map(String::as_str) == Some("emulator-5554"),
        "Every command needs an explicit ready device"
    );
    let args = &args[2 ..];
    if args == ["exec-out", "screencap", "-p"] {
        let mode = std::fs::read_to_string(root.join("mode")).unwrap_or_default();
        match mode.as_str() {
            | "invalid" => std::io::stdout().write_all(b"not PNG")?,
            | "large" => std::io::stdout().write_all(&vec![0u8; MAX_IMAGE_BYTES + 1])?,
            | "dimensions" => {
                let mut bytes = PNG.to_vec();
                bytes[16 .. 20].copy_from_slice(&16385_u32.to_be_bytes());
                std::io::stdout().write_all(&bytes)?;
            },
            | "hang" => {
                std::fs::write(root.join("ready"), "ready")?;
                std::thread::sleep(Duration::from_secs(60));
            },
            | _ => std::io::stdout().write_all(PNG)?,
        }
        return Ok(());
    }
    ensure!(args.first().map(String::as_str) == Some("shell"), "Only expected ADB commands are allowed");
    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(root.join("actions.jsonl"))?;
    writeln!(log, "{}", serde_json::to_string(args)?)?;
    Ok(())
}

struct Host {
    root: tempfile::TempDir,
    origin: String,
    client: reqwest::Client,
    writer: Box<dyn McpSessionLease>,
    reader: Box<dyn McpSessionLease>,
    foreign: Box<dyn McpSessionLease>,
    scope: McpScope,
    sessions: Arc<gpui_agent_infrastructure::McpSessions>,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl Host {
    async fn start() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let executable = root.path().join(if cfg!(windows) { "adb.exe" } else { "adb" });
        std::fs::copy(std::env::current_exe()?, &executable)?;
        let app = gpui_agent_infrastructure::application_from_agents(
            PROVIDERS.map(|_| Err(anyhow::anyhow!("no provider required"))),
            &root.path().join("history.sqlite3"),
            root.path(),
        )?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let sessions = Arc::new(gpui_agent_infrastructure::McpSessions::new(format!("{origin}/mcp"))?);
        let scope = McpScope {
            provider: Provider::Codex,
            account_id: "default".into(),
            workspace_path: "fixture-a".into(),
        };
        let writer = Box::new(sessions.issue(scope.clone(), true)?) as Box<dyn McpSessionLease>;
        let reader = Box::new(sessions.issue(scope.clone(), false)?) as Box<dyn McpSessionLease>;
        let foreign = Box::new(sessions.issue(
            McpScope {
                workspace_path: "fixture-b".into(),
                ..scope.clone()
            },
            true,
        )?) as Box<dyn McpSessionLease>;
        let controls = Arc::new(gpui_agent_control::Controls::default().with_adb(executable)?);
        let api = gpui_agent_api::Api::new(app, TOKEN.into())?
            .with_controls(controls)
            .with_mcp_sessions(sessions.clone());
        let (stop, done) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(gpui_agent_api::serve(listener, api, async {
            let _ = done.await;
        }));
        Ok(Self {
            root,
            origin,
            client: reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(20)).build()?,
            writer,
            reader,
            foreign,
            scope,
            sessions,
            stop,
            task,
        })
    }

    async fn call(&self, token: &str, name: &str, args: Value) -> Result<Value> {
        Ok(self
            .client
            .post(format!("{}/mcp", self.origin))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn grant(&self, enabled: bool) -> Result<()> {
        self.client
            .post(format!("{}/v1/control/devices/grant", self.origin))
            .bearer_auth(TOKEN)
            .json(&json!({"device_id":"emulator-5554","scope":self.scope,"enabled":enabled}))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn stop(self) -> Result<()> {
        let _ = self.stop.send(());
        self.task.await??;
        Ok(())
    }
}

async fn lifecycle() -> Result<()> {
    // Given a real host and a native ADB fixture, without any live Android
    // device.
    let host = Host::start().await?;
    let writer = &host.writer.connection().token;
    let reader = &host.reader.connection().token;
    let foreign = &host.foreign.connection().token;
    ensure!(
        host.client
            .post(format!("{}/v1/control/devices/grant", host.origin))
            .bearer_auth(writer)
            .json(&json!({"device_id":"emulator-5554","scope":host.scope,"enabled":true}))
            .send()
            .await?
            .status()
            == 401
    );
    let devices = host.call(TOKEN, "device_list", json!({})).await?;
    ensure!(
        devices["result"]["structuredContent"]["available"] == true,
        "Native ADB discovery must be available"
    );
    ensure!(devices["result"]["structuredContent"]["devices"].as_array().is_some_and(|d| d.len() == 2));
    ensure!(host.call(writer, "device_list", json!({})).await?["result"]["structuredContent"]["devices"] == json!([]));
    ensure!(host.call(writer, "device_open", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == true);
    // When the operator grants one scope, then other scopes and read-only
    // writes remain denied.
    host.grant(true).await?;
    ensure!(host.call(foreign, "device_open", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == true);
    ensure!(host.call(reader, "device_open", json!({"device_id":"emulator-5554"})).await?["error"]["code"] == -32001);
    ensure!(host.call(writer, "device_open", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == false);
    ensure!(host.call(foreign, "device_close", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == false);
    let shot = host.call(reader, "device_screenshot", json!({"device_id":"emulator-5554"})).await?;
    ensure!(shot["result"]["isError"] == false && shot["result"]["content"][1]["type"] == "image");
    ensure!(shot["result"]["structuredContent"]["width"] == 1 && shot["result"]["structuredContent"].get("image").is_none());
    let revision = shot["result"]["structuredContent"]["revision"].clone();
    ensure!(
        host.call(
            writer,
            "device_act",
            json!({"device_id":"emulator-5554","revision":revision,"action":{"type":"tap","x":1,"y":0}})
        )
        .await?["result"]["isError"]
            == true
    );
    let act = json!({"device_id":"emulator-5554","revision":revision,"action":{"type":"tap","x":0,"y":0}});
    ensure!(host.call(writer, "device_act", act.clone()).await?["result"]["isError"] == false);
    ensure!(host.call(writer, "device_act", act).await?["result"]["isError"] == true);
    // Native arguments stay on the selected device, including shell
    // metacharacters in text.
    for action in [
        json!({"type":"swipe","x":0,"y":0,"to_x":0,"to_y":0,"duration_ms":100}),
        json!({"type":"key","key":"Back"}),
        json!({"type":"text","text":"Hello & 'world'"}),
    ] {
        let screenshot = host.call(reader, "device_screenshot", json!({"device_id":"emulator-5554"})).await?;
        let revision = &screenshot["result"]["structuredContent"]["revision"];
        ensure!(
            host.call(writer, "device_act", json!({"device_id":"emulator-5554","revision":revision,"action":action}))
                .await?["result"]["isError"]
                == false
        );
    }
    let commands = std::fs::read_to_string(host.root.path().join("actions.jsonl"))?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        commands
            == vec![
                json!(["shell", "input", "tap", "0", "0"]),
                json!(["shell", "input", "swipe", "0", "0", "0", "0", "100"]),
                json!(["shell", "input", "keyevent", "4"]),
                json!(["shell", "input", "text", r#"'Hello%s&%s'\''world'\'''"#])
            ],
        "Native action arguments must match the selected operation"
    );
    ensure!(host.call(foreign, "device_screenshot", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == true);
    // A failed new capture must invalidate the previous screen reference.
    let before_failure = host.call(reader, "device_screenshot", json!({"device_id":"emulator-5554"})).await?;
    let old_revision = before_failure["result"]["structuredContent"]["revision"].clone();
    for mode in ["invalid", "large", "dimensions"] {
        std::fs::write(host.root.path().join("mode"), mode)?;
        ensure!(host.call(reader, "device_screenshot", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == true);
    }
    ensure!(
        host.call(
            writer,
            "device_act",
            json!({"device_id":"emulator-5554","revision":old_revision,"action":{"type":"tap","x":0,"y":0}})
        )
        .await?["result"]["isError"]
            == true,
        "A failed capture must discard the previous revision"
    );
    std::fs::write(host.root.path().join("mode"), "")?;
    // Revocation discards the session and all old screenshot references.
    host.grant(false).await?;
    ensure!(host.call(writer, "device_screenshot", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == true);
    ensure!(host.call(writer, "device_close", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == false);
    // The lease expires by elapsed time, without waiting ten real minutes.
    host.grant(true).await?;
    ensure!(host.call(writer, "device_open", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == false);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(601)).await;
    tokio::time::resume();
    ensure!(
        host.call(writer, "device_screenshot", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == true,
        "Elapsed-time expiry must discard the grant"
    );
    // Revoking a grant interrupts a hung native capture, with no late image
    // reply.
    host.grant(true).await?;
    host.call(writer, "device_open", json!({"device_id":"emulator-5554"})).await?;
    std::fs::write(host.root.path().join("mode"), "hang")?;
    let pending = host.call(reader, "device_screenshot", json!({"device_id":"emulator-5554"}));
    let revoke = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !host.root.path().join("ready").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        host.grant(false).await
    };
    let (reply, revoked) = tokio::join!(pending, revoke);
    revoked?;
    ensure!(reply?["result"]["isError"] == true, "Revocation must cancel native capture");
    std::fs::write(host.root.path().join("mode"), "")?;
    // The same public host service accepts grants for all three providers.
    for provider in PROVIDERS {
        let scope = McpScope {
            provider,
            ..host.scope.clone()
        };
        let lease = host.sessions.issue(scope.clone(), true)?;
        host.client
            .post(format!("{}/v1/control/devices/grant", host.origin))
            .bearer_auth(TOKEN)
            .json(&json!({"device_id":"emulator-5554","scope":scope,"enabled":true}))
            .send()
            .await?
            .error_for_status()?;
        let credential = &lease.connection().token;
        ensure!(host.call(credential, "device_open", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == false);
        ensure!(host.call(credential, "device_screenshot", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == false);
        ensure!(host.call(credential, "device_close", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == false);
        ensure!(host.call(credential, "device_open", json!({"device_id":"emulator-5554"})).await?["result"]["isError"] == true);
    }
    // The operator HTTP path uses the same service with its own explicit grant.
    let endpoint = format!("{}/v1/control/devices", host.origin);
    ensure!(
        !host
            .client
            .post(format!("{endpoint}/grant"))
            .bearer_auth(TOKEN)
            .json(&json!({"device_id":"USB-123","enabled":true}))
            .send()
            .await?
            .status()
            .is_success(),
        "Android-unauthorized devices cannot be granted"
    );
    host.client
        .post(format!("{endpoint}/grant"))
        .bearer_auth(TOKEN)
        .json(&json!({"device_id":"emulator-5554","enabled":true}))
        .send()
        .await?
        .error_for_status()?;
    host.client
        .post(&endpoint)
        .bearer_auth(TOKEN)
        .json(&json!({"type":"open","device_id":"emulator-5554"}))
        .send()
        .await?
        .error_for_status()?;
    let capture: Value = host
        .client
        .post(&endpoint)
        .bearer_auth(TOKEN)
        .json(&json!({"type":"screenshot","device_id":"emulator-5554"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(capture["image"]["mime_type"] == "image/png");
    for text in ["한글", "percent%", ""] {
        ensure!(
            host.client
                .post(&endpoint)
                .bearer_auth(TOKEN)
                .json(&json!({"type":"act","device_id":"emulator-5554","revision":capture["revision"],"action":{"type":"text","text":text}}))
                .send()
                .await?
                .status()
                == 400
        );
    }
    host.client
        .post(&endpoint)
        .bearer_auth(TOKEN)
        .json(&json!({"type":"close","device_id":"emulator-5554"}))
        .send()
        .await?
        .error_for_status()?;
    host.stop().await
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("-H") {
        return fixture(&args);
    }
    ensure!(args.is_empty(), "devices accepts no test filters");
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(lifecycle())?;
    println!("ok: native Android discovery, operator grants, scope isolation, image replies, stale references and revocation");
    Ok(())
}
