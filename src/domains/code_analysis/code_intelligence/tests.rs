use super::*;
use rusqlite::Connection;

#[test]
fn test_code_intelligence_extended_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("graph.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE nodes (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            qualified_name TEXT NOT NULL,
            label TEXT NOT NULL,
            file_path TEXT NOT NULL,
            start_line INTEGER,
            end_line INTEGER,
            properties TEXT
        );
        CREATE TABLE edges (
            id INTEGER PRIMARY KEY,
            source_id INTEGER NOT NULL,
            target_id INTEGER NOT NULL,
            type TEXT NOT NULL,
            properties TEXT
        );
        INSERT INTO nodes VALUES (1, 'handle_request', 'api::handle_request', 'Function', 'src/api.rs', 10, 20, '{}');
        INSERT INTO nodes VALUES (2, 'process_order', 'services::process_order', 'Function', 'src/service.rs', 15, 30, '{}');
        INSERT INTO nodes VALUES (3, 'save_db', 'dao::save_db', 'Function', 'src/dao.rs', 5, 12, '{}');
        INSERT INTO nodes VALUES (4, 'unused_helper', 'utils::unused_helper', 'Function', 'src/utils.rs', 1, 5, '{}');

        INSERT INTO edges VALUES (1, 1, 2, 'CALLS', '{}');
        INSERT INTO edges VALUES (2, 2, 3, 'CALLS', '{}');
        "#,
    )
    .unwrap();
    drop(conn);

    let rt = CodeIntelligenceRuntime::new(&db_path);

    // 1. Trace Path
    let trace = rt.trace_path("handle_request", "outbound", 3, "calls").unwrap();
    assert_eq!(trace.total_hops, 2);
    assert_eq!(trace.reachable_nodes.len(), 2);

    // 2. Schema
    let schema = rt.get_graph_schema().unwrap();
    assert_eq!(schema.total_nodes, 4);
    assert_eq!(schema.total_edges, 2);
    assert_eq!(schema.node_labels.get("Function"), Some(&4));
    assert_eq!(schema.edge_types.get("CALLS"), Some(&2));

    // 3. Dead code
    let dead = rt.get_dead_code(10).unwrap();
    let dead_names: Vec<&str> = dead.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(dead.len(), 2);
    assert!(dead_names.contains(&"unused_helper"));
    assert!(dead_names.contains(&"handle_request"));

    // 4. Search Graph with min_degree
    let search_res = rt.search_graph(Some("Function"), Some("process"), None, Some(1), None, 10, 0).unwrap();
    assert_eq!(search_res.len(), 1);
    assert_eq!(search_res[0].name, "process_order");
    assert_eq!(search_res[0].total_degree, 2); // 1 in (from handle_request) + 1 out (to save_db)

    // 5. Search Code fallback
    let code_res = rt.search_code("order", None, None, 10).unwrap();
    assert_eq!(code_res.len(), 1);
    assert_eq!(code_res[0].file_path, "src/service.rs");
}
