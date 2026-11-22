#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
//! Local HTTP adapter. Runtime and storage implementations are injected by the
//! host.
mod control;
mod environment;
mod guidance;
mod jobs;
mod mcp;
mod mcp_bridge;
mod mcp_config;
mod plugins;
pub use mcp_bridge::run_mcp_bridge;
mod web;
use anyhow::Result;
use axum::{Extension,
           Json,
           Router,
           body::{Body,
                  Bytes},
           extract::{DefaultBodyLimit,
                     Path,
                     Query,
                     Request,
                     State},
           http::{StatusCode,
                  header},
           middleware::{self,
                        Next},
           response::{IntoResponse,
                      Response},
           routing::{delete,
                     get,
                     post}};
use gpui_agent_application::{Application,
                             ApplicationError,
                             ErrorKind};
pub use gpui_agent_contracts::*;
use std::convert::Infallible;
use tokio_stream::{StreamExt,
                   wrappers::ReceiverStream};

#[derive(Clone)]
pub struct Api {
    application: Application,
    jobs: Option<gpui_agent_application::Jobs>,
    workspaces: Option<gpui_agent_application::Workspaces>,
    skill_proposals: gpui_agent_application::SkillProposals,
    mcp_proposals: gpui_agent_application::McpProposals,
    controls: Option<std::sync::Arc<dyn gpui_agent_application::ports::HostControl>>,
    token: String,
    mcp_read_token: Option<String>,
    mcp_sessions: Option<std::sync::Arc<dyn gpui_agent_application::ports::McpSessionAuthorizer>>,
    mcp_diagnostics: Option<std::sync::Arc<dyn gpui_agent_application::ports::McpDiagnosticSink>>,
}

impl Api {
    pub fn new(application: Application, token: String) -> Result<Self> {
        anyhow::ensure!(
            token.len() >= 32 && token.bytes().all(|b| b.is_ascii_alphanumeric()),
            "API token must contain at least 32 ASCII letters/digits"
        );
        Ok(Self {
            application,
            jobs: None,
            workspaces: None,
            skill_proposals: Default::default(),
            mcp_proposals: Default::default(),
            token,
            controls: None,
            mcp_read_token: None,
            mcp_sessions: None,
            mcp_diagnostics: None,
        })
    }

    pub fn with_controls(mut self, controls: std::sync::Arc<dyn gpui_agent_application::ports::HostControl>) -> Self {
        self.controls = Some(controls);
        self
    }

    pub fn with_mcp_read_token(mut self, token: String) -> Result<Self> {
        anyhow::ensure!(
            token != self.token && token.len() >= 32 && token.bytes().all(|b| b.is_ascii_alphanumeric()),
            "MCP read token must be distinct and contain at least 32 ASCII letters/digits"
        );
        self.mcp_read_token = Some(token);
        Ok(self)
    }

    pub fn with_mcp_sessions(mut self, sessions: std::sync::Arc<dyn gpui_agent_application::ports::McpSessionAuthorizer>) -> Self {
        self.mcp_sessions = Some(sessions);
        self
    }

    pub fn with_jobs(mut self, jobs: gpui_agent_application::Jobs) -> Self {
        self.jobs = Some(jobs);
        self
    }

    pub fn with_mcp_diagnostics(mut self, sink: std::sync::Arc<dyn gpui_agent_application::ports::McpDiagnosticSink>) -> Self {
        self.mcp_diagnostics = Some(sink);
        self
    }

    pub fn with_workspaces(mut self, workspaces: gpui_agent_application::Workspaces) -> Self {
        self.workspaces = Some(workspaces);
        self
    }

