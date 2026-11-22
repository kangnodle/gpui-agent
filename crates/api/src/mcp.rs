//! Stateless Streamable HTTP adapter. Durable jobs outlive individual MCP
//! calls.
mod attachments;
mod browser;
mod computer;
mod conversations;
mod devices;
mod environment;
mod guidance;
mod mcp_servers;
pub(crate) use mcp_servers::LocalHost;
mod projects;
mod pull_requests;
mod questions;
use super::*;
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value,
                 json};

const VERSION: &str = "2025-11-25";
const VERSIONS: &[&str] = &["2025-03-26", "2025-06-18", VERSION];

#[derive(Clone)]
pub(super) struct Access {
    pub read_only: bool,
    pub scope: Option<McpScope>,
    pub conversation_id: Option<i64>,
    pub execution_id: Option<u64>,
    pub job_id: Option<i64>,
    pub credential_active: Option<gpui_agent_application::ports::ControlValidity>,
}
impl Access {
    fn task_session(&self) -> Option<TaskSession> {
        self.scope.clone().map(|scope| TaskSession {
            scope,
            conversation_id: self.conversation_id,
        })
    }
}
pub(super) fn routes() -> Router<Api> { Router::new().route("/mcp", post(handle).get(|| async { StatusCode::METHOD_NOT_ALLOWED })) }

struct ToolDefinition {
    name: &'static str,
    description: &'static str,
    schema: Value,
    read_only: bool,
}

