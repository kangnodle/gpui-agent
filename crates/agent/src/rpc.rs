use crate::{Event,
            Handler,
            process};
use anyhow::{Result,
             anyhow,
             bail,
             ensure};
use serde_json::{Value,
                 json};
use std::{collections::{HashMap,
                        VecDeque},
          process::Stdio};
use tokio::{io::{AsyncWriteExt,
                 BufReader},
            process::{ChildStdin,
                      Command},
            sync::mpsc,
            task::JoinHandle};

#[derive(Debug)]
pub(crate) struct RpcError {
    pub code: i64,
    pub message: String,
}
impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "native RPC error {}: {}", self.code, self.message) }
}
impl std::error::Error for RpcError {}

#[derive(Debug)]
pub(crate) struct AuthRequired;
impl std::fmt::Display for AuthRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("sign-in is required; run login antigravity") }
}
impl std::error::Error for AuthRequired {}

pub(crate) enum RpcInput {
    Message(Value),
    Input(Option<gpui_agent_ports::RuntimeInput>),
}

pub(crate) struct Rpc {
    host_tools: HashMap<(String, String, String), String>,
    child: process::Process,
    input: Option<ChildStdin>,
    messages: mpsc::Receiver<Result<String>>,
    readers: Vec<JoinHandle<()>>,
    pending: VecDeque<Value>,
    next_id: u64,
    acp: bool,
    pub login_allowed: bool,
    authorization_url: Option<String>,
}

impl Drop for Rpc {
    fn drop(&mut self) {
        for reader in &self.readers {
            reader.abort();
        }
    }
}

