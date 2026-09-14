//! OpenAPI 3.1 specification & interactive API documentation routes.
//!
//! Exposes:
//! - `GET /api/openapi.json`: The machine-readable OpenAPI 3.1 specification.
//! - `GET /api/docs`: An interactive, modern API documentation page.

use std::sync::Arc;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::Value;

use crate::AppState;

/// Canonical OpenAPI 3.1 raw specification string embedded from docs/api/openapi.json.
pub const OPENAPI_SPEC_RAW: &str = include_str!("../../docs/api/openapi.json");

/// Router for OpenAPI specification and interactive documentation.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/openapi.json", get(get_openapi_json))
        .route("/api/docs", get(get_docs_html))
}

/// Returns the OpenAPI 3.1 JSON specification.
pub async fn get_openapi_json() -> Response {
    match serde_json::from_str::<Value>(OPENAPI_SPEC_RAW) {
        Ok(json) => Json(json).into_response(),
        Err(err) => {
            tracing::error!("Failed to parse embedded openapi.json: {}", err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to parse openapi.json: {}", err),
            )
                .into_response()
        }
    }
}

/// Returns an interactive modern API documentation page (using Scalar API Reference).
pub async fn get_docs_html() -> Html<String> {
    Html(render_docs_page())
}

fn render_docs_page() -> String {
    r#"<!doctype html>
<html lang="zh-CN">
  <head>
    <title>SuperAgent Hub-RS API Documentation</title>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <link rel="icon" href="data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 100 100%22><text y=%22.9em%22 font-size=%2290%22>⚡</text></svg>">
    <style>
      body {
        margin: 0;
        background-color: #0d1117;
        color: #c9d1d9;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
      }
      .fallback {
        padding: 40px;
        max-width: 800px;
        margin: 0 auto;
      }
      .card {
        background: #161b22;
        border: 1px solid #30363d;
        border-radius: 12px;
        padding: 24px;
        margin-top: 20px;
      }
      a {
        color: #58a6ff;
        text-decoration: none;
      }
      a:hover {
        text-decoration: underline;
      }
      code {
        background: rgba(110, 118, 129, 0.2);
        padding: 2px 6px;
        border-radius: 6px;
        font-family: monospace;
      }
    </style>
  </head>
  <body>
    <div id="scalar-container">
      <script
        id="api-reference"
        data-url="/api/openapi.json"
        data-configuration='{
          "theme": "purple",
          "layout": "modern",
          "darkMode": true,
          "showSidebar": true
        }'
      ></script>
      <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
    </div>
    <noscript>
      <div class="fallback">
        <h1>SuperAgent Hub-RS SDK API</h1>
        <div class="card">
          <p>当前环境未启用 JavaScript。请直接下载或查阅原始 OpenAPI 规范文件：</p>
          <p>👉 <a href="/api/openapi.json"><code>/api/openapi.json</code> (OpenAPI 3.1 Spec)</a></p>
        </div>
      </div>
    </noscript>
  </body>
</html>"#
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_openapi_spec_is_valid_json() {
        let parsed: Result<Value, _> = serde_json::from_str(OPENAPI_SPEC_RAW);
        assert!(
            parsed.is_ok(),
            "docs/api/openapi.json must be valid JSON: {:?}",
            parsed.err()
        );

        let json = parsed.unwrap();
        assert_eq!(json["openapi"], "3.1.0");
        assert!(json["info"]["title"].is_string());

        let paths = json["paths"].as_object().expect("paths must be an object");
        assert!(paths.contains_key("/v1/chat/completions"));
        assert!(paths.contains_key("/v1/models"));
        assert!(paths.contains_key("/api/agents/run"));
        assert!(paths.contains_key("/api/agents/tasks/{task_id}/events"));
        assert!(paths.contains_key("/api/agents/tasks/{task_id}/diff"));
        assert!(paths.contains_key("/api/workspaces/open"));
        assert!(paths.contains_key("/api/health"));
        assert!(paths.contains_key("/api/openapi.json"));
    }

    #[test]
    fn test_render_docs_html_contains_scalar_reference() {
        let html = render_docs_page();
        assert!(html.contains("/api/openapi.json"));
        assert!(html.contains("scalar/api-reference"));
    }
}
