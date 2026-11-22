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
             bail,
             ensure};
use serde_json::{Value,
                 json};

async fn connect(
    config: &Config,
    handler: &mut Handler<'_>,
    mcp: Option<&gpui_agent_ports::McpConnection>,
    tokens: Option<&std::collections::BTreeMap<String, String>>,
) -> Result<Rpc> {
    let mut command = process::command(config);
    crate::mcp::environment(&mut command, mcp);
    if let Some(tokens) = tokens {
        for (name, token) in tokens {
            command.env(crate::mcp::oauth_variable(name), token);
        }
    }
    let mut rpc = Rpc::spawn(command.args(["app-server", "--listen", "stdio://"]), false)?;
    rpc.request(
        "initialize",
        json!({"clientInfo":{"name":"gpui_agent","title":"Gpui Agent","version":env!("CARGO_PKG_VERSION")}}),
        handler,
    )
    .await?;
    rpc.notify("initialized", json!({})).await?;
    Ok(rpc)
}

async fn account(rpc: &mut Rpc, handler: &mut Handler<'_>) -> Result<AuthStatus> {
    let value = rpc.request("account/read", json!({"refreshToken":true}), handler).await?;
    let account = &value["account"];
    Ok(AuthStatus {
        authenticated: !account.is_null(),
        account: account["email"].as_str().map(str::to_owned),
        method: account["type"].as_str().map(str::to_owned),
    })
}

pub(crate) async fn status(config: &Config) -> Result<AuthStatus> {
    let handler = &mut |_| false;
    account(&mut connect(config, handler, None, None).await?, handler).await
}

pub(crate) async fn login(config: &Config, device: bool, handler: &mut Handler<'_>) -> Result<AuthStatus> {
    let mut rpc = connect(config, handler, None, None).await?;
    let value = rpc
        .request("account/login/start", json!({"type":if device {"chatgptDeviceCode"} else {"chatgpt"}}), handler)
        .await?;
    let id = required_string(&value, "/loginId")?;
    let url = required_string(&value, if device { "/verificationUrl" } else { "/authUrl" })?;
    crate::browser::validate(url, false)?;
    handler
        .emit_async(Event::Authorization {
            url: url.into(),
            code: value["userCode"].as_str().map(str::to_owned),
        })
        .await;
    loop {
        let message = rpc.next(handler).await?;
        if message["method"] == "account/login/completed" && message["params"]["loginId"] == id {
            ensure!(
                message["params"]["success"] == true,
                "Codex login failed: {}",
                message["params"]["error"].as_str().unwrap_or("login was not approved")
            );
            return account(&mut rpc, handler).await;
        }
    }
}

pub(crate) async fn logout(config: &Config) -> Result<()> {
    connect(config, &mut |_| false, None, None)
        .await?
        .request("account/logout", json!({}), &mut |_| false)
        .await?;
    Ok(())
}

pub(crate) async fn models(config: &Config) -> Result<Vec<Model>> {
    let mut rpc = connect(config, &mut |_| false, None, None).await?;
    let mut models = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let value = rpc.request("model/list", json!({"cursor":cursor,"limit":100}), &mut |_| false).await?;
        let data = value["data"].as_array().ok_or_else(|| anyhow::anyhow!("model/list has no data array"))?;
        for model in data {
            let id = required_string(model, "/id")?;
            models.push(Model {
                id: id.into(),
                name: model["displayName"].as_str().unwrap_or(id).into(),
            });
        }
        let next = value["nextCursor"].clone();
        if next.is_null() {
            break;
        }
        ensure!(next != cursor && models.len() < 10_000, "invalid model pagination");
        cursor = next;
    }
    Ok(models)
}

