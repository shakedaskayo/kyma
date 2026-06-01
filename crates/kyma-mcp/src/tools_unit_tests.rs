use crate::tools::ToolDispatch;
use kyma_server::agent::SharedToolCtx;
use kyma_server::test_support::seeded_state_empty;

#[tokio::test]
async fn list_returns_all_named_tools() {
    let state = seeded_state_empty().await;
    let pool = sqlx::PgPool::connect(
        &std::env::var("KYMA_TEST_DATABASE_URL").expect("KYMA_TEST_DATABASE_URL"),
    )
    .await
    .unwrap();
    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool,
    };
    let dispatch = ToolDispatch::new(shared);
    let listed = dispatch.list();
    let names: Vec<_> = listed.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 13);
    for expected in [
        "list_databases", "describe_table", "run_sql", "run_kql",
        "sample_rows", "explore_schema", "find_references_to", "graph_traverse",
        "memory_search", "recall_memory", "save_memory", "list_memories",
        "link_memory_to_entity",
    ] {
        assert!(names.contains(&expected), "missing tool: {expected}");
    }
}

#[tokio::test]
async fn list_entries_have_inputschema_objects() {
    let state = seeded_state_empty().await;
    let pool = sqlx::PgPool::connect(
        &std::env::var("KYMA_TEST_DATABASE_URL").expect("KYMA_TEST_DATABASE_URL"),
    )
    .await
    .unwrap();
    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool,
    };
    let dispatch = ToolDispatch::new(shared);
    for tool in dispatch.list() {
        assert!(tool.get("inputSchema").is_some());
        assert!(tool["description"].as_str().unwrap().len() > 10);
    }
}
