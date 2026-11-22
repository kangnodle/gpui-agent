#[allow(dead_code)]
#[path = "protocols.rs"]
mod native;
#[path = "support/transcript.rs"]
mod transcript;
use anyhow::{Result,
             ensure};
use gpui_agent::{Agent,
                 Config,
                 Provider};
use serde_json::json;
use std::{fs,
          path::Path,
          time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    if transcript::proxy()? {
        return Ok(());
    }
    if let Ok(spec) = fs::read_to_string("fixture.json") {
        return native::fixture(serde_json::from_str(&spec)?);
    }
    if std::env::args().nth(1).as_deref() == Some("--record-live") {
        return transcript::record_live().await;
    }
    native::test_arguments(&["--roundtrip-only"])?;
    // Given native process traffic, recording must redact and replay through
    // the same Agent API.
    for provider in [Provider::Claude, Provider::Codex, Provider::Antigravity] {
        round_trip(provider).await?;
    }
    if !std::env::args().any(|arg| arg == "--roundtrip-only") {
        transcript::replay_committed().await?;
    }
    println!("ok: sanitized native recordings replay through real adapters; mismatched requests fail closed");
    Ok(())
}

pub fn config(root: &Path, provider: Provider) -> Config {
    let executable = std::env::current_exe().unwrap();
    Config {
        provider,
        executable: executable.clone(),
        home: root.join("home"),
        cwd: root.into(),
        harness: Some(executable.clone()),
        browser_helper: Some(executable),
        timeout: Duration::from_secs(15),
        mcp_home: None,
        mcp_selection: None,
        plugin_mcp: vec![],
    }
}

async fn round_trip(provider: Provider) -> Result<()> {
    let root = tempfile::tempdir()?;
    let config = config(root.path(), provider);
    let capture = root.path().join("capture.jsonl");
    fs::write(
        root.path().join("fixture.json"),
        json!({"provider":gpui_agent_domain::provider_name(provider),"scenario":"normal"}).to_string(),
    )?;
    let auth = if provider == Provider::Antigravity {
        config.home.join("antigravity-acp/acp_token.json")
    } else {
        config.home.join("fixture-auth")
    };
    fs::create_dir_all(auth.parent().unwrap())?;
    fs::write(auth, "secret-capture-sentinel")?;
    fs::write(root.path().join("capture-proxy.json"), json!({"mode":"record","provider":gpui_agent_domain::provider_name(provider),"source":"fixture","runtime_version":"native-fixture-v1","executable":std::env::current_exe()?,"output":capture}).to_string())?;
    ensure!(Agent::new(config.clone())?.status().await?.authenticated);
    let recorded = fs::read_to_string(&capture)?;
    let private_directory = root.path().file_name().unwrap().to_string_lossy();
    ensure!(!recorded.contains("secret-capture-sentinel") && !recorded.contains('@') && !recorded.contains(private_directory.as_ref()));
    fs::write(root.path().join("capture-proxy.json"), json!({"mode":"replay","input":capture}).to_string())?;
    ensure!(
        Agent::new(config.clone())?.status().await?.authenticated,
        "{provider:?} replay lost auth structure"
    );
    let changed = recorded
        .replace("account/read", "unknown/request")
        .replace("initialize", "unknown/request")
        .replace("loggedIn", "changedField");
    fs::write(capture, changed)?;
    ensure!(Agent::new(config)?.status().await.is_err(), "Changed native contracts must reject replay");
    Ok(())
}