    pub fn router(&self) -> Router {
        Router::new()
            .merge(web::routes())
            .merge(control::routes())
            .merge(jobs::routes())
            .merge(environment::routes())
            .merge(guidance::routes())
            .merge(plugins::routes())
            .merge(mcp_config::routes())
            .merge(mcp::routes())
            .route("/v1/runtimes/{provider}", get(runtime_inspect).post(runtime_command))
            .route("/v1/projects", get(projects).post(register_project))
            .route("/v1/projects/{id}", delete(forget_project))
            .route("/v1/projects/{id}/verification", axum::routing::put(set_project_verification))
            .route("/v1/projects/{id}/worktrees", post(create_worktree))
            .route("/v1/workspaces/{id}", delete(remove_worktree))
            .route("/v1/workspaces/{id}/git", get(git_inspect).post(git_command))
            .route("/v1/workspaces/{id}/changes", get(changes))
            .route("/v1/workspaces/{id}/diff", get(diff))
            .route("/v1/providers", get(providers))
            .route("/v1/providers/{provider}/status", get(status))
            .route("/v1/providers/{provider}/models", get(models))
            .route("/v1/providers/{provider}/login", post(login))
            .route("/v1/providers/{provider}/logout", post(logout))
            .route("/v1/providers/{provider}/chat", post(chat))
            .route("/v1/providers/{provider}/context", post(file_context))
            .route("/v1/providers/{provider}/files", get(workspace_files))
            .route("/v1/providers/{provider}/conversations", get(conversations))
            .route(
                "/v1/providers/{provider}/conversations/{id}",
                get(conversation).patch(rename_conversation).delete(delete_conversation),
            )
            .route("/v1/providers/{provider}/conversations/{id}/state", axum::routing::patch(conversation_state))
            .route("/v1/providers/{provider}/conversations/{id}/usage", get(conversation_usage))
            .route("/v1/providers/{provider}/conversations/{id}/versions", get(conversation_versions))
            .route("/v1/providers/{provider}/conversations/{id}/revisions", post(revise_conversation))
            .route("/v1/operations/{id}", delete(cancel))
            .route("/v1/operations/{id}/interactions/{request_id}", post(respond))
            .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
            .layer(middleware::from_fn_with_state(self.clone(), authenticate))
            .with_state(self.clone())
    }

    pub fn cancel_all(&self) {
        if let Some(sessions) = &self.mcp_sessions {
            sessions.revoke_all();
        }
        if let Some(jobs) = &self.jobs {
            jobs.cancel_all();
        }
        if let Some(controls) = &self.controls {
            controls.shutdown();
        }
        self.application.cancel_all();
        if let Some(workspaces) = &self.workspaces {
            workspaces.cancel_all();
        }
    }
}

fn stream(rx: tokio::sync::mpsc::Receiver<StreamEvent>) -> Response {
    let stream = ReceiverStream::new(rx).map(|event| {
        let mut data = serde_json::to_vec(&event).expect("stream events are JSON serializable");
        data.push(b'\n');
        Ok::<_, Infallible>(Bytes::from(data))
    });
    (
        [(header::CONTENT_TYPE, "application/x-ndjson"), (header::CACHE_CONTROL, "no-store")],
        Body::from_stream(stream),
    )
        .into_response()
}

struct ApiError(StatusCode, String);
type ApiResult<T> = std::result::Result<T, ApiError>;
impl IntoResponse for ApiError {
    fn into_response(self) -> Response { (self.0, Json(serde_json::json!({"error":self.1}))).into_response() }
}
impl From<ApplicationError> for ApiError {
    fn from(error: ApplicationError) -> Self {
        let code = match error.0 {
            | ErrorKind::Invalid => StatusCode::BAD_REQUEST,
            | ErrorKind::NotFound => StatusCode::NOT_FOUND,
            | ErrorKind::Busy => StatusCode::CONFLICT,
            | ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            | ErrorKind::Failure => StatusCode::BAD_GATEWAY,
        };
        Self(code, error.1)
    }
}