fn tools() -> Vec<ToolDefinition> {
    let id = json!({"type":"integer","minimum":1});
    let mut registered = vec![
        ToolDefinition {
            name: "list_jobs",
            description: "List durable jobs in this environment.",
            schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "get_job",
            description: "Read a durable job and its original request.",
            schema: json!({"type":"object","properties":{"id":id},"required":["id"],"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "get_job_events",
            description: "Read persisted job events after a sequence cursor. Poll again for an active job.",
            schema: json!({"type":"object","properties":{"id":id,"after":{"type":"integer","minimum":0,"default":0}},"required":["id"],"additionalProperties":false}),
            read_only: true,
        },
        ToolDefinition {
            name: "enqueue_job",
            description: "Submit durable agent work. Reuse command_id only with identical input when retrying. Accepts inline attachments and scoped attachment_ids from upload tools; combined maximum 8 attachments/4 MiB. Text may be empty with attachments. Admission copies uploaded bytes; later staging deletion does not alter accepted jobs or identical retries. Acceptance is not completion; inspect the job and its events.",
            schema: json!({"type":"object","properties":{"provider":{"type":"string","enum":["claude","codex","antigravity"]},"text":{"type":"string"},"attachments":attachments::schema(),"attachment_ids":attachments::ids_schema(),"command_id":{"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_.:-]+$"},"account":{"type":"string","description":"Defaults to the calling session account, or default for an operator."},"workspace":{"type":"integer","minimum":1},"model":{"type":"string"},"conversation_id":{"type":"integer","minimum":1}},"required":["provider","text","command_id"],"additionalProperties":false}),
            read_only: false,
        },
        ToolDefinition {
            name: "cancel_job",
            description: "Request cancellation of a durable job. Read its events to observe completion.",
            schema: json!({"type":"object","properties":{"id":id},"required":["id"],"additionalProperties":false}),
            read_only: false,
        },
    ];
    registered.extend(conversations::tools());
    registered.extend(browser::tools());
    registered.extend(computer::tools());
    registered.extend(devices::tools());
    registered.extend(projects::tools());
    registered.extend(pull_requests::tools());
    registered.extend(questions::tools());
    registered.extend(environment::tools());
    registered.extend(attachments::tools());
    registered.extend(guidance::tools());
    registered.extend(mcp_servers::tools());
    registered
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobId {
    id: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventCursor {
    id: i64,
    #[serde(default)]
    after: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enqueue {
    provider: Provider,
    text: String,
    #[serde(default)]
    attachments: Vec<Attachment>,
    #[serde(default)]
    attachment_ids: Vec<i64>,
    command_id: String,
    #[serde(default)]
    account: Option<String>,
    workspace: Option<i64>,
    model: Option<String>,
    conversation_id: Option<i64>,
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ApplicationError> {
    serde_json::from_value(value).map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
}

struct Executed {
    value: Value,
    job: Option<Job>,
    image: Option<Attachment>,
}

fn encode(value: impl serde::Serialize, job: Option<Job>) -> Result<Executed, ApplicationError> {
    Ok(Executed {
        value: serde_json::to_value(value).map_err(|error| ApplicationError(ErrorKind::Failure, error.to_string()))?,
        job,
        image: None,
    })
}

async fn execute(api: &Api, name: &str, args: Value, access: &Access) -> Result<Executed, ApplicationError> {
    if matches!(name, "host_mcp_propose" | "host_mcp_proposal_status") {
        return mcp_servers::execute(api, name, args, access).await;
    }
    if matches!(
        name,
        "host_skills_list" | "host_skills_read" | "host_skill_propose" | "host_skill_proposal_status"
    ) {
        return guidance::execute(api, name, args, access).await;
    }
    if matches!(name, "computer_capture" | "computer_act" | "computer_close") {
        return computer::execute(api, name, args, access).await;
    }
    if matches!(name, "device_list" | "device_open" | "device_screenshot" | "device_act" | "device_close") {
        return devices::execute(api, name, args, access).await;
    }
    if matches!(name, "environment_read" | "environment_preferences_update") {
        return environment::execute(api, name, args, access).await;
    }
    if matches!(name, "project_list" | "workspace_changes" | "workspace_diff" | "workspace_propose_commit") || projects::operator_only(name) {
        return projects::execute(api, name, args, access).await;
    }
    if matches!(
        name,
        "browser_open"
            | "browser_render_html"
            | "browser_diagnostics"
            | "browser_snapshot"
            | "browser_act"
            | "browser_wait"
            | "browser_capture"
            | "browser_close"
    ) {
        return browser::execute(api, name, args, access).await;
    }
    let jobs = api
        .jobs
        .as_ref()
        .ok_or(ApplicationError(ErrorKind::Unavailable, "Background jobs are unavailable".into()))?;
    match name {
        | "link_pull_request"
        | "unlink_pull_request"
        | "list_thread_pull_requests"
        | "watch_pull_request"
        | "unwatch_pull_request"
        | "write_pull_request_guides"
        | "read_pull_request_guide" => pull_requests::execute(jobs, name, args, access).await,
        | "list_jobs" => {
            let _: Empty = decode(args)?;
            let listed: Vec<_> = jobs
                .list()
                .await?
                .into_iter()
                .filter(|job| access.scope.as_ref().is_none_or(|scope| scope.allows(&job.scope)))
                .collect();
            encode(json!({"jobs":listed}), None)
        },
        | "get_job" => {
            let args: JobId = decode(args)?;
            if args.id <= 0 {
                return Err("Job ID must be positive".into());
            }
            let detail = jobs.get(args.id).await?;
            authorize_job(access, &detail.job)?;
            let job = detail.job.clone();
            encode(detail, Some(job))
        },
        | "get_job_events" => {
            let args: EventCursor = decode(args)?;
            if args.id <= 0 || args.after < 0 {
                return Err("Job ID must be positive and cursor nonnegative".into());
            }
            let job = jobs.job(args.id).await?;
            authorize_job(access, &job)?;
            encode(json!({"events":jobs.events(args.id, args.after).await?}), Some(job))
        },
        | "enqueue_job" => {
            let args: Enqueue = decode(args)?;
            if args.workspace.is_some_and(|id| id <= 0) || args.conversation_id.is_some_and(|id| id <= 0) {
                return Err("Workspace and conversation IDs must be positive".into());
            }
            let request = ConversationSendRequest {
                skills: vec![],
                target: ConversationTarget {
                    provider: args.provider,
                    account: args.account,
                    workspace: args.workspace,
                },
                text: args.text,
                model: args.model,
                id: args.conversation_id,
                attachments: args.attachments,
                attachment_ids: args.attachment_ids,
                command_id: args.command_id,
            };
            let job = jobs.conversation_send(access.scope.as_ref(), request).await?;
            encode(&job, Some(job.clone()))
        },
        | "cancel_job" => {
            let args: JobId = decode(args)?;
            if args.id <= 0 {
                return Err("Job ID must be positive".into());
            }
            let job = jobs.job(args.id).await?;
            authorize_job(access, &job)?;
            jobs.cancel(args.id).await?;
            encode(json!({"accepted":true}), Some(job))
        },
        | "attachment_upload" | "attachment_from_workspace" | "attachment_list" | "attachment_delete" => attachments::execute(jobs, name, args, access).await,
        | "get_pending_questions" | "answer_questions" => questions::execute(jobs, name, args, access).await,
        | _ => conversations::execute(jobs, name, args, access).await,
    }
}

fn authorize_job(access: &Access, job: &Job) -> Result<(), ApplicationError> {
    if access.scope.as_ref().is_some_and(|scope| !scope.allows(&job.scope)) {
        return Err(ApplicationError(ErrorKind::NotFound, "Job not found in this MCP session".into()));
    }
    Ok(())
}

fn failure(id: Value, code: i64, message: &str) -> Value { json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}) }

fn automatic_computer_tool(api: &Api, access: &Access, name: &str) -> bool {
    matches!(name, "computer_capture" | "computer_act" | "computer_close")
        && access.scope.is_some()
        && access.conversation_id.is_some()
        && access.job_id.is_some()
        && access.execution_id.is_some()
        && api.controls.as_ref().is_some_and(|controls| controls.automatic_computer_enabled())
}

async fn request(api: &Api, message: &Value, access: Access, request_id: &str) -> Value {
    let id = message["id"].clone();
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    match message["method"].as_str() {
        | Some("initialize") => {
            let Some(version) = params["protocolVersion"].as_str() else {
                return failure(id, -32602, "protocolVersion is required");
            };
            if !params["capabilities"].is_object() || !params["clientInfo"]["name"].is_string() || !params["clientInfo"]["version"].is_string() {
                return failure(id, -32602, "capabilities and clientInfo are required");
            }
            let version = if VERSIONS.contains(&version) { version } else { VERSION };
            json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"gpui-agent","version":env!("CARGO_PKG_VERSION")}}})
        },
        | Some("ping") => json!({"jsonrpc":"2.0","id":id,"result":{}}),
        | Some("tools/list") => {
            if !params.is_object() || params.get("cursor").is_some() {
                return failure(id, -32602, "This server returns all tools without a cursor");
            }
            let tools: Vec<_> = tools()
                .into_iter()
                .filter(|tool| {
                    !access.read_only
                        || tool.read_only
                        || (tool.name == "workspace_propose_commit" && access.scope.is_some())
                        || automatic_computer_tool(api, &access, tool.name)
                })
                .filter(|tool| {
                    access.scope.is_none() || (tool.name != "answer_questions" && !projects::operator_only(tool.name) && !environment::operator_only(tool.name))
                })
                .map(|tool| json!({"name":tool.name,"description":tool.description,"inputSchema":tool.schema,"annotations":{"readOnlyHint":tool.read_only}}))
                .collect();
            json!({"jsonrpc":"2.0","id":id,"result":{"tools":tools}})
        },
        | Some("tools/call") => {
            let Some(name) = params["name"].as_str() else {
                return failure(id, -32602, "Tool name is required");
            };
            let Some(tool) = tools().into_iter().find(|tool| tool.name == name) else {
                return failure(id, -32602, "Unknown tool");
            };
            if access.read_only
                && !tool.read_only
                && !(name == "workspace_propose_commit" && access.scope.is_some())
                && !automatic_computer_tool(api, &access, name)
            {
                return failure(id, -32001, "Tool requires write access");
            }
            if name == "answer_questions" && access.scope.is_some() {
                return failure(id, -32001, "User question replies require an operator credential");
            }
            if projects::operator_only(name) && access.scope.is_some() {
                return failure(id, -32001, "Project and worktree management require an operator credential");
            }
            if environment::operator_only(name) && access.scope.is_some() {
                return failure(id, -32001, "Environment preferences require an operator credential");
            }
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            if !arguments.is_object() {
                return failure(id, -32602, "Tool arguments must be an object");
            }
            let started = std::time::Instant::now();
            let result = execute(api, name, arguments, &access).await;
            let job = result.as_ref().ok().and_then(|executed| executed.job.as_ref());
            let diagnostic = McpDiagnostic {
                request_id: request_id.into(),
                tool: tool.name.into(),
                ok: result.is_ok(),
                duration_ms: started.elapsed().as_millis(),
                job_id: job.map(|job| job.id),
                operation_id: job.and_then(|job| job.operation_id),
            };
            if let Some(sink) = &api.mcp_diagnostics {
                sink.record(&diagnostic);
            } else {
                eprintln!("{}", json!(diagnostic));
            }
            let result = match result {
                | Ok(executed) => {
                    let mut content = vec![json!({"type":"text","text":executed.value.to_string()})];
                    if let Some(Attachment::Image {
                        mime_type,
                        data,
                        ..
                    }) = executed.image
                    {
                        content.push(json!({"type":"image","mimeType":mime_type,"data":data}));
                    }
                    json!({"content":content,"structuredContent":executed.value,"isError":false})
                },
                | Err(error) => json!({"content":[{"type":"text","text":error.to_string()}],"isError":true}),
            };
            json!({"jsonrpc":"2.0","id":id,"result":result})
        },
        | _ => failure(id, -32601, "Method not found"),
    }
}