pub(crate) async fn prompt(
    config: &Config,
    prompt: &Prompt,
    handler: &mut Handler<'_>,
    mcp: Option<&gpui_agent_ports::McpConnection>,
    tokens: &std::collections::BTreeMap<String, String>,
    mut input_control: Option<&mut dyn gpui_agent_ports::TurnInput>,
) -> Result<Completion> {
    let mut rpc = connect(config, handler, mcp, Some(tokens)).await?;
    ensure!(account(&mut rpc, handler).await?.authenticated, "Codex is signed out; run login codex");
    let policy = prompt.codex_execution_policy.unwrap_or_default();
    let mut params = json!({"cwd":config.cwd,"approvalPolicy":policy.approval_policy,"sandbox":policy.sandbox});
    let mut overrides = serde_json::Map::new();
    if prompt.guidance.is_some() {
        overrides.insert("project_doc_max_bytes".into(), json!(0));
        overrides.insert("skills.include_instructions".into(), json!(false));
    }
    for server in crate::mcp::servers(config, mcp)? {
        match server {
            | gpui_agent_domain::McpServer::Stdio {
                name,
                command,
                args,
            } => {
                overrides.insert(format!("mcp_servers.{name}.command"), json!(command));
                overrides.insert(format!("mcp_servers.{name}.args"), json!(args));
            },
            | gpui_agent_domain::McpServer::Http {
                name,
                url,
                bearer_token_env_var,
                env_http_headers,
                oauth,
            } => {
                crate::mcp::http_headers(bearer_token_env_var.as_deref(), &env_http_headers)?;
                overrides.insert(format!("mcp_servers.{name}.url"), json!(url));
                if oauth.is_some() {
                    overrides.insert(format!("mcp_servers.{name}.bearer_token_env_var"), json!(crate::mcp::oauth_variable(&name)));
                } else if let Some(variable) = bearer_token_env_var {
                    overrides.insert(format!("mcp_servers.{name}.bearer_token_env_var"), json!(variable));
                }
                if !env_http_headers.is_empty() {
                    overrides.insert(format!("mcp_servers.{name}.env_http_headers"), json!(env_http_headers));
                }
            },
        }
    }
    if mcp.is_some() {
        overrides.insert(
            format!("mcp_servers.{}.env_vars", crate::mcp::NAME),
            json!([crate::mcp::TOKEN, crate::mcp::URL]),
        );
    }
    params["config"] = json!(overrides);
    if let Some(model) = &prompt.model {
        params["model"] = json!(model);
    }
    let method = if let Some(id) = &prompt.session_id {
        params["threadId"] = json!(id);
        "thread/resume"
    } else {
        "thread/start"
    };
    let started = rpc.request(method, params, handler).await?;
    let session = required_string(&started, "/thread/id")?.to_owned();
    handler
        .emit_async(Event::Session {
            id: session.clone(),
        })
        .await;
    rpc.drain().for_each(drop);
    if mcp.is_some() {
        // Thread creation can finish before its injected MCP catalog is ready.
        let inventory = rpc
            .request(
                "mcpServerStatus/list",
                json!({"threadId":session,"serverName":crate::mcp::NAME,"detail":"toolsAndAuthOnly"}),
                handler,
            )
            .await
            .map_err(|_| anyhow::anyhow!("Host MCP tools are unavailable; Codex turn was not started"))?;
        ensure!(
            inventory["data"].as_array().is_some_and(|servers| servers.iter().any(|server| {
                server["name"] == crate::mcp::NAME
                    && server["toolsError"].is_null()
                    && server["tools"]
                        .as_object()
                        .is_some_and(|tools| tools.values().any(|tool| tool["name"] == "list_jobs"))
            })),
            "Host MCP tools are unavailable; Codex turn was not started"
        );
    }
    let runtime_text = prompt.runtime_text();
    let mut input = vec![json!({"type":"text","text":if runtime_text.trim().is_empty() { "See attached files." } else { &runtime_text }})];
    for attachment in &prompt.attachments {
        if let Some(text) = attachment.text_context() {
            input.push(json!({"type":"text","text":text}));
        }
        if let gpui_agent_domain::Attachment::Image {
            mime_type,
            data,
            ..
        } = attachment
        {
            input.push(json!({"type":"text","text":format!("Attached image: {}", serde_json::to_string(attachment.name())?)}));
            input.push(json!({"type":"image","url":format!("data:{mime_type};base64,{data}")}));
        }
    }
    // A turn/started notification permits scoped progress before the RPC reply.
    let mut early_turn: Option<String> = None;
    let started = rpc
        .request_stream("turn/start", json!({"threadId":session,"input":input}), handler, &mut |message| {
            let params = &message["params"];
            if params["threadId"] != session {
                return None;
            }
            if message["method"] == "turn/started" && early_turn.is_none() {
                early_turn = params["turn"]["id"].as_str().map(str::to_owned);
            }
            if early_turn.as_deref().is_some_and(|turn| params["turnId"].as_str().is_none_or(|id| id == turn)) {
                crate::usage::codex(message)
                    .or_else(|| crate::tools::codex(message))
                    .or_else(|| crate::recovery::codex(message))
            } else {
                None
            }
        })
        .await?;
    let turn_id = required_string(&started, "/turn/id")?.to_owned();
    let mut output = String::new();
    let mut delivered = std::collections::HashSet::new();
    loop {
        let message = if let Some(control) = input_control.as_mut() {
            match rpc.next_or_input(handler, *control).await? {
                | crate::rpc::RpcInput::Message(message) => message,
                | crate::rpc::RpcInput::Input(None) => {
                    input_control = None;
                    continue;
                },
                | crate::rpc::RpcInput::Input(Some(input)) => {
                    if delivered.insert(input.message_id) {
                        ensure!(
                            input.message_id > 0 && !input.text.trim().is_empty() && input.text.len() <= 33000,
                            "Invalid live turn input"
                        );
                        let accepted = rpc
                            .request(
                                "turn/steer",
                                json!({"threadId":session,"expectedTurnId":turn_id,"input":[{"type":"text","text":input.text}]}),
                                handler,
                            )
                            .await
                            .is_ok();
                        handler
                            .emit_async(Event::InputDelivery {
                                message_id: input.message_id,
                                delivered: accepted,
                            })
                            .await;
                        if !accepted {
                            // A later acknowledgement must not skip this unread
                            // mailbox message.
                            input_control = None;
                        }
                    }
                    continue;
                },
            }
        } else {
            rpc.next(handler).await?
        };
        let params = &message["params"];
        if params["threadId"] != session || params["turnId"].as_str().is_some_and(|id| id != turn_id) {
            continue;
        }
        if let Some(event) = crate::usage::codex(&message) {
            handler.emit_async(event).await;
        }
        if let Some(event) = crate::tools::codex(&message) {
            handler.emit_async(event).await;
        }
        if let Some(event) = crate::recovery::codex(&message) {
            handler.emit_async(event).await;
        }
        match message["method"].as_str() {
            | Some("item/agentMessage/delta") => text(handler, &mut output, required_string(params, "/delta")?).await,
            | Some("turn/completed") if params["turn"]["id"] == turn_id => match params["turn"]["status"].as_str() {
                | Some("completed") => {
                    return Ok(Completion {
                        session_id: session,
                        text: output,
                    });
                },
                | _ => bail!(
                    "Codex turn did not complete: {}",
                    params["turn"]["error"]["message"].as_str().unwrap_or("failed or interrupted")
                ),
            },
            | _ => {},
        }
    }
}
