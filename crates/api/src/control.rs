use super::*;
use gpui_agent_application::ports::HostControl;

pub(super) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/control", get(status))
        .route("/v1/control/browser", post(browser).delete(stop_browser))
        .route("/v1/control/desktop", post(desktop).delete(disable_desktop))
        .route("/v1/control/desktop/grant", post(enable_desktop))
        .route("/v1/control/computer", get(computer_status))
        .route("/v1/control/computer/grant", post(computer_grant))
        .route("/v1/control/devices", get(device_list).post(device))
        .route("/v1/control/devices/grant", post(device_grant))
}
fn backend(api: &Api) -> ApiResult<&dyn HostControl> {
    api.controls
        .as_deref()
        .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Host controls are not available on this server".into()))
}
async fn computer_grant(State(api): State<Api>, Json(grant): Json<ComputerGrantRequest>) -> ApiResult<StatusCode> {
    let jobs = api
        .jobs
        .as_ref()
        .ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Background jobs are unavailable".into()))?;
    jobs.grant_computer(backend(&api)?, grant).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn computer_status(State(api): State<Api>) -> ApiResult<Json<ComputerStatus>> { Ok(Json(gpui_agent_application::computer_status(backend(&api)?).await?)) }
async fn device_list(State(api): State<Api>) -> ApiResult<Json<DeviceReply>> {
    Ok(Json(
        gpui_agent_application::control_device(backend(&api)?, None, DeviceCommand::List {}).await?,
    ))
}
async fn device(State(api): State<Api>, Json(command): Json<DeviceCommand>) -> ApiResult<Json<DeviceReply>> {
    Ok(Json(gpui_agent_application::control_device(backend(&api)?, None, command).await?))
}
async fn device_grant(State(api): State<Api>, Json(grant): Json<DeviceGrantRequest>) -> ApiResult<StatusCode> {
    gpui_agent_application::grant_device(backend(&api)?, grant).await?;
    Ok(StatusCode::NO_CONTENT)
}
fn failure(error: anyhow::Error) -> ApiError { ApiError(StatusCode::BAD_GATEWAY, error.to_string()) }
async fn status(State(api): State<Api>) -> ApiResult<Json<ControlStatus>> { Ok(Json(backend(&api)?.status().await.map_err(failure)?)) }
async fn browser(State(api): State<Api>, Json(command): Json<ControlCommand>) -> ApiResult<Json<Option<ControlFrame>>> {
    command.validate().map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.into()))?;
    Ok(Json(backend(&api)?.browser(command).await.map_err(failure)?))
}
async fn desktop(State(api): State<Api>, Json(command): Json<ControlCommand>) -> ApiResult<Json<Option<ControlFrame>>> {
    command.validate().map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.into()))?;
    Ok(Json(backend(&api)?.desktop(command).await.map_err(failure)?))
}
async fn stop_browser(State(api): State<Api>) -> ApiResult<StatusCode> {
    backend(&api)?.browser(ControlCommand::Stop {}).await.map_err(failure)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn enable_desktop(State(api): State<Api>) -> ApiResult<StatusCode> {
    backend(&api)?.enable_desktop(true).await.map_err(failure)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn disable_desktop(State(api): State<Api>) -> ApiResult<StatusCode> {
    backend(&api)?.enable_desktop(false).await.map_err(failure)?;
    Ok(StatusCode::NO_CONTENT)
}
