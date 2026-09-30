use std::sync::Arc;

use datafusion::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_engine::{Engine, pretty_format_batches};

fn batch(schema: Arc<Schema>, values: Vec<i64>) -> RecordBatch {
    RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(values))]).unwrap()
}

#[tokio::test]
async fn cached_logical_plan_reads_current_provider_rows() {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let table = Arc::new(
        MemTable::try_new(schema.clone(), vec![vec![batch(schema.clone(), vec![1])]]).unwrap(),
    );
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("numbers", schema.clone(), "memory"),
            table.clone(),
        )
        .unwrap();

    let sql = "SELECT value FROM numbers ORDER BY value";
    let first = engine.plan_generated_sql(sql).await.unwrap();
    let first_rows = first.collect().await.unwrap();
    assert_eq!(
        pretty_format_batches(&first_rows).unwrap().to_string(),
        "+-------+\n| value |\n+-------+\n| 1     |\n+-------+"
    );

    table.batches[0].write().await.push(batch(schema, vec![2]));
    let second = engine.plan_generated_sql(sql).await.unwrap();
    let second_rows = second.collect().await.unwrap();
    assert_eq!(
        pretty_format_batches(&second_rows).unwrap().to_string(),
        "+-------+\n| value |\n+-------+\n| 1     |\n| 2     |\n+-------+"
    );
}

#[tokio::test]
async fn a_cache_hit_does_not_relax_generated_query_authorization() {
    let mut engine = Engine::new();
    engine
        .create_view("constant", "SELECT 1 AS value")
        .await
        .unwrap();
    let sql = "SELECT value FROM constant";
    engine.plan_generated_sql(sql).await.unwrap();
    engine.plan_generated_sql(sql).await.unwrap();

    for rejected in [
        "SELECT value FROM information_schema.tables",
        "SELECT value FROM public.constant",
        "SELECT value FROM constant; SELECT value FROM constant",
        "DROP TABLE constant",
    ] {
        assert!(
            engine.plan_generated_sql(rejected).await.is_err(),
            "accepted: {rejected}"
        );
    }

    engine
        .create_view("other", "SELECT value FROM constant")
        .await
        .unwrap();
    // View registration increments the binding generation and clears the
    // retained plans. Both the old and newly registered relation still plan.
    engine.plan_generated_sql(sql).await.unwrap();
    engine
        .plan_generated_sql("SELECT value FROM other")
        .await
        .unwrap();
}
