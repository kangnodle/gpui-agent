use super::*;

pub(super) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/plugins", get(catalog))
        .route("/v1/plugins/import", post(import))
        .route("/v1/plugins/claude", post(claude))
        .route("/v1/plugins/enabled", axum::routing::put(enabled))
        .route("/v1/plugins/remove", post(remove))
}
async fn catalog(Extension(app): Extension<Application>) -> ApiResult<Json<PluginCatalog>> { Ok(Json(app.plugins()?.catalog().await?)) }
async fn import(Extension(app): Extension<Application>, Json(request): Json<PluginImportRequest>) -> ApiResult<Json<PluginCatalog>> {
    Ok(Json(app.plugins()?.import(request).await?))
}
async fn claude(Extension(app): Extension<Application>, Json(request): Json<PluginClaudeSyncRequest>) -> ApiResult<Json<PluginSyncResult>> {
    Ok(Json(app.plugins()?.sync_claude(&request.scope, request.enable).await?))
}
async fn enabled(Extension(app): Extension<Application>, Json(request): Json<PluginEnabledRequest>) -> ApiResult<Json<PluginCatalog>> {
    Ok(Json(app.plugins()?.set_enabled(&request.id, request.enabled).await?))
}
async fn remove(Extension(app): Extension<Application>, Json(request): Json<PluginRemoveRequest>) -> ApiResult<Json<PluginCatalog>> {
    Ok(Json(app.plugins()?.remove(&request.id).await?))
}