impl Rpc {
    pub fn spawn(command: &mut Command, acp: bool) -> Result<Self> {
        let mut child = process::spawn(command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()))?;
        let input = match child.stdin.take() {
            | Some(pipe) => pipe,
            | None => bail!("Native RPC stdin pipe is unavailable"),
        };
        let (tx, messages) = mpsc::channel(64);
        let mut readers = Vec::new();
        let mut stdout = BufReader::new(match child.stdout.take() {
            | Some(pipe) => pipe,
            | None => bail!("Native RPC stdout pipe is unavailable"),
        });
        let mut stderr = BufReader::new(match child.stderr.take() {
            | Some(pipe) => pipe,
            | None => bail!("Native RPC stderr pipe is unavailable"),
        });
        let out_tx = tx.clone();
        readers.push(tokio::spawn(async move {
            loop {
                match process::line(&mut stdout).await {
                    | Ok(Some(line)) =>
                        if out_tx.send(Ok(line)).await.is_err() {
                            break;
                        },
                    | Ok(None) => {
                        let _ = out_tx.send(Err(anyhow!("native runtime closed stdout"))).await;
                        break;
                    },
                    | Err(error) => {
                        let _ = out_tx.send(Err(error)).await;
                        break;
                    },
                }
            }
        }));
        readers.push(tokio::spawn(async move {
            loop {
                match process::line(&mut stderr).await {
                    | Ok(Some(line)) if line.starts_with(process::AUTH_PREFIX) || line.starts_with(crate::browser::MARKER) => {
                        if tx.send(Ok(line)).await.is_err() {
                            break;
                        }
                    },
                    | Ok(Some(_)) => {},
                    | Ok(None) => break,
                    | Err(error) => {
                        let _ = tx.send(Err(error)).await;
                        break;
                    },
                }
            }
        }));
        Ok(Self {
            child,
            input: Some(input),
            messages,
            readers,
            pending: VecDeque::new(),
            host_tools: HashMap::new(),
            next_id: 0,
            acp,
            login_allowed: false,
            authorization_url: None,
        })
    }

    pub(crate) async fn send(&mut self, mut value: Value) -> Result<()> {
        if self.acp {
            value["jsonrpc"] = json!("2.0");
        }
        let mut encoded = serde_json::to_vec(&value)?;
        encoded.push(b'\n');
        let input = self.input.as_mut().ok_or_else(|| anyhow!("runtime stdin is closed"))?;
        input.write_all(&encoded).await?;
        input.flush().await?;
        Ok(())
    }

    pub async fn notify(&mut self, method: &str, params: Value) -> Result<()> { self.send(json!({"method":method,"params":params})).await }

    pub async fn finish(&mut self) -> Result<()> {
        let status = self.child.wait().await?;
        ensure!(status.success(), "native runtime failed ({status})");
        Ok(())
    }

    async fn receive(&mut self, handler: &mut Handler<'_>) -> Result<Value> {
        loop {
            let line = self.messages.recv().await.ok_or_else(|| anyhow!("native runtime disconnected"))??;
            if let Some(message) = self.receive_one(line, handler).await? {
                return Ok(message);
            }
        }
    }

    pub(crate) async fn next_or_input(&mut self, handler: &mut Handler<'_>, input: &mut dyn gpui_agent_ports::TurnInput) -> Result<RpcInput> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(RpcInput::Message(message));
        }
        loop {
            let line = tokio::select! {
                value=input.next()=>return Ok(RpcInput::Input(value?)),
                line=self.messages.recv()=>line.ok_or_else(||anyhow!("native runtime disconnected"))??,
            };
            if let Some(message) = self.receive_one(line, handler).await? {
                return Ok(RpcInput::Message(message));
            }
        }
    }

    async fn receive_one(&mut self, line: String, handler: &mut Handler<'_>) -> Result<Option<Value>> {
        if let Some(url) = process::authorization(&line)? {
            if !self.login_allowed {
                return Err(AuthRequired.into());
            }
            if self.authorization_url.as_ref() != Some(&url) {
                handler
                    .emit_async(Event::Authorization {
                        url: url.clone(),
                        code: None,
                    })
                    .await;
                self.authorization_url = Some(url);
            }
            return Ok(None);
        }
        if line.trim().is_empty() {
            return Ok(None);
        }
        // Antigravity mixes its sign-in notice with JSON on stdout; other
        // text is a protocol error.
        let message: Value = serde_json::from_str(&line).map_err(|_| anyhow!("native runtime returned invalid JSON"))?;
        ensure!(message.is_object(), "native runtime returned a non-object RPC message");
        if matches!(message["method"].as_str(), Some("item/started" | "item/completed")) {
            let params = &message["params"];
            if let (Some(thread), Some(turn), Some(id)) = (params["threadId"].as_str(), params["turnId"].as_str(), params["item"]["id"].as_str()) {
                let key = (thread.to_owned(), turn.to_owned(), id.to_owned());
                if message["method"] == "item/completed" {
                    self.host_tools.remove(&key);
                } else if params["item"]["type"] == "mcpToolCall" && params["item"]["server"] == crate::mcp::NAME {
                    ensure!(self.host_tools.len() < 256, "Too many active host MCP tools");
                    if let Some(tool) = params["item"]["tool"].as_str() {
                        self.host_tools.insert(key, tool.to_owned());
                    }
                }
            }
        }
        if message["type"] == "control_request" {
            self.claude_control(&message, handler).await?;
            return Ok(None);
        }
        if message.get("method").is_some() && message.get("id").is_some() {
            self.server_request(&message, handler).await?;
            return Ok(None);
        }
        Ok(Some(message))
    }

    async fn server_request(&mut self, message: &Value, handler: &mut Handler<'_>) -> Result<()> {
        let method = message["method"].as_str().unwrap_or("");
        let params = &message["params"];
        let result = match method {
            | "mcpServer/elicitation/request" => {
                let server_name = params["serverName"].as_str().ok_or_else(|| anyhow!("MCP server name missing"))?;
                let message = params["message"].as_str().ok_or_else(|| anyhow!("MCP request message missing"))?;
                let (schema, url) = match params["mode"].as_str() {
                    | Some("form") => {
                        ensure!(
                            params["requestedSchema"]["type"] == "object" && params["requestedSchema"]["properties"].is_object(),
                            "Unsupported MCP form schema"
                        );
                        (Some(params["requestedSchema"].clone()), None)
                    },
                    | Some("url") => {
                        let raw = params["url"].as_str().ok_or_else(|| anyhow!("MCP request URL missing"))?;
                        let parsed = url::Url::parse(raw)?;
                        ensure!(
                            matches!(parsed.scheme(), "https" | "http")
                                && parsed.host_str().is_some()
                                && parsed.username().is_empty()
                                && parsed.password().is_none(),
                            "Unsupported MCP request URL"
                        );
                        (None, Some(raw.to_owned()))
                    },
                    | _ => bail!("Unsupported MCP elicitation mode"),
                };
                let event = Event::Elicitation {
                    server_name: server_name.into(),
                    message: message.into(),
                    schema,
                    url,
                };
                let mut candidates = self.host_tools.iter().filter(|((thread, turn, id), _)| {
                    params["threadId"] == *thread
                        && params["turnId"] == *turn
                        && params
                            .get("itemId")
                            .is_none_or(|item| item.is_null() || item.as_str().is_some_and(|item| item == id))
                });
                let tool = candidates.next().map(|(_, tool)| tool.clone()).filter(|_| candidates.next().is_none());
                let computer = server_name == crate::mcp::NAME
                    && matches!(&event, Event::Elicitation { schema: Some(schema), url: None, .. } if schema["properties"].as_object().is_some_and(|fields| fields.is_empty()))
                    && tool
                        .as_deref()
                        .is_some_and(|tool| matches!(tool, "computer_capture" | "computer_act" | "computer_close"));
                let response = if computer {
                    let response = handler
                        .respond(Event::Permission {
                            method: "computer/native_tool".into(),
                            details: json!({"tool":tool}),
                        })
                        .await;
                    match response {
                        | gpui_agent_domain::InteractionResponse::Approval {
                            allow: true,
                        } => event.elicitation_response("{}").unwrap_or(gpui_agent_domain::InteractionResponse::Cancelled),
                        | gpui_agent_domain::InteractionResponse::Approval {
                            allow: false,
                        } => gpui_agent_domain::InteractionResponse::Elicitation {
                            action: gpui_agent_domain::ElicitationAction::Decline,
                            content: None,
                        },
                        | _ => gpui_agent_domain::InteractionResponse::Cancelled,
                    }
                } else {
                    handler.respond(event.clone()).await
                };
                let response = if gpui_agent_domain::validate_response(&event, &response).is_ok() {
                    response
                } else {
                    gpui_agent_domain::InteractionResponse::Cancelled
                };
                match response {
                    | gpui_agent_domain::InteractionResponse::Elicitation {
                        action,
                        content,
                    } => json!({"action":action,"content":content,"_meta":null}),
                    | _ => json!({"action":"cancel","content":null,"_meta":null}),
                }
            },
            | "item/permissions/requestApproval" => {
                let permissions = params["permissions"].as_object().ok_or_else(|| anyhow!("Invalid requested permissions"))?;
                ensure!(
                    permissions.keys().all(|key| matches!(key.as_str(), "network" | "fileSystem")),
                    "Unsupported requested permissions"
                );
                let response = handler
                    .respond(Event::Permission {
                        method: method.into(),
                        details: params.clone(),
                    })
                    .await;
                let permissions = if matches!(
                    response,
                    gpui_agent_domain::InteractionResponse::Approval {
                        allow: true
                    }
                ) {
                    params["permissions"].clone()
                } else {
                    json!({})
                };
                json!({"permissions":permissions,"scope":"turn"})
            },
            | "session/request_permission" => {
                let response = handler
                    .respond(Event::Permission {
                        method: method.into(),
                        details: params.clone(),
                    })
                    .await;
                let allow = matches!(
                    response,
                    gpui_agent_domain::InteractionResponse::Approval {
                        allow: true
                    }
                );
                if matches!(response, gpui_agent_domain::InteractionResponse::Cancelled) {
                    return self.send(json!({"id":message["id"],"result":{"outcome":{"outcome":"cancelled"}}})).await;
                }
                let kind = if allow { "allow_once" } else { "reject_once" };
                let selected = params["options"]
                    .as_array()
                    .and_then(|options| options.iter().find(|option| option["kind"] == kind))
                    .and_then(|option| option["optionId"].as_str());
                match selected {
                    | Some(id) => json!({"outcome":{"outcome":"selected","optionId":id}}),
                    | None => json!({"outcome":{"outcome":"cancelled"}}),
                }
            },
            | "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                let response = handler
                    .respond(Event::Permission {
                        method: method.into(),
                        details: params.clone(),
                    })
                    .await;
                let allow = matches!(
                    response,
                    gpui_agent_domain::InteractionResponse::Approval {
                        allow: true
                    }
                );
                json!({"decision":if matches!(response,gpui_agent_domain::InteractionResponse::Cancelled) { "cancel" } else if allow { "accept" } else { "decline" }})
            },
            | "item/tool/requestUserInput" | "tool/requestUserInput" => {
                let questions = questions(&params["questions"], false)?;
                match handler
                    .respond(Event::UserInput {
                        questions,
                    })
                    .await
                {
                    | gpui_agent_domain::InteractionResponse::Answers {
                        answers,
                    } => {
                        let answers: serde_json::Map<String, Value> = answers.into_iter().map(|(id, values)| (id, json!({"answers":values}))).collect();
                        json!({"answers":answers})
                    },
                    | _ => json!({"answers":{}}),
                }
            },
            | _ =>
                return self
                    .send(json!({"id":message["id"],"error":{"code":-32601,"message":"Client method is not supported"}}))
                    .await,
        };
        self.send(json!({"id":message["id"],"result":result})).await
    }

    async fn claude_control(&mut self, message: &Value, handler: &mut Handler<'_>) -> Result<()> {
        use gpui_agent_domain::InteractionResponse;
        let request = &message["request"];
        if request["subtype"] != "can_use_tool" {
            return self
                .send(
                    json!({"type":"control_response","response":{"subtype":"error","request_id":message["request_id"],"error":"Unsupported control request"}}),
                )
                .await;
        }
        let mut input = request["input"].clone();
        let response = if request["tool_name"] == "AskUserQuestion" {
            handler
                .respond(Event::UserInput {
                    questions: questions(&input["questions"], true)?,
                })
                .await
        } else {
            handler
                .respond(Event::Permission {
                    method: "claude/can_use_tool".into(),
                    details: request.clone(),
                })
                .await
        };
        let result = match response {
            | InteractionResponse::ClaudeMode {
                mode,
            } if request["tool_name"] != "AskUserQuestion" => {
                json!({"behavior":"allow","updatedInput":input,"updatedPermissions":[{"type":"setMode","mode":mode,"destination":"session"}]})
            },
            | InteractionResponse::ClaudeSuggestion {
                index,
            } if request["tool_name"] != "AskUserQuestion" => {
                let suggestion = request["permission_suggestions"].as_array().and_then(|suggestions| suggestions.get(index));
                let suggestion = match suggestion {
                    | Some(suggestion) if request["suppress_always_allow_rule"] != true => suggestion,
                    | _ => bail!("Claude permission suggestion is unavailable"),
                };
                json!({"behavior":"allow","updatedInput":input,"updatedPermissions":[suggestion]})
            },
            | InteractionResponse::Approval {
                allow: true,
            } => json!({"behavior":"allow","updatedInput":input}),
            | InteractionResponse::Answers {
                answers,
            } => {
                input["answers"] = json!(
                    answers
                        .into_iter()
                        .map(|(id, values)| (id, values.join(", ")))
                        .collect::<std::collections::BTreeMap<_, _>>()
                );
                json!({"behavior":"allow","updatedInput":input})
            },
            | _ => json!({"behavior":"deny","message":"User declined or cancelled the request"}),
        };
        self.send(json!({"type":"control_response","response":{"subtype":"success","request_id":message["request_id"],"response":result}}))
            .await
    }

    pub async fn claude_start(&mut self, prompt: &crate::Prompt, handler: &mut Handler<'_>) -> Result<()> {
        self.send(json!({"type":"control_request","request_id":"initialize","request":{"subtype":"initialize","hooks":{}}}))
            .await?;
        loop {
            let message = self.receive(handler).await?;
            if message["type"] == "control_response" && message["response"]["request_id"] == "initialize" {
                ensure!(message["response"]["subtype"] == "success", "Claude initialization failed");
                break;
            }
            ensure!(self.pending.len() < 1024, "Too many initialization events");
            self.pending.push_back(message);
        }
        let runtime_text = prompt.runtime_text();
        let content = if prompt.attachments.is_empty() {
            json!(runtime_text)
        } else {
            let mut blocks = vec![json!({"type":"text","text":if runtime_text.trim().is_empty() { "See attached files." } else { &runtime_text }})];
            for attachment in &prompt.attachments {
                if let Some(text) = attachment.text_context() {
                    blocks.push(json!({"type":"text","text":text}));
                }
                if let gpui_agent_domain::Attachment::Image {
                    mime_type,
                    data,
                    ..
                } = attachment
                {
                    blocks.push(json!({"type":"text","text":format!("Attached image: {}", serde_json::to_string(attachment.name())?)}));
                    blocks.push(json!({"type":"image","source":{"type":"base64","media_type":mime_type,"data":data}}));
                }
            }
            json!(blocks)
        };
        self.send(json!({"type":"user","session_id":"","message":{"role":"user","content":content},"parent_tool_use_id":null}))
            .await
    }

    pub async fn close_input(&mut self) -> Result<()> {
        if let Some(mut input) = self.input.take() {
            input.shutdown().await?;
        }
        Ok(())
    }

    pub async fn request(&mut self, method: &str, params: Value, handler: &mut Handler<'_>) -> Result<Value> {
        self.request_stream(method, params, handler, &mut |_| None).await
    }

    pub async fn request_stream(
        &mut self,
        method: &str,
        params: Value,
        handler: &mut Handler<'_>,
        notification: &mut (dyn FnMut(&Value) -> Option<Event> + Send),
    ) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id,"method":method,"params":params})).await?;
        loop {
            let message = self.receive(handler).await?;
            if message["id"] == id {
                if let Some(error) = message.get("error") {
                    return Err(RpcError {
                        code: error["code"].as_i64().unwrap_or(-32603),
                        message: error["message"].as_str().unwrap_or("unknown error").to_owned(),
                    }
                    .into());
                }
                return message.get("result").cloned().ok_or_else(|| anyhow!("RPC response has no result"));
            }
            if message.get("method").is_some() {
                if let Some(event) = notification(&message) {
                    handler.emit_async(event).await;
                    continue;
                }
                ensure!(self.pending.len() < 1024, "too many notifications before RPC response");
                self.pending.push_back(message);
            } else {
                bail!("unexpected RPC response id");
            }
        }
    }

    pub async fn next(&mut self, handler: &mut Handler<'_>) -> Result<Value> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(message);
        }
        self.receive(handler).await
    }

    pub fn drain(&mut self) -> impl Iterator<Item = Value> + '_ { self.pending.drain(..) }
}

fn questions(value: &Value, claude: bool) -> Result<Vec<gpui_agent_domain::Question>> {
    let values = value.as_array().ok_or_else(|| anyhow!("Runtime supplied invalid questions"))?;
    ensure!(!values.is_empty() && values.len() <= 16, "Unsupported question count");
    let mut ids = std::collections::HashSet::new();
    values
        .iter()
        .map(|value| {
            let question = value["question"].as_str().ok_or_else(|| anyhow!("Question text missing"))?;
            let id = if claude {
                question
            } else {
                value["id"].as_str().ok_or_else(|| anyhow!("Question ID missing"))?
            };
            ensure!(!id.is_empty() && ids.insert(id.to_owned()), "Duplicate or empty question ID");
            Ok(gpui_agent_domain::Question {
                id: id.into(),
                question: question.into(),
                options: value["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v["label"].as_str().map(str::to_owned))
                    .collect(),
                multi_select: claude && value["multiSelect"] == true,
            })
        })
        .collect()
}
