use super::*;
use axum::http::HeaderMap;
fn jobs(api: &Api) -> ApiResult<&gpui_agent_application::Jobs> {
    api.jobs
        .as_ref()
        .ok_or(ApiError(StatusCode::SERVICE_UNAVAILABLE, "Background jobs are unavailable in this host".into()))
}
pub(super) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/providers/{provider}/accounts", get(accounts).post(create_account))
        .route("/v1/providers/{provider}/jobs", post(enqueue))
        .route("/v1/jobs", get(list))
        .route("/v1/jobs/{id}", get(detail).delete(cancel))
        .route("/v1/jobs/{id}/events", get(events))
        .route("/v1/jobs/{id}/interactions/{request_id}", post(respond))
}
pub(super) fn selection(headers: &HeaderMap) -> ApiResult<(String, Option<i64>)> {
    let account = headers
        .get("x-knd-account")
        .map(|v| v.to_str())
        .transpose()
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Invalid account profile ID".into()))?
        .unwrap_or("default")
        .to_owned();
    validate_account_id(&account).map_err(|message| ApiError(StatusCode::BAD_REQUEST, message.into()))?;
    let workspace = headers
        .get("x-knd-workspace")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|id| *id > 0)
                .ok_or(ApiError(StatusCode::BAD_REQUEST, "Invalid workspace ID".into()))
        })
        .transpose()?;
    Ok((account, workspace))
}
async fn accounts(State(api): State<Api>, Path(provider): Path<Provider>) -> ApiResult<Json<Vec<AccountProfile>>> {
    Ok(Json(jobs(&api)?.accounts(provider).await?))
}
async fn create_account(State(api): State<Api>, Path(provider): Path<Provider>, Json(request): Json<CreateAccount>) -> ApiResult<Json<AccountProfile>> {
    Ok(Json(jobs(&api)?.create_account(provider, request.label).await?))
}
async fn enqueue(
    State(api): State<Api>,
    Path(provider): Path<Provider>,
    headers: HeaderMap,
    Json(request): Json<ChatRequest>,
) -> ApiResult<(StatusCode, Json<Job>)> {
    let (account, workspace) = selection(&headers)?;
    let job = if let Some(key) = headers.get("idempotency-key") {
        if headers.get_all("idempotency-key").iter().count() != 1 {
            return Err(ApiError(StatusCode::BAD_REQUEST, "One Idempotency-Key is required".into()));
        }
        let key = key
            .to_str()
            .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Invalid Idempotency-Key".into()))?
            .to_owned();
        jobs(&api)?.enqueue_once(provider, account, workspace, request, key).await?
    } else {
        jobs(&api)?.enqueue(provider, account, workspace, request).await?
    };
    Ok((StatusCode::ACCEPTED, Json(job)))
}
async fn list(State(api): State<Api>) -> ApiResult<Json<Vec<Job>>> { Ok(Json(jobs(&api)?.list().await?)) }
async fn detail(State(api): State<Api>, Path(id): Path<i64>) -> ApiResult<Json<JobDetail>> { Ok(Json(jobs(&api)?.get(id).await?)) }
async fn cancel(State(api): State<Api>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    jobs(&api)?.cancel(id).await?;
    Ok(StatusCode::ACCEPTED)
}
async fn respond(State(api): State<Api>, Path((id, request_id)): Path<(i64, u64)>, Json(response): Json<InteractionResponse>) -> ApiResult<StatusCode> {
    jobs(&api)?.respond(id, request_id, response)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Default, serde::Deserialize)]
struct EventQuery {
    #[serde(default)]
    after: i64,
}
async fn events(State(api): State<Api>, Path(id): Path<i64>, Query(query): Query<EventQuery>) -> ApiResult<Response> {
    if query.after < 0 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Event cursor must be nonnegative".into()));
    }
    let jobs = jobs(&api)?.clone();
    jobs.job(id).await?;
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move {
        let mut cursor = query.after;
        loop {
            if tx.is_closed() {
                break;
            }
            let Ok(job) = jobs.job(id).await else {
                break;
            };
            let Ok(events) = jobs.events(id, cursor).await else {
                break;
            };
            if events.is_empty() {
                if job.state.terminal() || jobs.is_stopping() {
                    break;
                }
                tokio::select! { _=tx.closed()=>break, _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{} }
            } else {
                for event in events {
                    cursor = event.sequence;
                    if tx.send(event).await.is_err() {
                        return;
                    }
                }
            }
        }
    });
    let stream = ReceiverStream::new(rx).map(|event| {
        let mut data = serde_json::to_vec(&event).expect("serializable job event");
        data.push(b'\n');
        Ok::<_, Infallible>(Bytes::from(data))
    });
    Ok((
        [(header::CONTENT_TYPE, "application/x-ndjson"), (header::CACHE_CONTROL, "no-store")],
        Body::from_stream(stream),
    )
        .into_response())
}
