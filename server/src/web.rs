//! axum router: /mcp (rmcp), /healthz, /status, embedded static landing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rust_embed::Embed;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::db::SharedIndex;
use crate::mcp::tools::{Caches, KubeDocs};
use crate::metrics::Metrics;
use crate::search;

pub const MAX_BODY_BYTES: usize = 1 << 20;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Embed)]
#[folder = "../web/dist"]
#[allow_missing = true]
struct Asset;

#[derive(Clone)]
pub struct AppState {
    pub idx: SharedIndex,
    pub caches: Arc<Caches>,
    pub metrics: Arc<Metrics>,
    pub static_dir: Option<PathBuf>,
}

pub fn router(state: AppState, allowed_hosts: Vec<String>, ct: CancellationToken) -> Router {
    let mcp_state = state.clone();
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_allowed_hosts(allowed_hosts)
        .with_max_request_body_bytes(MAX_BODY_BYTES)
        .with_cancellation_token(ct);
    let mcp = StreamableHttpService::new(
        move || {
            Ok(KubeDocs::new(
                mcp_state.idx.clone(),
                mcp_state.caches.clone(),
                mcp_state.metrics.clone(),
            ))
        },
        LocalSessionManager::default().into(),
        config,
    );
    Router::new()
        .route("/healthz", get(healthz))
        .route("/status", get(status))
        .nest_service("/mcp", mcp)
        .fallback(get(static_file))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
}

async fn healthz(State(s): State<AppState>) -> Response {
    if s.idx.load().is_some() {
        (StatusCode::OK, "ok").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "index not loaded").into_response()
    }
}

async fn status(State(s): State<AppState>) -> Response {
    let Some(idx) = s.idx.load_full() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"ready": false})),
        )
            .into_response();
    };
    let meta = idx.meta.clone();
    let size = idx.size_bytes;
    let projects = tokio::task::spawn_blocking(move || idx.with_conn(search::list_projects)).await;
    let projects = match projects {
        Ok(Ok(p)) => serde_json::to_value(p).unwrap_or(json!([])),
        _ => json!([]),
    };
    Json(json!({
        "ready": true,
        "server_version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": s.metrics.uptime_secs(),
        "index": { "size_bytes": size, "meta": meta },
        "projects": projects,
        "metrics": s.metrics.snapshot(),
    }))
    .into_response()
}

async fn static_file(State(s): State<AppState>, uri: Uri) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() || path.ends_with('/') {
        path.push_str("index.html");
    }
    if path.contains("..") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(dir) = &s.static_dir {
        if let Ok(bytes) = tokio::fs::read(dir.join(&path)).await {
            return file_response(&path, bytes);
        }
    }
    match Asset::get(&path) {
        Some(f) => file_response(&path, f.data.into_owned()),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn file_response(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.ends_with(".html") {
        "public, max-age=300"
    } else {
        "public, max-age=86400"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
