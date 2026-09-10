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
use crate::services::brain::capabilities::certification::CertificationGatekeeper;
use crate::services::brain::evolution_memory::EvolutionMemory;
use crate::services::brain::tool_approval_store::ToolApprovalStore;
use crate::services::task_manager::TaskManager;
use crate::services::visual_pipeline::VisualPipelineRuntime;
use crate::services::workspace_capability::WorkspaceCapabilityRegistry;

use std::sync::atomic::AtomicUsize;

/// Shared application state handed to every route via `axum::extract::State`.
pub struct AppState {
    pub runtime_config: RuntimeConfig,
    pub app_paths: AppPaths,
    pub task_manager: Arc<TaskManager>,
    pub http_client: reqwest::Client,
    pub request_counter: AtomicUsize,
    pub certification: Arc<CertificationGatekeeper>,
    pub approval_store: Arc<ToolApprovalStore>,
    pub evolution_memory: Arc<EvolutionMemory>,
    pub workspace_capabilities: Arc<WorkspaceCapabilityRegistry>,
    pub visual_pipeline: Arc<VisualPipelineRuntime>,
}

/// Default listen port when `hub service` is invoked without an explicit port.
pub const DEFAULT_PORT: u16 = 3001;

/// Boot the web backend and serve until shutdown. Runs on the ambient Tokio
/// runtime (see [`crate::cli`], which drives this from `#[tokio::main]`).
pub async fn serve(port: u16) {
    init_tracing();

    crate::services::usage_tracker::init_db();
    crate::services::agent::registry_db::init_db();

    // Load .env if present
    let _ = dotenvy::dotenv();

    let state = build_state();

    let app = build_router(state.clone());

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("AIndex Hub backend listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(state.clone()))
        .await
        .unwrap();
    tracing::info!("AIndex Hub backend shutdown complete");
}

async fn shutdown_signal(state: Arc<AppState>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C / SIGINT signal, initiating graceful shutdown...");
        },
        _ = terminate => {
            tracing::info!("Received SIGTERM signal, initiating graceful shutdown...");
        },
    }

    tracing::info!("Draining in-flight tasks and operations...");
    if let Err(e) = state.task_manager.drain_active_tasks("server shutdown", std::time::Duration::from_secs(10)).await {
        tracing::error!("Failed to drain tasks cleanly: {}", e);
    } else {
        tracing::info!("In-flight tasks drained successfully.");
    }
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
    let task_manager = Arc::new(TaskManager::new(&app_paths));
    let approval_store = Arc::new(ToolApprovalStore::new(
        app_paths.data_path(&["tool-approvals"]),
    ));
    let evolution_memory = Arc::new(EvolutionMemory::new(
        app_paths.data_path(&["evolution", "strategies.json"]),
    ));
    let workspace_capabilities = Arc::new(WorkspaceCapabilityRegistry::new(
        &runtime_config.projects_root,
    ));
    let http_client = reqwest::Client::builder()
        .http1_only()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let visual_pipeline = Arc::new(VisualPipelineRuntime::new(&app_paths, http_client.clone()));

    Arc::new(AppState {
        runtime_config,
        app_paths,
        task_manager,
        http_client,
        request_counter: std::sync::atomic::AtomicUsize::new(0),
        certification: Arc::new(CertificationGatekeeper::with_builtins()),
        approval_store,
        evolution_memory,
        workspace_capabilities,
        visual_pipeline,
    })
}

#[cfg(test)]
pub(crate) fn test_state() -> Arc<AppState> {
    build_state()
}

/// Compose every feature router plus shared middleware.
fn build_router(state: Arc<AppState>) -> Router {
    use crate::routes;

    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(
            |origin: &axum::http::HeaderValue, _request_parts: &axum::http::request::Parts| {
                if let Ok(origin_str) = origin.to_str() {
                    origin_str == "http://localhost"
                        || origin_str.starts_with("http://localhost:")
                        || origin_str == "http://127.0.0.1"
                        || origin_str.starts_with("http://127.0.0.1:")
                        || origin_str == "app://-"
                        || origin_str.starts_with("app://")
                } else {
                    false
                }
            },
        ))
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .merge(routes::health::router())
        .merge(routes::capabilities::router())
        .merge(routes::tool_approvals::router())
        .merge(routes::evolution::router())
        .merge(routes::settings::router())
        .merge(routes::claude::router())
        .merge(routes::files::router())
        .merge(routes::mcp_skills::router())
        .merge(routes::git::router())
        .merge(routes::projects::router())
        .merge(routes::tasks::router())
        .merge(routes::memory::router())
        .merge(routes::reverse::router())
        .merge(routes::qa::router())
        .merge(routes::context::router())
        .merge(routes::understand::router())
        .merge(routes::vision::router())
        .merge(routes::workspaces::router())
        .merge(routes::compress::router())
        .merge(routes::agents::router())
        .merge(routes::analyze::router())
        .merge(routes::proxy::router())
        .merge(routes::skills::router())
        .merge(routes::mcp::router())
        .merge(routes::stats::router())
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
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(body.pointer("/providerHealth").is_some());
        assert!(body.pointer("/gatewayEval/profiles").is_some());
    }

    #[tokio::test]
    async fn test_capabilities_discovery() {
        let state = build_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/capabilities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_evolution_strategy_discovery() {
        let state = build_state();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/evolution/strategies")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
