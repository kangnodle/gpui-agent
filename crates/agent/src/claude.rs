use crate::{AuthStatus,
            Completion,
            Config,
            Event,
            Handler,
            Model,
            Prompt,
            process,
            required_string,
            rpc::Rpc,
            text};
use anyhow::{Result,
             ensure};
use serde_json::{Value,
                 json};
use std::process::Stdio;
use tokio::io::{AsyncReadExt,
                BufReader};

pub(crate) async fn status(config: &Config) -> Result<AuthStatus> {
    let mut child = process::spawn(
        process::command(config)
            .args(["auth", "status", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )?;
    let mut output = Vec::new();
    let stdout = match child.stdout.take() {
        | Some(stdout) => stdout,
        | None => anyhow::bail!("Claude auth status stdout pipe is unavailable"),
    };
    stdout.take((process::MAX_LINE + 1) as u64).read_to_end(&mut output).await?;
    ensure!(output.len() <= process::MAX_LINE, "Claude auth response is too large");
    let status = child.wait().await?;
    let value: Value = serde_json::from_slice(&output).map_err(|_| anyhow::anyhow!("Claude auth status did not return valid JSON ({status})"))?;
    let authenticated = value["loggedIn"]
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("Claude auth status has no loggedIn field"))?;
    ensure!(
        status.success() || (!authenticated && status.code() == Some(1)),
        "Claude auth status failed ({status})"
    );
    Ok(AuthStatus {
        authenticated,
        account: value["email"].as_str().map(str::to_owned),
        method: value["authMethod"].as_str().map(str::to_owned),
    })
}

pub(crate) async fn login(config: &Config, handler: &mut Handler<'_>) -> Result<AuthStatus> {
    let mut child = process::spawn(
        process::command(config)
            .args(["auth", "login"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    let mut stdout = BufReader::new(match child.stdout.take() {
        | Some(pipe) => pipe,
        | None => anyhow::bail!("Claude login stdout pipe is unavailable"),
    });
    let mut stderr = BufReader::new(match child.stderr.take() {
        | Some(pipe) => pipe,
        | None => anyhow::bail!("Claude login stderr pipe is unavailable"),
    });
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut last_url = None;
    while stdout_open || stderr_open {
        let line = tokio::select! {
            line = process::line(&mut stdout), if stdout_open => {
                match line? { Some(line) => line, None => { stdout_open = false; continue; } }
            }
            line = process::line(&mut stderr), if stderr_open => {
                match line? { Some(line) => line, None => { stderr_open = false; continue; } }
            }
        };
        if let Some(url) = authorization_url(&line)
            && last_url.as_ref() != Some(&url)
        {
            handler
                .emit_async(Event::Authorization {
                    url: url.clone(),
                    code: None,
                })
                .await;
            last_url = Some(url);
        }
    }
    ensure!(child.wait().await?.success(), "Claude login failed; retry in a local browser");
    let status = status(config).await?;
    ensure!(status.authenticated, "Claude login completed without authenticated credentials");
    Ok(status)
}

fn authorization_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start ..];
    let end = rest.find(|c: char| c.is_whitespace() || c.is_control()).unwrap_or(rest.len());
    let url = &rest[.. end];
    let parsed = url::Url::parse(url).ok()?;
    (matches!(parsed.host_str(), Some("claude.com" | "claude.ai" | "platform.claude.com"))
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port_or_known_default() == Some(443)
        && parsed.path().contains("oauth/authorize"))
    .then(|| url.to_owned())
}

pub(crate) async fn logout(config: &Config) -> Result<()> { process::inherited(config, &["auth", "logout"]).await }

pub(crate) fn models() -> Vec<Model> {
    // shortcut: Claude CLI has no model-list command; use stable aliases until
    // it exposes a catalog.
    ["sonnet", "opus", "haiku"]
        .into_iter()
        .map(|id| Model {
            id: id.into(),
            name: id.into(),
        })
        .collect()
}

pub(crate) async fn prompt(
    config: &Config,
    prompt: &Prompt,
    handler: &mut Handler<'_>,
    mcp: Option<&gpui_agent_ports::McpConnection>,
    tokens: &std::collections::BTreeMap<String, String>,
) -> Result<Completion> {
    turn(config, prompt, handler, mcp, tokens).await
}

async fn turn(
    config: &Config,
    prompt: &Prompt,
    handler: &mut Handler<'_>,
    mcp: Option<&gpui_agent_ports::McpConnection>,
    tokens: &std::collections::BTreeMap<String, String>,
) -> Result<Completion> {
    ensure!(status(config).await?.authenticated, "Claude is signed out; run login claude");
    let mut command = process::command(config);
    crate::mcp::environment(&mut command, mcp);
    command.args([
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--input-format",
        "stream-json",
        "--permission-prompt-tool",
        "stdio",
        "--settings",
        "{\"disableAllHooks\":true}",
        "--strict-mcp-config",
        "--mcp-config",
    ]);
    let servers: serde_json::Map<String, Value> = crate::mcp::servers(config, mcp)?
        .into_iter()
        .map(|s| match s {
            | gpui_agent_domain::McpServer::Stdio {
                name,
                command,
                args,
            } => Ok((name, json!({"command":command,"args":args}))),
            | gpui_agent_domain::McpServer::Http {
                name,
                url,
                bearer_token_env_var,
                env_http_headers,
                oauth,
            } => {
                let mut headers = crate::mcp::http_headers(bearer_token_env_var.as_deref(), &env_http_headers)?;
                if oauth.is_some() {
                    headers.insert(
                        "Authorization".into(),
                        format!("Bearer {}", tokens.get(&name).ok_or_else(|| anyhow::anyhow!("MCP OAuth login required"))?),
                    );
                }
                Ok((name, json!({"type":"http","url":url,"headers":headers})))
            },
        })
        .collect::<Result<_>>()?;
    // Keep resolved credentials out of process arguments; the file is deleted
    // when this turn ends.
    let mut mcp_config = tempfile::NamedTempFile::new_in(&config.home)?;
    serde_json::to_writer(&mut mcp_config, &json!({"mcpServers":servers}))?;
    command.arg(mcp_config.path());
    command.args([
        "--permission-mode",
        prompt.claude_permission_mode.unwrap_or_default().as_str(),
        "--allow-dangerously-skip-permissions",
    ]);
    if prompt.guidance.is_some() {
        command.arg("--disable-slash-commands");
    }
    if let Some(model) = &prompt.model {
        command.arg("--model").arg(model);
    }
    if let Some(id) = &prompt.session_id {
        command.arg("--resume").arg(id);
    }
    let mut rpc = Rpc::spawn(&mut command, false)?;
    rpc.claude_start(prompt, handler).await?;
    let mut output = String::new();
    let mut streamed = false;
    loop {
        let message = rpc.next(handler).await?;
        for event in crate::tools::claude(&message) {
            handler.emit_async(event).await;
        }
        match message["type"].as_str() {
            | Some("system") if message["subtype"] == "init" => {
                handler
                    .emit_async(Event::Session {
                        id: required_string(&message, "/session_id")?.into(),
                    })
                    .await;
            },
            | Some("stream_event") if message["event"]["delta"]["type"] == "text_delta" => {
                streamed = true;
                let delta = required_string(&message, "/event/delta/text")?;
                text(handler, &mut output, delta).await;
            },
            | Some("result") => {
                if let Some(event) = crate::usage::claude(&message) {
                    handler.emit_async(event).await;
                }
                ensure!(
                    message["is_error"] != true && message["subtype"] == "success",
                    "Claude turn failed: {}",
                    message["result"].as_str().unwrap_or("turn limit, permission denial, or runtime error")
                );
                if !streamed && let Some(result) = message["result"].as_str() {
                    text(handler, &mut output, result).await;
                }
                let session_id = required_string(&message, "/session_id")?.into();
                rpc.close_input().await?;
                rpc.finish().await?;
                return Ok(Completion {
                    session_id,
                    text: output,
                });
            },
            | _ => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_only_claude_authorization_links() {
        let url = "https://claude.com/cai/oauth/authorize?state=fixture";
        assert_eq!(
            authorization_url(&format!("If browser did not open: \x1b]8;;{url}\x1b\\{url}")),
            Some(url.into())
        );
        assert!(authorization_url("https://evil.test/oauth/authorize").is_none());
        assert!(authorization_url("https://claude.ai/privacy").is_none());
    }
}
