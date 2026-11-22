use crate::{AuthStatus,
            Completion,
            Config,
            Event,
            Handler,
            Model,
            Prompt,
            process,
            required_string,
            rpc::{AuthRequired,
                  Rpc,
                  RpcError}};
use anyhow::{Result,
             ensure};
use serde_json::{Value,
                 json};
use tempfile::TempDir;

struct Runtime {
    rpc: Rpc,
    initialized: Value,
    _temp: TempDir,
}

async fn connect(config: &Config, handler: &mut Handler<'_>) -> Result<Runtime> {
    let harness = config
        .harness
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Antigravity requires --harness pointing to localharness_external"))?;
    let profile = config.home.join("antigravity-acp");
    std::fs::create_dir_all(&profile)?;
    let settings = profile.join("settings.json");
    // create_new avoids overwriting a profile that the user deliberately
    // configured.
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&settings) {
        | Ok(mut file) => {
            use std::io::Write;
            file.write_all(b"{\"auth\":{\"type\":\"oauth-personal\"}}\n")?;
        },
        | Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let settings: Value = serde_json::from_reader(std::fs::File::open(settings)?)?;
            ensure!(
                settings["auth"]["type"] == "oauth-personal",
                "this profile uses a different auth method; select a separate --home for Google subscription login"
            );
        },
        | Err(error) => return Err(error.into()),
    }
    let temp = tempfile::Builder::new().prefix("kangnodle-agy-").tempdir()?;
    let mut command = process::command(config);
    command.env("ANTIGRAVITY_HARNESS_PATH", harness);
    let helper = config
        .browser_helper
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Antigravity requires a browser_helper executable using run_browser_helper"))?;
    command.env("BROWSER", crate::browser::command(helper)?);
    if cfg!(target_os = "linux") {
        command.arg("--uid=");
    }
    if cfg!(windows) {
        command.env("TEMP", temp.path()).env("TMP", temp.path());
    } else {
        command.env("TMPDIR", temp.path());
    }
    let mut rpc = Rpc::spawn(&mut command, true)?;
    let initialized = rpc
        .request(
            "initialize",
            json!({"protocolVersion":2,
        "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},
        "clientInfo":{"name":"gpui-agent","version":env!("CARGO_PKG_VERSION")}}),
            handler,
        )
        .await?;
    ensure!(
        initialized["agentInfo"]["name"] == "antigravity-acp",
        "runtime is not Google's Antigravity ACP server"
    );
    ensure!(initialized["protocolVersion"] == 2, "Antigravity ACP protocol version 2 is required");
    Ok(Runtime {
        rpc,
        initialized,
        _temp: temp,
    })
}

async fn authenticate(runtime: &mut Runtime, handler: &mut Handler<'_>) -> Result<()> {
    ensure!(
        runtime.initialized["authMethods"]
            .as_array()
            .is_some_and(|methods| methods.iter().any(|m| m["id"] == "oauth-personal")),
        "runtime does not advertise Google login"
    );
    runtime.rpc.request("authenticate", json!({"methodId":"oauth-personal"}), handler).await?;
    Ok(())
}

async fn session(
    runtime: &mut Runtime,
    config: &Config,
    id: Option<&str>,
    use_mcp: bool,
    handler: &mut Handler<'_>,
    mcp: Option<&gpui_agent_ports::McpConnection>,
    tokens: Option<&std::collections::BTreeMap<String, String>>,
) -> Result<Value> {
    let servers = if use_mcp { crate::mcp::servers(config, mcp)? } else { vec![] };
    let supports = |transport: &str| {
        runtime
            .initialized
            .pointer(&format!("/capabilities/session/mcp/{transport}"))
            .is_some_and(|v| v.is_object())
            || runtime.initialized.pointer(&format!("/agentCapabilities/mcpCapabilities/{transport}")) == Some(&json!(true))
    };
    let host_http = mcp.is_some() && !supports("stdio") && supports("http");
    for server in &servers {
        let transport = if matches!(server, gpui_agent_domain::McpServer::Http { .. }) || (host_http && server.name() == crate::mcp::NAME) {
            "http"
        } else {
            "stdio"
        };
        ensure!(
            supports(transport),
            "This Antigravity runtime does not advertise MCP {} support; upgrade it to use configured or session MCP tools",
            if transport == "http" { "HTTP" } else { "stdio" }
        );
    }
    let servers: Vec<Value> = servers
        .into_iter()
        .map(|s| match s {
            | gpui_agent_domain::McpServer::Stdio {
                name,
                command,
                args,
            } => {
                if name == crate::mcp::NAME && host_http {
                    let connection = mcp.ok_or_else(|| anyhow::anyhow!("Host MCP connection is missing"))?;
                    return Ok(json!({"type":"http","name":name,"url":connection.url,"headers":[{"name":"Authorization","value":format!("Bearer {}",connection.token)}]}));
                }
                let env = if name == crate::mcp::NAME {
                    mcp.map(|connection| json!([{"name":crate::mcp::TOKEN,"value":connection.token},{"name":crate::mcp::URL,"value":connection.url}]))
                        .unwrap_or_else(|| json!([]))
                } else {
                    json!([])
                };
                Ok(json!({"type":"stdio","name":name,"command":command,"args":args,"env":env}))
            },
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
                        format!(
                            "Bearer {}",
                            tokens
                                .and_then(|tokens| tokens.get(&name))
                                .ok_or_else(|| anyhow::anyhow!("MCP OAuth login required"))?
                        ),
                    );
                }
                let headers: Vec<Value> = headers.into_iter().map(|(name, value)| json!({"name":name,"value":value})).collect();
                Ok(json!({"type":"http","name":name,"url":url,"headers":headers}))
            },
        })
        .collect::<Result<_>>()?;
    let mut params = json!({"cwd":config.cwd,"mcpServers":servers});
    let method = if let Some(id) = id {
        ensure!(
            runtime.initialized.pointer("/agentCapabilities/sessionCapabilities/resume").is_some(),
            "runtime does not support session resume"
        );
        params["sessionId"] = json!(id);
        "session/resume"
    } else {
        "session/new"
    };
    runtime.rpc.request(method, params, handler).await
}