async fn authenticate(State(api): State<Api>, mut request: Request, next: Next) -> Response {
    if request.uri().path() == "/mcp" && request.headers().contains_key(header::ORIGIN) && !web::same_origin(&request) {
        return ApiError(StatusCode::FORBIDDEN, "Invalid MCP origin".into()).into_response();
    }
    if web::public(request.uri().path()) && matches!(*request.method(), axum::http::Method::GET | axum::http::Method::HEAD) {
        let mut response = next.run(request).await;
        response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, header::HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"));
        response
            .headers_mut()
            .insert(header::X_CONTENT_TYPE_OPTIONS, header::HeaderValue::from_static("nosniff"));
        response
            .headers_mut()
            .insert(header::REFERRER_POLICY, header::HeaderValue::from_static("no-referrer"));
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
        return response;
    }
    let expected = format!("Bearer {}", api.token);
    let session = if request.uri().path() == "/mcp" {
        request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(|token| api.mcp_sessions.as_ref().and_then(|sessions| sessions.resolve(token).ok().flatten()))
    } else {
        None
    };
    let read_only = request.uri().path() == "/mcp"
        && api.mcp_read_token.as_ref().is_some_and(|token| {
            request.headers().get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()) == Some(format!("Bearer {token}").as_str())
        });
    if (request.headers().contains_key(header::ORIGIN) && !web::same_origin(&request))
        || (session.is_none() && !read_only && request.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some(&expected))
    {
        return ApiError(StatusCode::UNAUTHORIZED, "A local API bearer token is required".into()).into_response();
    }
    if request.uri().path() == "/mcp" {
        request.extensions_mut().insert(mcp::Access {
            execution_id: session.as_ref().map(|grant| grant.execution_id),
            job_id: session.as_ref().and_then(|grant| grant.job_id),
            credential_active: None,
            read_only: read_only || session.as_ref().is_some_and(|grant| !grant.can_write),
            conversation_id: session.as_ref().and_then(|grant| grant.conversation_id),
            scope: session.map(|grant| grant.scope),
        });
    }
    let queue_route = request.uri().path().ends_with("/jobs") || request.uri().path().ends_with("/accounts");
    let scoped = !queue_route
        && (request.uri().path().starts_with("/v1/providers")
            || request.uri().path().starts_with("/v1/operations")
            || request.uri().path().starts_with("/v1/guidance")
            || request.uri().path().starts_with("/v1/skills")
            || request.uri().path().starts_with("/v1/plugins"));
    let mut account_scope = None;
    if scoped && request.headers().get("x-knd-account").is_some_and(|value| value != "default") {
        let selected = match jobs::selection(request.headers()) {
            | Ok(value) => value,
            | Err(error) => return error.into_response(),
        };
        let provider = request
            .uri()
            .path()
            .split('/')
            .nth(3)
            .and_then(|name| PROVIDERS.into_iter().find(|provider| provider_name(*provider) == name))
            .or_else(|| {
                request
                    .headers()
                    .get("x-knd-provider")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|name| PROVIDERS.into_iter().find(|provider| provider_name(*provider) == name))
            });
        if let Some(provider) = provider {
            let Some(jobs) = &api.jobs else {
                return ApiError(StatusCode::SERVICE_UNAVAILABLE, "Account profiles require background jobs support".into()).into_response();
            };
            match jobs.account_scope(provider, selected.0, selected.1).await {
                | Ok(scope) => account_scope = Some(scope),
                | Err(error) => return ApiError::from(error).into_response(),
            }
        } else if request.uri().path().starts_with("/v1/operations/") {
            return ApiError(StatusCode::BAD_REQUEST, "X-KND-Provider is required for account operation routes".into()).into_response();
        }
    }
    if let Some(scope) = &account_scope {
        request.extensions_mut().insert(scope.application.clone());
    }
    let mut scope = None;
    if scoped && account_scope.is_none() {
        let workspace = if let Some(value) = request.headers().get("x-knd-workspace") {
            match value.to_str().ok().and_then(|value| value.parse::<i64>().ok()).filter(|id| *id > 0) {
                | Some(id) => Some(id),
                | None => return ApiError(StatusCode::BAD_REQUEST, "Invalid workspace ID".into()).into_response(),
            }
        } else {
            None
        };
        if let Some(workspaces) = &api.workspaces {
            if request.uri().path().starts_with("/v1/operations/") {
                match workspaces.operation_application(workspace) {
                    | Ok(application) => {
                        request.extensions_mut().insert(application);
                    },
                    | Err(error) => return ApiError::from(error).into_response(),
                }
            } else {
                match workspaces.scope(workspace).await {
                    | Ok(value) => scope = Some(value),
                    | Err(error) => return ApiError::from(error).into_response(),
                }
            }
        } else if workspace.is_some() {
            return ApiError(StatusCode::NOT_FOUND, "Workspace not found".into()).into_response();
        }
        if request.extensions().get::<Application>().is_none() {
            request
                .extensions_mut()
                .insert(scope.as_ref().map(|scope| scope.application.clone()).unwrap_or_else(|| api.application.clone()));
        }
    }
    let mut response = next.run(request).await;
    let cache_control = match "no-store".parse::<axum::http::HeaderValue>() {
        | Ok(value) => value,
        | Err(error) => return ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("Cannot encode cache policy: {error}")).into_response(),
    };
    response.headers_mut().insert(header::CACHE_CONTROL, cache_control);
    response
}

