use super::*;
use axum::response::Html;

pub(super) fn public(path: &str) -> bool { matches!(path, "/" | "/app.js" | "/style.css" | "/manifest.webmanifest" | "/icon.svg" | "/sw.js") }
pub(super) fn routes() -> Router<Api> {
    Router::new()
        .route("/", get(|| async { Html(include_str!("../../../apps/web/index.html")) }))
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../../apps/web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async { ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("../../../apps/web/style.css")) }),
        )
        .route(
            "/manifest.webmanifest",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/manifest+json")],
                    include_str!("../../../apps/web/manifest.webmanifest"),
                )
            }),
        )
        .route(
            "/icon.svg",
            get(|| async { ([(header::CONTENT_TYPE, "image/svg+xml")], include_str!("../../../apps/web/icon.svg")) }),
        )
        .route(
            "/sw.js",
            get(|| async { ([(header::CONTENT_TYPE, "text/javascript")], include_str!("../../../apps/web/sw.js")) }),
        )
}

/// Browser fetches must carry an explicit bearer, our client header, and the
/// browser's same-origin fetch metadata. No cookies, URL credentials or CORS.
pub(super) fn same_origin(request: &Request) -> bool {
    if ["origin", "host", "x-knd-web", "sec-fetch-site"]
        .iter()
        .any(|name| request.headers().get_all(*name).iter().count() > 1)
    {
        return false;
    }
    if request.headers().get("x-knd-web").is_none_or(|v| v != "1") || request.headers().get("sec-fetch-site").is_none_or(|v| v != "same-origin") {
        return false;
    }
    let Some(origin) = request.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host) = request.headers().get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    origin == format!("https://{host}")
        || (origin == format!("http://{host}") && (host.starts_with("127.0.0.1:") || host.starts_with("localhost:") || host.starts_with("[::1]:")))
}