fn authenticated(value: bool) -> AuthStatus {
    AuthStatus {
        authenticated: value,
        account: None,
        method: Some("oauth-personal".into()),
    }
}

pub(crate) async fn login(config: &Config, handler: &mut Handler<'_>) -> Result<AuthStatus> {
    let mut runtime = connect(config, handler).await?;
    runtime.rpc.login_allowed = true;
    authenticate(&mut runtime, handler).await?;
    session(&mut runtime, config, None, false, handler, None, None).await?;
    Ok(authenticated(true))
}

pub(crate) async fn status(config: &Config) -> Result<AuthStatus> {
    if !config.home.join("antigravity-acp/acp_token.json").try_exists()? {
        return Ok(authenticated(false));
    }
    let mut runtime = connect(config, &mut |_| false).await?;
    // BROWSER interception prevents a status probe from opening an interactive
    // sign-in.
    match async {
        authenticate(&mut runtime, &mut |_| false).await?;
        session(&mut runtime, config, None, false, &mut |_| false, None, None).await
    }
    .await
    {
        | Ok(_) => Ok(authenticated(true)),
        | Err(error) if error.is::<AuthRequired>() => Ok(authenticated(false)),
        | Err(error) if error.downcast_ref::<RpcError>().is_some_and(|error| error.code == -32000) => Ok(authenticated(false)),
        | Err(error) => Err(error),
    }
}

pub(crate) async fn logout(config: &Config) -> Result<()> {
    let mut runtime = connect(config, &mut |_| false).await?;
    ensure!(
        runtime
            .initialized
            .pointer("/agentCapabilities/auth/logout")
            .is_some_and(|value| !value.is_null()),
        "runtime does not support logout"
    );
    runtime.rpc.request("logout", json!({}), &mut |_| false).await?;
    Ok(())
}

pub(crate) fn model_options(session: &Value) -> Vec<Model> {
    let mut result = Vec::new();
    if let Some(options) = session["configOptions"].as_array()
        && let Some(model) = options.iter().find(|option| option["id"] == "model")
        && let Some(options) = model["options"].as_array()
    {
        for option in options {
            let entries = option["options"].as_array().map(Vec::as_slice).unwrap_or(std::slice::from_ref(option));
            for entry in entries {
                if let Some(id) = entry["value"].as_str() {
                    result.push(Model {
                        id: id.into(),
                        name: entry["name"].as_str().unwrap_or(id).into(),
                    });
                }
            }
        }
    }
    if result.is_empty()
        && let Some(models) = session.pointer("/models/availableModels").and_then(Value::as_array)
    {
        for model in models {
            if let Some(id) = model["modelId"].as_str() {
                result.push(Model {
                    id: id.into(),
                    name: model["name"].as_str().unwrap_or(id).into(),
                });
            }
        }
    }
    result
}

pub(crate) async fn models(config: &Config) -> Result<Vec<Model>> {
    let mut runtime = connect(config, &mut |_| false).await?;
    authenticate(&mut runtime, &mut |_| false).await?;
    let value = session(&mut runtime, config, None, false, &mut |_| false, None, None).await?;
    Ok(model_options(&value))
}

