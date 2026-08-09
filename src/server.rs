//! HTTP server bootstrap for the AIndex Hub web backend.
//!
//! Holds the shared [`AppState`] and [`serve`], which builds the axum router
//! and listens on the requested port. This is the *web backend* role of the
//! `hub` binary; command dispatch (server vs rtk hook) lives in
//! [`crate::cli`], keeping this module focused on serving HTTP.

use std::sync::Arc;

use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::app_paths::AppPaths;
use crate::config::runtime::RuntimeConfig;
use crate::services::task_manager::TaskManager;

use std::sync::atomic::AtomicUsize;

/// Shared application state handed to every route via `axum::extract::State`.
pub struct AppState {
    pub runtime_config: RuntimeConfig,
    pub app_paths: AppPaths,
    pub task_manager: TaskManager,
    pub http_client: reqwest::Client,
    pub request_counter: AtomicUsize,
}

/// Default listen port when `hub service` is invoked without an explicit port.
pub const DEFAULT_PORT: u16 = 3001;

/// Boot the web backend and serve until shutdown. Runs on the ambient Tokio
/// runtime (see [`crate::cli`], which drives this from `#[tokio::main]`).
pub async fn serve(port: u16) {
    init_tracing();

    // Load .env if present
    let _ = dotenvy::dotenv();

    let state = build_state();

    let app = build_router(state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("AIndex Hub backend listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    axum::serve(listener, app).await.unwrap();
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hub_rs=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

/// Initialise application state: paths, persisted settings, runtime config,
/// and the background task manager.
fn build_state() -> Arc<AppState> {
    let app_paths = AppPaths::new();
    app_paths.migrate_legacy_data();
    let llm_settings_path = crate::config::llm_settings::ensure_persisted();
    let claude_settings_path = crate::config::claude_settings::ensure_persisted();
    tracing::info!("App data root: {}", app_paths.app_data_root.display());
    tracing::info!("LLM settings file: {}", llm_settings_path.display());
    tracing::info!("Claude settings file: {}", claude_settings_path.display());

    let runtime_config = RuntimeConfig::from_env(&app_paths);
    let task_manager = TaskManager::new(&app_paths);

    Arc::new(AppState {
        runtime_config,
        app_paths,
        task_manager,
        http_client: reqwest::Client::new(),
        request_counter: std::sync::atomic::AtomicUsize::new(0),
    })
}

/// Compose every feature router plus shared middleware.
fn build_router(state: Arc<AppState>) -> Router {
    use crate::routes;

    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(
            |origin: &axum::http::HeaderValue, _request_parts: &axum::http::request::Parts| {
                if let Ok(origin_str) = origin.to_str() {
                    origin_str == "http://localhost" || origin_str.starts_with("http://localhost:") ||
                    origin_str == "http://127.0.0.1" || origin_str.starts_with("http://127.0.0.1:")
                } else {
                    false
                }
            },
        ))
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .merge(routes::health::router())
        .merge(routes::settings::router())
        .merge(routes::claude::router())
        .merge(routes::files::router())
        .merge(routes::git::router())
        .merge(routes::projects::router())
        .merge(routes::tasks::router())
        .merge(routes::memory::router())
        .merge(routes::reverse::router())
        .merge(routes::qa::router())
        .merge(routes::context::router())
        .merge(routes::understand::router())
        .merge(routes::compress::router())
        .merge(routes::agents::router())
        .merge(routes::analyze::router())
        .merge(routes::proxy::router())
        .merge(routes::skills::router())
        .merge(routes::mcp::router())
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_check() {
        let state = build_state();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