async fn providers(State(api): State<Api>, Extension(application): Extension<Application>) -> Json<Vec<ProviderInfo>> {
    let mut providers = application.providers();
    for provider in &mut providers {
        provider.capabilities.background_jobs = api.jobs.is_some();
    }
    Json(providers)
}
async fn status(Extension(application): Extension<Application>, Path(provider): Path<Provider>) -> ApiResult<Json<AuthStatus>> {
    Ok(Json(application.status(provider).await?))
}
async fn models(Extension(application): Extension<Application>, Path(provider): Path<Provider>) -> ApiResult<Json<Vec<Model>>> {
    Ok(Json(application.models(provider).await?))
}
async fn logout(Extension(application): Extension<Application>, Path(provider): Path<Provider>) -> ApiResult<Json<AuthStatus>> {
    Ok(Json(application.logout(provider).await?))
}
async fn login(Extension(application): Extension<Application>, Path(provider): Path<Provider>, Json(request): Json<LoginRequest>) -> ApiResult<Response> {
    Ok(stream(application.login(provider, request).await?))
}
async fn file_context(
    Extension(application): Extension<Application>,
    Path(_provider): Path<Provider>,
    Json(request): Json<FileContextRequest>,
) -> ApiResult<Json<Attachment>> {
    Ok(Json(application.file_context(request).await?))
}

