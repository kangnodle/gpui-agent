use super::*;
use axum::http::HeaderMap;
fn jobs(api: &Api) -> ApiResult<&gpui_agent_application::Jobs> {
    api.jobs
        .as_ref()
        .ok_or(ApiError(StatusCode::SERVICE_UNAVAILABLE, "MCP settings require background jobs".into()))
}
pub(super) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/mcp", get(common_mcp_get).put(common_mcp_put))
        .route("/v1/mcp/{provider}", get(mcp_get).put(mcp_put))
        .route("/v1/mcp/{provider}/{name}/oauth", get(mcp_oauth_status).post(mcp_oauth))
}
async fn mcp_get(State(api): State<Api>, Path(provider): Path<Provider>, headers: HeaderMap) -> ApiResult<Json<Vec<McpServer>>> {
    let (account, workspace) = jobs::selection(&headers)?;
    Ok(Json(jobs(&api)?.mcp(provider, account, workspace, None).await?))
}
async fn common_mcp_get(State(api): State<Api>, headers: HeaderMap) -> ApiResult<Json<Vec<McpServer>>> {
    let (_, workspace) = jobs::selection(&headers)?;
    Ok(Json(jobs(&api)?.common_mcp(workspace, None).await?))
}
async fn common_mcp_put(State(api): State<Api>, headers: HeaderMap, Json(servers): Json<Vec<McpServer>>) -> ApiResult<Json<Vec<McpServer>>> {
    let (_, workspace) = jobs::selection(&headers)?;
    Ok(Json(jobs(&api)?.common_mcp(workspace, Some(servers)).await?))
}
async fn mcp_oauth_status(State(api): State<Api>, Path((provider, name)): Path<(Provider, String)>, headers: HeaderMap) -> ApiResult<Json<McpOAuthStatus>> {
    let (account, workspace) = jobs::selection(&headers)?;
    Ok(Json(jobs(&api)?.mcp_oauth(provider, account, workspace, name, McpOAuthCommand::Status).await?))
}
async fn mcp_oauth(
    State(api): State<Api>,
    Path((provider, name)): Path<(Provider, String)>,
    headers: HeaderMap,
    Json(command): Json<McpOAuthCommand>,
) -> ApiResult<Json<McpOAuthStatus>> {
    let (account, workspace) = jobs::selection(&headers)?;
    Ok(Json(jobs(&api)?.mcp_oauth(provider, account, workspace, name, command).await?))
}
async fn mcp_put(
    State(api): State<Api>,
    Path(provider): Path<Provider>,
    headers: HeaderMap,
    Json(servers): Json<Vec<McpServer>>,
) -> ApiResult<Json<Vec<McpServer>>> {
    let (account, workspace) = jobs::selection(&headers)?;
    Ok(Json(jobs(&api)?.mcp(provider, account, workspace, Some(servers)).await?))
}