pub(crate) fn update(message: &Value, session: &str) -> Option<Event> {
    if message["method"] == "session/update"
        && message["params"]["sessionId"] == session
        && let Some(event) = crate::tools::acp(&message["params"]["update"])
    {
        return Some(event);
    }
    if message["method"] == "session/update"
        && message["params"]["sessionId"] == session
        && message["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
        && message["params"]["update"]["content"]["type"] == "text"
    {
        return message.pointer("/params/update/content/text").and_then(Value::as_str).map(|delta| Event::Text {
            delta: delta.into(),
        });
    }
    None
}

pub(crate) async fn prompt(
    config: &Config,
    prompt: &Prompt,
    handler: &mut Handler<'_>,
    mcp: Option<&gpui_agent_ports::McpConnection>,
    tokens: &std::collections::BTreeMap<String, String>,
) -> Result<Completion> {
    let mut runtime = connect(config, handler).await?;
    let capabilities = &runtime.initialized["agentCapabilities"]["promptCapabilities"];
    let runtime_text = prompt.runtime_text();
    let mut content = vec![json!({"type":"text","text":if runtime_text.trim().is_empty() { "See attached files." } else { &runtime_text }})];
    for (index, attachment) in prompt.attachments.iter().enumerate() {
        match attachment {
            | gpui_agent_domain::Attachment::Text {
                name,
                text,
                source,
            } if capabilities["embeddedContext"] == true => {
                content.push(json!({"type":"text","text":format!("Attached file: {}", serde_json::to_string(&(name, source))?)}));
                content.push(json!({"type":"resource","resource":{"uri":format!("kangnodle-attachment:///{index}"),"mimeType":"text/plain","text":text}}));
            },
            | gpui_agent_domain::Attachment::Text {
                ..
            } => match attachment.text_context() {
                | Some(text) => content.push(json!({"type":"text","text":text})),
                | None => anyhow::bail!("Text attachment has no context"),
            },
            | gpui_agent_domain::Attachment::Image {
                mime_type,
                data,
                ..
            } => {
                ensure!(
                    capabilities["image"] == true,
                    "This Antigravity runtime does not advertise image input; attach UTF-8 text instead"
                );
                content.push(json!({"type":"text","text":format!("Attached image: {}", serde_json::to_string(attachment.name())?)}));
                content.push(json!({"type":"image","mimeType":mime_type,"data":data}));
            },
        }
    }
    authenticate(&mut runtime, handler).await?;
    let value = session(&mut runtime, config, prompt.session_id.as_deref(), true, handler, mcp, Some(tokens)).await?;
    let id = match &prompt.session_id {
        | Some(id) => id.clone(),
        | None => required_string(&value, "/sessionId")?.to_owned(),
    };
    if let Some(model) = &prompt.model {
        ensure!(
            model_options(&value).iter().any(|entry| entry.id == *model),
            "model is not in the Antigravity session catalog"
        );
        let config_option = value["configOptions"]
            .as_array()
            .is_some_and(|options| options.iter().any(|option| option["id"] == "model"));
        let (method, params) = if config_option {
            ("session/set_config_option", json!({"sessionId":id,"configId":"model","value":model}))
        } else {
            ("session/set_model", json!({"sessionId":id,"modelId":model}))
        };
        runtime.rpc.request(method, params, handler).await?;
    }
    handler
        .emit_async(Event::Session {
            id: id.clone(),
        })
        .await;
    runtime.rpc.drain().for_each(drop);
    let mut output = String::new();
    // ACP sends deltas before the session/prompt response, so dispatch them
    // during the request.
    let result = runtime
        .rpc
        .request_stream("session/prompt", json!({"sessionId":id,"prompt":content}), handler, &mut |message| {
            let event = update(message, &id);
            if let Some(Event::Text {
                delta,
            }) = &event
            {
                output.push_str(delta);
            }
            event
        })
        .await?;
    ensure!(result["stopReason"] == "end_turn", "Antigravity turn stopped: {}", result["stopReason"]);
    Ok(Completion {
        session_id: id,
        text: output,
    })
}

pub(crate) async fn validate(config: &Config) -> Result<()> {
    let _runtime = connect(config, &mut |_| false).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouped_models_and_legacy_catalog() {
        assert_eq!(
            model_options(&json!({"configOptions":[{"id":"model","options":[{"group":"Gemini","options":[{"value":"gemini-test","name":"Gemini"}]}]}]})),
            vec![Model {
                id: "gemini-test".into(),
                name: "Gemini".into()
            }]
        );
        assert_eq!(
            model_options(&json!({"models":{"availableModels":[{"modelId":"legacy","name":"Legacy"}]}}))[0].id,
            "legacy"
        );
    }
}