async fn workspace_files(
    Extension(application): Extension<Application>,
    Path(_provider): Path<Provider>,
    Query(request): Query<WorkspaceFilesRequest>,
) -> ApiResult<Json<WorkspaceFiles>> {
    Ok(Json(application.workspace_files(request).await?))
}
async fn chat(
    State(api): State<Api>,
    Extension(application): Extension<Application>,
    Path(provider): Path<Provider>,
    headers: axum::http::HeaderMap,
    Json(request): Json<ChatRequest>,
) -> ApiResult<Response> {
    if let Some(workspaces) = &api.workspaces {
        let (_, workspace) = jobs::selection(&headers)?;
        if !workspaces.verification_plan(workspace).await?.commands.is_empty() {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "This project requires verified durable jobs. Submit through /v1/providers/{provider}/jobs and follow its events.".into(),
            ));
        }
    }
    Ok(stream(application.chat(provider, request).await?))
}
#[derive(Default, serde::Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    q: String,
    archived: Option<String>,
}
async fn conversations(
    State(api): State<Api>,
    Extension(application): Extension<Application>,
    Path(provider): Path<Provider>,
    headers: axum::http::HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> ApiResult<Json<Vec<ConversationSummary>>> {
    let mut conversations = application.conversations(provider, query.q).await?;
    if let Some(jobs) = &api.jobs {
        let (account, workspace) = jobs::selection(&headers)?;
        let workspace = match (&api.workspaces, workspace) {
            | (Some(workspaces), None) => workspaces.catalog().await?.default_workspace_id,
            | (_, workspace) => workspace,
        };
        let jobs = jobs.list().await?;
        for conversation in &mut conversations {
            conversation.execution_state = jobs
                .iter()
                .filter(|job| {
                    job.scope.provider == provider
                        && job.scope.account_id == account
                        && job.scope.workspace_id == workspace
                        && job.conversation_id == Some(conversation.id)
                })
                .min_by_key(|job| match job.state {
                    | JobState::Waiting => 0,
                    | JobState::Running => 1,
                    | JobState::Queued => 2,
                    | _ => 3,
                })
                .map(|job| job.state);
        }
    }
    match query.archived.as_deref().unwrap_or("active") {
        | "active" => conversations.retain(|c| !c.archived),
        | "archived" => conversations.retain(|c| c.archived),
        | "all" => {},
        | _ => return Err(ApiError(StatusCode::BAD_REQUEST, "archived must be active, archived or all".into())),
    }
    Ok(Json(conversations))
}
async fn conversation(Extension(application): Extension<Application>, Path((provider, id)): Path<(Provider, i64)>) -> ApiResult<Json<SavedConversation>> {
    Ok(Json(application.conversation(provider, id).await?))
}

async fn conversation_versions(
    Extension(application): Extension<Application>,
    Path((provider, id)): Path<(Provider, i64)>,
) -> ApiResult<Json<Vec<ConversationVersion>>> {
    Ok(Json(application.conversation_versions(provider, id).await?))
}

async fn revise_conversation(
    Extension(application): Extension<Application>,
    Path((provider, id)): Path<(Provider, i64)>,
    Json(request): Json<ReviseRequest>,
) -> ApiResult<Json<PreparedRevision>> {
    Ok(Json(application.revise_conversation(provider, id, request).await?))
}
async fn cancel(Extension(application): Extension<Application>, Path(id): Path<u64>) -> StatusCode {
    if application.cancel(id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}
async fn rename_conversation(
    Extension(application): Extension<Application>,
    Path((provider, id)): Path<(Provider, i64)>,
    Json(request): Json<RenameRequest>,
) -> ApiResult<StatusCode> {
    application.rename_conversation(provider, id, request.title).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_conversation(
    State(api): State<Api>,
    Extension(application): Extension<Application>,
    Path((provider, id)): Path<(Provider, i64)>,
    headers: axum::http::HeaderMap,
) -> ApiResult<StatusCode> {
    if let Some(jobs) = &api.jobs {
        let (account, workspace) = jobs::selection(&headers)?;
        let _guard = jobs.mutation_guard().await;
        jobs.purge_conversation(provider, account, workspace, id).await?;
        application.delete_conversation(provider, id).await?;
    } else {
        application.delete_conversation(provider, id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Rejects public binds even if the caller creates the listener itself.
pub async fn serve(listener: tokio::net::TcpListener, api: Api, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<()> {
    anyhow::ensure!(listener.local_addr()?.ip().is_loopback(), "API must listen on loopback");
    if let Some(jobs) = &api.jobs {
        jobs.start().await?;
    }
    let cleanup = api.clone();
    axum::serve(listener, api.router())
        .with_graceful_shutdown(async move {
            shutdown.await;
            cleanup.cancel_all();
        })
        .await?;
    Ok(())
}

async fn respond(
    Extension(application): Extension<Application>,
    Path((id, request_id)): Path<(u64, u64)>,
    Json(response): Json<InteractionResponse>,
) -> ApiResult<StatusCode> {
    application.respond(id, request_id, response)?;
    Ok(StatusCode::NO_CONTENT)
}

fn workspaces(api: &Api) -> ApiResult<&gpui_agent_application::Workspaces> {
    api.workspaces
        .as_ref()
        .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Project management is unavailable in this host".into()))
}
async fn projects(State(api): State<Api>) -> ApiResult<Json<ProjectCatalog>> {
    Ok(Json(if let Some(workspaces) = &api.workspaces {
        workspaces.catalog().await?
    } else {
        ProjectCatalog::default()
    }))
}
async fn register_project(State(api): State<Api>, Json(request): Json<RegisterProject>) -> ApiResult<Json<ProjectCatalog>> {
    Ok(Json(workspaces(&api)?.register(request.path).await?))
}
async fn set_project_verification(State(api): State<Api>, Path(id): Path<i64>, Json(plan): Json<ProjectVerification>) -> ApiResult<Json<ProjectCatalog>> {
    Ok(Json(workspaces(&api)?.set_project_verification(None, id, plan).await?))
}
async fn create_worktree(State(api): State<Api>, Path(id): Path<i64>, Json(request): Json<CreateWorktree>) -> ApiResult<Json<ProjectCatalog>> {
    Ok(Json(workspaces(&api)?.create(id, request.branch, request.base).await?))
}
async fn forget_project(State(api): State<Api>, Path(id): Path<i64>) -> ApiResult<Json<ProjectCatalog>> {
    Ok(Json(workspaces(&api)?.forget_project_if_idle(id, api.jobs.clone()).await?))
}
async fn remove_worktree(State(api): State<Api>, Path(id): Path<i64>) -> ApiResult<Json<ProjectCatalog>> {
    Ok(Json(workspaces(&api)?.remove_if_idle(id, api.jobs.clone()).await?))
}
async fn changes(State(api): State<Api>, Path(id): Path<i64>) -> ApiResult<Json<GitChanges>> { Ok(Json(workspaces(&api)?.changes(id).await?)) }
#[derive(serde::Deserialize)]
struct DiffQuery {
    path: String,
}
async fn diff(State(api): State<Api>, Path(id): Path<i64>, Query(query): Query<DiffQuery>) -> ApiResult<Json<FileDiff>> {
    Ok(Json(workspaces(&api)?.diff(id, query.path).await?))
}

async fn conversation_state(
    State(api): State<Api>,
    Extension(application): Extension<Application>,
    Path((provider, id)): Path<(Provider, i64)>,
    headers: axum::http::HeaderMap,
    Json(state): Json<ConversationState>,
) -> ApiResult<StatusCode> {
    if let Some(jobs) = api.jobs.as_ref().filter(|_| state.pinned.is_some() || state.archived.is_some()) {
        let (account, workspace) = jobs::selection(&headers)?;
        jobs.conversation_organize(
            None,
            ConversationOrganizeRequest {
                target: ConversationTarget {
                    provider,
                    account: Some(account),
                    workspace,
                },
                id,
                pinned: state.pinned,
                archived: state.archived,
            },
        )
        .await?;
    } else {
        application.set_conversation_state(provider, id, state).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn git_inspect(State(api): State<Api>, Path(id): Path<i64>) -> ApiResult<Json<GitOverview>> {
    git_command(State(api), Path(id), Json(GitCommand::Inspect)).await
}
async fn git_command(State(api): State<Api>, Path(id): Path<i64>, Json(command): Json<GitCommand>) -> ApiResult<Json<GitOverview>> {
    let jobs = api.jobs.clone();
    Ok(Json(
        workspaces(&api)?
            .git(id, command, move || {
                if let Some(jobs) = jobs
                    && jobs.workspace_busy(id)?
                {
                    return Err(ApplicationError(
                        ErrorKind::Busy,
                        "Workspace has pending or running jobs; finish or cancel them first".into(),
                    ));
                }
                Ok(())
            })
            .await?,
    ))
}

async fn conversation_usage(Extension(application): Extension<Application>, Path((provider, id)): Path<(Provider, i64)>) -> ApiResult<Json<UsageReport>> {
    let saved = application.conversation(provider, id).await?;
    Ok(Json(UsageReport::recorded(&saved.timeline, saved.messages.iter().filter(|m| !m.user).count())))
}

async fn runtime_inspect(State(api): State<Api>, Path(provider): Path<Provider>) -> ApiResult<Json<RuntimeInfo>> {
    runtime_command(State(api), Path(provider), Json(RuntimeCommand::Inspect)).await
}
async fn runtime_command(State(api): State<Api>, Path(provider): Path<Provider>, Json(command): Json<RuntimeCommand>) -> ApiResult<Json<RuntimeInfo>> {
    let install = matches!(command, RuntimeCommand::Install { .. });
    let jobs = api.jobs.clone();
    Ok(Json(
        workspaces(&api)?
            .runtime(provider, command, move || {
                if install
                    && let Some(jobs) = jobs
                    && jobs.maintenance_busy()?
                {
                    return Err(ApplicationError(
                        ErrorKind::Busy,
                        "Finish or cancel pending jobs before installing runtimes".into(),
                    ));
                }
                Ok(())
            })
            .await?,
    ))
}
