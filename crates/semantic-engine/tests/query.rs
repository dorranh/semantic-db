use semantic_catalog::RelationKind;
use semantic_engine::{Engine, pretty_format_batches};

async fn fixture() -> Engine {
    let mut engine = Engine::new();
    engine
        .register_csv(
            "wells",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/geospatial/wells.csv"
            ),
        )
        .await
        .unwrap();
    engine
}

#[tokio::test]
async fn queries_csv_and_composes_views_with_lineage() {
    let mut engine = fixture().await;
    engine
        .create_view(
            "deep_wells",
            "SELECT well_id, basin FROM wells WHERE total_depth_m >= 2500",
        )
        .await
        .unwrap();
    engine
        .create_view(
            "northern_deep_wells",
            "SELECT well_id FROM deep_wells WHERE basin = 'North Basin'",
        )
        .await
        .unwrap();
    let batches = engine
        .query("SELECT well_id FROM northern_deep_wells ORDER BY well_id")
        .await
        .unwrap();
    assert_eq!(
        pretty_format_batches(&batches).unwrap().to_string(),
        "+---------+\n| well_id |\n+---------+\n| W-001   |\n| W-004   |\n+---------+"
    );
    let relation = engine.catalog().relation("northern_deep_wells").unwrap();
    assert_eq!(relation.schema.fields().len(), 1);
    let RelationKind::View { dependencies, .. } = &relation.kind else {
        panic!("expected view")
    };
    assert_eq!(dependencies, &["deep_wells"]);
}

#[tokio::test]
async fn rejects_invalid_queries_and_preserves_catalog_on_failure() {
    let mut engine = fixture().await;
    assert!(
        engine
            .plan_sql("SELECT missing_column FROM wells")
            .await
            .is_err()
    );
    assert!(engine.query("DROP TABLE wells").await.is_err());
    assert!(
        engine
            .create_view("broken", "SELECT missing_column FROM wells")
            .await
            .is_err()
    );
    assert!(engine.catalog().relation("broken").is_none());
    assert!(engine.create_view("wells", "SELECT 1").await.is_err());
    assert!(
        engine
            .create_view("Invalid.Name", "SELECT 1")
            .await
            .is_err()
    );
    let batches = engine.query("SELECT * FROM wells").await.unwrap();
    assert_eq!(batches.iter().map(|b| b.num_rows()).sum::<usize>(), 5);
    assert_eq!(engine.catalog().relations().count(), 1);
}

#[tokio::test]
async fn records_dependencies_inside_subqueries() {
    let mut engine = fixture().await;
    engine
        .create_view("well_count", "SELECT (SELECT count(*) FROM wells) AS total")
        .await
        .unwrap();
    let RelationKind::View { dependencies, .. } =
        &engine.catalog().relation("well_count").unwrap().kind
    else {
        panic!("expected view")
    };
    assert_eq!(dependencies, &["wells"]);
}

#[tokio::test]
async fn excludes_cte_names_from_lineage_and_rejects_non_query_views() {
    let mut engine = fixture().await;
    engine.create_view("active_wells", "WITH active AS (SELECT * FROM wells WHERE status = 'active') SELECT well_id FROM active").await.unwrap();
    let RelationKind::View { dependencies, .. } =
        &engine.catalog().relation("active_wells").unwrap().kind
    else {
        panic!("expected view")
    };
    assert_eq!(dependencies, &["wells"]);
    assert!(
        engine
            .create_view("explanation", "EXPLAIN SELECT * FROM wells")
            .await
            .is_err()
    );
    assert!(
        engine
            .create_view("mutation", "DROP TABLE wells")
            .await
            .is_err()
    );
    assert_eq!(engine.catalog().relations().count(), 2);
}
