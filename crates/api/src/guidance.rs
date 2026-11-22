use super::*;

pub(super) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/guidance", get(snapshot))
        .route("/v1/guidance/files", get(files).put(write_file))
        .route("/v1/guidance/files/read", post(read_file))
        .route("/v1/skills/import", post(import_skill))
        .route("/v1/skills", get(catalog))
        .route("/v1/skills/read", post(read))
        .route("/v1/skills/enabled", axum::routing::put(set_enabled))
        .route("/v1/skill-proposals", get(pending_proposals))
        .route("/v1/skill-proposals/{id}", post(decide_proposal))
        .route("/v1/mcp-proposals", get(pending_mcp_proposals))
        .route("/v1/mcp-proposals/{id}", post(decide_mcp_proposal))
}

/// The application of the workspace an agent session runs in.
async fn scoped_application(api: &Api, scope: &McpScope) -> ApiResult<Application> {
    let jobs = api
        .jobs
        .as_ref()
        .ok_or(ApiError(StatusCode::SERVICE_UNAVAILABLE, "Proposals require background jobs".into()))?;
    Ok(jobs.guidance_scope(scope).await?.application)
}

async fn pending_mcp_proposals(State(api): State<Api>) -> ApiResult<Json<Vec<McpProposal>>> { Ok(Json(api.mcp_proposals.pending(&super::mcp::LocalHost)?)) }

async fn decide_mcp_proposal(State(api): State<Api>, Path(id): Path<String>, Json(answer): Json<SkillProposalAnswer>) -> ApiResult<Json<McpProposal>> {
    if !answer.allow {
        return Ok(Json(api.mcp_proposals.reject(&id)?));
    }
    let (scope, conversation) = api.mcp_proposals.origin(&id)?;
    let application = scoped_application(&api, &scope).await?;
    let mut proposal = api.mcp_proposals.approve(&id, &application.plugins()?, &super::mcp::LocalHost).await?;
    proposal.message = continue_conversation(
        &api,
        &scope,
        conversation,
        &proposal.id,
        gpui_agent_application::mcp_proposal_continuation(&proposal),
    )
    .await;
    Ok(Json(proposal))
}

/// Guidance for the workspace an agent session runs in.
pub(super) async fn scoped_guidance(api: &Api, scope: &McpScope) -> ApiResult<gpui_agent_application::Guidance> {
    Ok(scoped_application(api, scope).await?.guidance()?)
}

async fn pending_proposals(State(api): State<Api>) -> ApiResult<Json<Vec<SkillProposal>>> { Ok(Json(api.skill_proposals.pending()?)) }

async fn decide_proposal(State(api): State<Api>, Path(id): Path<String>, Json(answer): Json<SkillProposalAnswer>) -> ApiResult<Json<SkillProposal>> {
    if !answer.allow {
        return Ok(Json(api.skill_proposals.reject(&id)?));
    }
    let (scope, conversation) = api.skill_proposals.origin(&id)?;
    let guidance = scoped_guidance(&api, &scope).await?;
    let mut proposal = api.skill_proposals.approve(&id, &guidance).await?;
    proposal.message = continue_conversation(
        &api,
        &scope,
        conversation,
        &proposal.id,
        gpui_agent_application::skill_proposal_continuation(&proposal),
    )
    .await;
    Ok(Json(proposal))
}

/// Applying a proposal only takes effect on a new turn, so the proposing
/// conversation continues in one. A failure to queue it is reported on the
/// response, never as a failed approval: the proposal has already been applied.
async fn continue_conversation(api: &Api, scope: &McpScope, conversation: Option<i64>, proposal_id: &str, text: String) -> Option<String> {
    let conversation = conversation?;
    let Some(jobs) = &api.jobs else {
        return Some("Applied, but background jobs are unavailable to continue the conversation".into());
    };
    jobs.continue_after_proposal(scope, conversation, proposal_id, text)
        .await
        .err()
        .map(|error| format!("Applied, but the conversation could not be continued: {}", error.1))
}

async fn files(Extension(app): Extension<Application>) -> ApiResult<Json<GuidanceFileCatalog>> { Ok(Json(app.guidance()?.files().await?)) }
async fn read_file(Extension(app): Extension<Application>, Json(request): Json<GuidanceFileRequest>) -> ApiResult<Json<GuidanceFile>> {
    Ok(Json(app.guidance()?.read_file(&request.id).await?))
}
async fn write_file(Extension(app): Extension<Application>, Json(request): Json<GuidanceWriteRequest>) -> ApiResult<Json<GuidanceFile>> {
    Ok(Json(app.guidance()?.write_file(request).await?))
}

async fn snapshot(Extension(app): Extension<Application>) -> ApiResult<Json<GuidanceSnapshot>> { Ok(Json(app.guidance()?.snapshot().await?)) }
async fn catalog(Extension(app): Extension<Application>) -> ApiResult<Json<SkillCatalog>> { Ok(Json(app.guidance()?.snapshot().await?.into())) }
async fn read(Extension(app): Extension<Application>, Json(request): Json<SkillReadRequest>) -> ApiResult<Json<SkillResource>> {
    Ok(Json(app.guidance()?.read_skill(&request.id, &request.resource).await?))
}
async fn set_enabled(Extension(app): Extension<Application>, Json(request): Json<SkillEnabledRequest>) -> ApiResult<Json<SkillCatalog>> {
    let guidance = app.guidance()?;
    guidance.set_skill_enabled(&request.id, request.enabled).await?;
    Ok(Json(guidance.snapshot().await?.into()))
}

async fn import_skill(Extension(app): Extension<Application>, Json(request): Json<SkillImportRequest>) -> ApiResult<Json<GuidanceFile>> {
    Ok(Json(app.guidance()?.import_skill(request).await?))
}
