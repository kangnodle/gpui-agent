use anyhow::{Context,
             Result,
             ensure};
use serde_json::Value;
use std::{path::PathBuf,
          process::Stdio,
          time::Duration};
use tokio::{io::{AsyncBufReadExt,
                 AsyncWriteExt,
                 BufReader},
            process::Command,
            time::{Instant,
                   timeout,
                   timeout_at}};

#[tokio::test]
async fn given_a_fixture_host_when_cancelled_and_stopped_then_client_works_and_state_is_removed() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gpui-agent-harness"));
    command
        .arg("client-fixture")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut host = gpui_agent_process::spawn(&mut command)?;
    let stdout = host.stdout.take().context("fixture stdout is missing")?;
    let mut lines = BufReader::new(stdout).lines();
    let result: Result<PathBuf> = async {
        let deadline = Instant::now() + Duration::from_secs(120);
        let (mut url, mut token_file) = (None, None);
        while url.is_none() || token_file.is_none() {
            let line = timeout_at(deadline, lines.next_line())
                .await??
                .context("fixture exited before reporting endpoint")?;
            if let Some(value) = line.strip_prefix("Fixture client: ") {
                url = Some(value.to_owned());
            }
            if let Some(value) = line.strip_prefix("Fixture-only token file: ") {
                token_file = Some(PathBuf::from(value));
            }
        }
        let url = url.context("fixture URL is missing")?;
        let token_file = token_file.context("fixture token path is missing")?;
        let token = std::fs::read_to_string(&token_file)?;
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(15)).build()?;
        ensure!(
            client.get(&url).send().await?.error_for_status()?.text().await?.contains("Gpui Agent"),
            "real web client was not served"
        );
        client.get(format!("{url}/v1/providers")).bearer_auth(&token).send().await?.error_for_status()?;
        let job: Value = client
            .post(format!("{url}/v1/providers/codex/jobs"))
            .bearer_auth(&token)
            .json(&serde_json::json!({"text":"client fixture cleanup"}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let id = job["id"].as_i64().context("job ID is missing")?;
        let events_url = format!("{url}/v1/jobs/{id}/events");
        reaches_approval(client.get(&events_url).bearer_auth(&token).send().await?.error_for_status()?).await?;
        let cancelled = client.delete(format!("{url}/v1/jobs/{id}")).bearer_auth(&token).send().await?;
        ensure!(cancelled.status() == reqwest::StatusCode::ACCEPTED, "cancellation was not accepted");
        let events = client.get(&events_url).bearer_auth(&token).send().await?.error_for_status()?.text().await?;
        ensure!(events.contains("\"cancelled\""), "job did not report cancellation");
        Ok(token_file)
    }
    .await;
    if let Some(mut stdin) = host.stdin.take() {
        let _ = stdin.write_all(b"\n").await;
    }
    let status = timeout(Duration::from_secs(30), host.wait()).await??;
    ensure!(status.success(), "fixture exited unsuccessfully: {status}");
    let token_file = result?;
    let temporary_home = token_file
        .parent()
        .and_then(|path| path.parent())
        .context("temporary fixture home is missing")?;
    ensure!(!temporary_home.exists(), "owned fixture state remains after shutdown");
    Ok(())
}

async fn reaches_approval(mut response: reqwest::Response) -> Result<()> {
    let mut buffer = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        buffer.extend_from_slice(&chunk);
        while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
            let event: Value = serde_json::from_slice(&buffer[.. end])?;
            buffer.drain(..= end);
            if event["event"]["type"] == "interaction" {
                return Ok(());
            }
        }
    }
    anyhow::bail!("native provider did not reach approval")
}