async fn handle(State(api): State<Api>, Extension(mut access): Extension<Access>, headers: HeaderMap, body: Bytes) -> Response {
    // Body upload may outlive the credential checked by middleware.
    if access.scope.is_some() {
        let grant = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(|token| api.mcp_sessions.as_ref().and_then(|sessions| sessions.resolve(token).ok().flatten()));
        let Some(grant) = grant else {
            return StatusCode::UNAUTHORIZED.into_response();
        };
        access = Access {
            read_only: !grant.can_write,
            conversation_id: grant.conversation_id,
            execution_id: Some(grant.execution_id),
            job_id: grant.job_id,
            credential_active: {
                let sessions = api.mcp_sessions.clone();
                let token = headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("Bearer "))
                    .unwrap_or("")
                    .to_owned();
                let execution = grant.execution_id;
                Some(std::sync::Arc::new(move || {
                    sessions
                        .as_ref()
                        .is_some_and(|sessions| sessions.resolve(&token).ok().flatten().is_some_and(|grant| grant.execution_id == execution))
                }))
            },
            scope: Some(grant.scope),
        };
    }
    let accepts = headers.get(header::ACCEPT).and_then(|value| value.to_str().ok()).unwrap_or("");
    let accepts: Vec<_> = accepts.split(',').map(|part| part.split(';').next().unwrap_or("").trim()).collect();
    if !accepts.contains(&"application/json") || !accepts.contains(&"text/event-stream") {
        return (StatusCode::NOT_ACCEPTABLE, "Accept must include application/json and text/event-stream").into_response();
    }
    if !headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(';').next() == Some("application/json"))
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    if headers
        .get("mcp-protocol-version")
        .is_some_and(|value| value.to_str().ok().is_none_or(|version| !VERSIONS.contains(&version)))
    {
        return (StatusCode::BAD_REQUEST, "Unsupported MCP protocol version").into_response();
    }
    let message: Value = match serde_json::from_slice(&body) {
        | Ok(value) => value,
        | Err(_) => return (StatusCode::BAD_REQUEST, Json(failure(Value::Null, -32700, "Parse error"))).into_response(),
    };
    if !message.is_object() || message["jsonrpc"] != "2.0" {
        return (StatusCode::BAD_REQUEST, Json(failure(Value::Null, -32600, "Invalid request"))).into_response();
    }
    if message.get("id").is_none() {
        return if matches!(message["method"].as_str(), Some("notifications/initialized" | "notifications/cancelled")) {
            StatusCode::ACCEPTED.into_response()
        } else {
            (StatusCode::BAD_REQUEST, Json(failure(Value::Null, -32600, "Unsupported notification"))).into_response()
        };
    }
    if !(message["id"].is_string() || message["id"].is_i64() || message["id"].is_u64()) || !message["method"].is_string() {
        return (StatusCode::BAD_REQUEST, Json(failure(Value::Null, -32600, "Invalid request ID or method"))).into_response();
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let request_id = format!("{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let mut response = Json(request(&api, &message, access, &request_id).await).into_response();
    response
        .headers_mut()
        .insert("x-request-id", request_id.parse().expect("generated numeric correlation ID"));
    response
}
