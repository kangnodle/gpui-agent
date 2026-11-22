use super::*;

pub(super) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/environment", get(read))
        .route("/v1/environment/preferences", axum::routing::patch(update))
}
async fn read(State(api): State<Api>) -> ApiResult<Json<EnvironmentDescriptor>> { Ok(Json(workspaces(&api)?.environment_read().await?)) }
async fn update(State(api): State<Api>, Json(patch): Json<EnvironmentPreferencesPatch>) -> ApiResult<Json<EnvironmentPreferences>> {
    Ok(Json(workspaces(&api)?.environment_preferences_update(None, patch).await?))
}
