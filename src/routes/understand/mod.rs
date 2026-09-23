//! 项目与源码理解 HTTP 路由门面 (`/api/understand/*`)
//! 按职责拆分为：
//! - [`comprehension`]：单文件与项目全局结构提取 + LLM/Claude 语义理解合成与磁盘缓存
//! - [`graph_queries`]：调用链追踪、死代码检测、全文/图谱搜索、RepoMap、ADR 与复杂度/克隆治理

pub mod comprehension;
pub mod graph_queries;

use crate::AppState;
use axum::{routing::get, Router};
use std::path::PathBuf;
use std::sync::Arc;

pub use comprehension::{
    understand_file, understand_project, UnderstandFileQuery, UnderstandProjectQuery,
};
pub use graph_queries::{
    create_adr_endpoint, list_or_get_adr, understand_classify_update, understand_complexity,
    understand_concepts, understand_deadcode, understand_diff, understand_freshness,
    understand_onboard, understand_projects, understand_repo_map, understand_search,
    understand_search_code, understand_similarity, understand_trace, CreateAdrPayload,
    RepoMapQuery, UnderstandAdrQuery, UnderstandClassifyUpdateQuery, UnderstandComplexityQuery,
    UnderstandConceptsQuery, UnderstandDeadcodeQuery, UnderstandDiffQuery,
    UnderstandFreshnessQuery, UnderstandOnboardQuery, UnderstandSearchCodeQuery,
    UnderstandSearchQuery, UnderstandSimilarityPayload, UnderstandTraceQuery,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/understand/file", get(understand_file))
        .route("/api/understand/project", get(understand_project))
        .route("/api/understand/onboard", get(understand_onboard))
        .route("/api/understand/diff", get(understand_diff))
        .route("/api/understand/trace", get(understand_trace))
        .route("/api/understand/deadcode", get(understand_deadcode))
        .route("/api/understand/search", get(understand_search))
        .route("/api/understand/search_code", get(understand_search_code))
        .route(
            "/api/understand/classify_update",
            get(understand_classify_update),
        )
        .route("/api/understand/concepts", get(understand_concepts))
        .route("/api/understand/freshness", get(understand_freshness))
        .route("/api/understand/complexity", get(understand_complexity))
        .route(
            "/api/understand/similarity",
            axum::routing::post(understand_similarity),
        )
        .route("/api/understand/projects", get(understand_projects))
        .route(
            "/api/understand/adr",
            get(list_or_get_adr).post(create_adr_endpoint),
        )
        .route("/api/understand/repo_map", get(understand_repo_map))
}

pub(crate) fn resolve_project_path(state: &AppState, project: &str) -> PathBuf {
    let p = std::path::Path::new(project);
    if p.is_absolute() && p.exists() {
        p.to_path_buf()
    } else {
        let joined = state.runtime_config.workspace_root.join(project);
        if joined.exists() {
            joined
        } else {
            p.to_path_buf()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_understand_router_construction() {
        let state = crate::server::test_state();
        let app: Router = router().with_state(state);
        assert!(format!("{:?}", app).contains("Router"));
    }
}
