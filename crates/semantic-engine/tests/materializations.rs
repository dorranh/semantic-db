use datafusion::{
    arrow::{
        array::{Int64Array, RecordBatch},
        datatypes::{DataType, Field, Schema},
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_engine::{
    CacheOptions, Engine, MaterializationPolicy, QueryOptions, SourceDescriptor,
};
use std::sync::Arc;
fn provider(value: i64) -> Arc<MemTable> {
    let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int64, false)]));
    Arc::new(
        MemTable::try_new(
            schema.clone(),
            vec![vec![
                RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![value]))])
                    .unwrap(),
            ]],
        )
        .unwrap(),
    )
}
async fn engine(path: &std::path::Path, value: i64, view: &str) -> Engine {
    use datafusion::catalog::TableProvider;
    let mut engine = Engine::new();
    let provider = provider(value);
    engine
        .register_table(
            Relation::base("source", provider.schema(), "test-source"),
            provider,
        )
        .unwrap();
    engine
        .set_source_descriptor(
            "source",
            SourceDescriptor {
                scope: "source".into(),
                authorization_scope: "a".into(),
                schema_revision: "1".into(),
                revision: "1".into(),
            },
        )
        .unwrap();
    engine
        .configure_cache(CacheOptions {
            directory: path.to_owned(),
            max_memory_bytes: 65536,
            max_disk_bytes: 1024 * 1024,
        })
        .unwrap();
    engine.create_view("summary", view).await.unwrap();
    for name in ["source", "summary"] {
        engine
            .materialize(
                name,
                MaterializationPolicy {
                    max_age_seconds: 60,
                    max_fill_bytes: 65536,
                },
            )
            .unwrap();
    }
    engine
}
async fn value(engine: &Engine, options: QueryOptions) -> i64 {
    let batches = engine
        .execute("SELECT x FROM summary", options)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}
#[tokio::test]
async fn source_and_view_generations_are_resolved_before_planning() {
    let path = std::env::temp_dir().join(format!(
        "semantic-engine-cache-{}",
        semantic_runtime::unique_id()
    ));
    let engine1 = engine(&path, 7, "SELECT x FROM source").await;
    engine1.prepare("SELECT x FROM summary").await.unwrap();
    engine1
        .query("EXPLAIN SELECT x FROM summary")
        .await
        .unwrap();
    assert!(
        engine1
            .materialization_manager()
            .unwrap()
            .status()
            .unwrap()
            .is_empty()
    );
    assert_eq!(value(&engine1, QueryOptions::default()).await, 7);
    assert_eq!(
        engine1
            .materialization_manager()
            .unwrap()
            .status()
            .unwrap()
            .len(),
        2
    );
    let engine2 = engine(&path, 9, "SELECT x FROM source").await;
    assert_eq!(value(&engine2, QueryOptions::default()).await, 7);
    assert_eq!(
        value(
            &engine2,
            QueryOptions {
                bypass_materialization: true,
                ..Default::default()
            }
        )
        .await,
        9
    );
    engine2.refresh_materialization("source").await.unwrap();
    assert_eq!(value(&engine2, QueryOptions::default()).await, 9);
    let engine3 = engine(&path, 10, "SELECT x + 1 AS x FROM source").await;
    assert_eq!(value(&engine3, QueryOptions::default()).await, 10);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn stricter_view_freshness_refreshes_cached_ancestors() {
    let path = std::env::temp_dir().join(format!(
        "semantic-freshness-{}",
        semantic_runtime::unique_id()
    ));
    let mut first = engine(&path, 7, "SELECT x FROM source").await;
    first
        .materialize(
            "source",
            MaterializationPolicy {
                max_age_seconds: 300,
                max_fill_bytes: 65536,
            },
        )
        .unwrap();
    first.query("SELECT x FROM source").await.unwrap();
    let manifest = first
        .materialization_manager()
        .unwrap()
        .status()
        .unwrap()
        .remove(0);
    let file = path
        .join(&manifest.key)
        .join(&manifest.generation)
        .join("manifest.json");
    let text = std::fs::read_to_string(&file).unwrap().replace(
        &format!("\"acquired_at_ms\":{}", manifest.acquired_at_ms),
        &format!(
            "\"acquired_at_ms\":{}",
            semantic_materialization::now_ms() - 120000
        ),
    );
    std::fs::write(file, text).unwrap();
    let mut second = engine(&path, 9, "SELECT x FROM source").await;
    second
        .materialize(
            "source",
            MaterializationPolicy {
                max_age_seconds: 300,
                max_fill_bytes: 65536,
            },
        )
        .unwrap();
    second
        .create_view("bridge", "SELECT x FROM source")
        .await
        .unwrap();
    second
        .create_view("strict", "SELECT x FROM bridge")
        .await
        .unwrap();
    second
        .materialize(
            "strict",
            MaterializationPolicy {
                max_age_seconds: 60,
                max_fill_bytes: 65536,
            },
        )
        .unwrap();
    let result = second.query("SELECT x FROM strict").await.unwrap();
    assert_eq!(
        result[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0),
        9
    );
    std::fs::remove_dir_all(path).unwrap();
}
