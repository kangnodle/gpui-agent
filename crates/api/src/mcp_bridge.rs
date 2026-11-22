//! Credential-free argv; inherited session secrets remain outside JSON-RPC
//! frames.
use anyhow::{Result,
             ensure};
use serde_json::{Value,
                 json};
use std::io::{BufRead,
              Read,
              Write};

pub async fn run_mcp_bridge() -> Result<bool> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new("--mcp-stdio")) {
        return Ok(false);
    }
    let url = reqwest::Url::parse(&std::env::var("GPUI_AGENT_MCP_URL")?)?;
    ensure!(
        url.scheme() == "http"
            && url
                .host_str()
                .is_some_and(|host| host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()))
            && url.port().is_some()
            && url.path() == "/mcp"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "MCP bridge requires an explicit loopback endpoint"
    );
    let token = std::env::var("GPUI_AGENT_MCP_TOKEN")?;
    ensure!(
        token
            .strip_prefix("session_")
            .is_some_and(|secret| secret.len() == 64 && secret.bytes().all(|byte| byte.is_ascii_hexdigit())),
        "MCP bridge requires a session credential"
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut output = std::io::stdout().lock();
    const LIMIT: u64 = 16 * 1024 * 1024;
    loop {
        let mut line = Vec::new();
        if (&mut input).take(LIMIT + 1).read_until(b'\n', &mut line)? == 0 {
            break;
        }
        ensure!(line.len() as u64 <= LIMIT, "MCP input exceeds 16 MiB");
        let message: Value = serde_json::from_slice(&line).map_err(|_| anyhow::anyhow!("Invalid MCP JSON"))?;
        let response = client
            .post(url.clone())
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&message)
            .send()
            .await;
        let reply = match response {
            | Ok(response) if response.status() == reqwest::StatusCode::ACCEPTED => continue,
            | Ok(response) if response.status() == reqwest::StatusCode::UNAUTHORIZED => {
                json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32001,"message":"MCP session expired or was revoked"}})
            },
            | Ok(mut response) => {
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    ensure!(bytes.len() + chunk.len() <= LIMIT as usize, "MCP response exceeds 16 MiB");
                    bytes.extend_from_slice(&chunk);
                }
                serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid MCP response"))?
            },
            | Err(_) => json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32000,"message":"MCP endpoint is unavailable"}}),
        };
        if message.get("id").is_some() {
            serde_json::to_writer(&mut output, &reply)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(true)
}
