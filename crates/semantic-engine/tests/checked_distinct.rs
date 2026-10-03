use std::sync::Arc;

use datafusion::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_engine::Engine;

fn result(batch: &RecordBatch) -> ScalarValue {
    ScalarValue::try_from_array(batch.column(0), 0).unwrap()
}

#[tokio::test]
async fn exact_distinct_unions_identities_and_ignores_null() {
    let engine = Engine::new();
    let batches = engine
        .query("SELECT semantic_exact_count_i64_v1(id) FROM (VALUES (1::bigint),(2),(1),(NULL::bigint),(-1)) t(id)")
        .await
        .unwrap();
    assert_eq!(result(&batches[0]), ScalarValue::Int64(Some(3)));
    for sql in [
        "SELECT semantic_exact_count_i64_v1(id) FROM (VALUES (NULL::bigint)) t(id)",
        "SELECT semantic_exact_count_i64_v1(id) FROM (VALUES (1::bigint)) t(id) WHERE false",
    ] {
        let batches = engine.query(sql).await.unwrap();
        assert_eq!(result(&batches[0]), ScalarValue::Int64(Some(0)));
    }
}

#[tokio::test]
async fn exact_distinct_merge_is_partition_invariant_across_duplicates() {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
    let batch = |values| {
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))]).unwrap()
    };
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("partitioned", schema.clone(), "memory"),
            Arc::new(
                MemTable::try_new(
                    schema.clone(),
                    vec![
                        vec![batch(vec![Some(1), Some(2), None])],
                        vec![batch(vec![Some(2), Some(3), Some(1)])],
                    ],
                )
                .unwrap(),
            ),
        )
        .unwrap();
    engine
        .register_table(
            Relation::base("single", schema.clone(), "memory"),
            Arc::new(
                MemTable::try_new(
                    schema.clone(),
                    vec![vec![batch(vec![
                        Some(1),
                        Some(2),
                        None,
                        Some(2),
                        Some(3),
                        Some(1),
                    ])]],
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let partitioned = engine
        .query("SELECT semantic_exact_count_i64_v1(id) FROM partitioned")
        .await
        .unwrap();
    let single = engine
        .query("SELECT semantic_exact_count_i64_v1(id) FROM single")
        .await
        .unwrap();
    assert_eq!(result(&partitioned[0]), result(&single[0]));
    assert_eq!(result(&partitioned[0]), ScalarValue::Int64(Some(3)));
}

#[tokio::test]
async fn exact_distinct_rejects_unsupported_identity_type() {
    let engine = Engine::new();
    assert!(
        engine
            .query("SELECT semantic_exact_count_i64_v1('one')")
            .await
            .is_err()
    );
}
