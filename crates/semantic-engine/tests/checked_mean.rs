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
use std::sync::Arc;

fn values(batch: &RecordBatch) -> Vec<ScalarValue> {
    (0..batch.num_rows())
        .map(|row| ScalarValue::try_from_array(batch.column(0), row).unwrap())
        .collect()
}

#[tokio::test]
async fn checked_mean_uses_sum_count_and_exact_decimal_truncation() {
    let engine = Engine::new();
    let batches = engine
        .query("SELECT semantic_mean_i64_v1(v) FROM (VALUES (10::bigint),(20),(20),(20)) t(v)")
        .await
        .unwrap();
    assert_eq!(
        values(&batches[0]),
        vec![ScalarValue::Decimal128(
            Some(17_500_000_000_000_000_000),
            38,
            18
        )]
    );
    let batches = engine
        .query("SELECT semantic_mean_i64_v1(v) FROM (VALUES (1::bigint),(2),(2)) t(v)")
        .await
        .unwrap();
    assert_eq!(
        values(&batches[0]),
        vec![ScalarValue::Decimal128(
            Some(1_666_666_666_666_666_666),
            38,
            18
        )]
    );
    let batches = engine
        .query("SELECT semantic_mean_i64_v1(v) FROM (VALUES (-1::bigint),(0),(0)) t(v)")
        .await
        .unwrap();
    assert_eq!(
        values(&batches[0]),
        vec![ScalarValue::Decimal128(
            Some(-333_333_333_333_333_333),
            38,
            18
        )]
    );
}

#[tokio::test]
async fn checked_mean_preserves_empty_null_and_partition_merge_results() {
    let engine = Engine::new();
    for sql in [
        "SELECT semantic_mean_i64_v1(v) FROM (VALUES (NULL::bigint)) t(v)",
        "SELECT semantic_mean_i64_v1(v) FROM (VALUES (1::bigint)) t(v) WHERE false",
    ] {
        let batches = engine.query(sql).await.unwrap();
        assert_eq!(
            values(&batches[0]),
            vec![ScalarValue::Decimal128(None, 38, 18)]
        );
    }

    let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, true)]));
    let chunks = [vec![Some(10)], vec![Some(20), None, Some(20), Some(20)]];
    let make_batch = |chunk: Vec<Option<i64>>| {
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(chunk))]).unwrap()
    };
    let mut partitioned = Engine::new();
    partitioned
        .register_table(
            Relation::base("partitioned", schema.clone(), "memory"),
            Arc::new(
                MemTable::try_new(
                    schema.clone(),
                    vec![
                        vec![make_batch(chunks[0].clone())],
                        vec![make_batch(chunks[1].clone())],
                    ],
                )
                .unwrap(),
            ),
        )
        .unwrap();
    partitioned
        .register_table(
            Relation::base("single", schema.clone(), "memory"),
            Arc::new(
                MemTable::try_new(
                    schema.clone(),
                    vec![vec![make_batch(chunks.into_iter().flatten().collect())]],
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let a = partitioned
        .query("SELECT semantic_mean_i64_v1(v) FROM partitioned")
        .await
        .unwrap();
    let b = partitioned
        .query("SELECT semantic_mean_i64_v1(v) FROM single")
        .await
        .unwrap();
    assert_eq!(values(&a[0]), values(&b[0]));
    assert_eq!(
        values(&a[0]),
        vec![ScalarValue::Decimal128(
            Some(17_500_000_000_000_000_000),
            38,
            18
        )]
    );
}

#[tokio::test]
async fn checked_mean_rejects_wrong_input_and_datafusion_distinct_is_exact() {
    let engine = Engine::new();
    assert!(
        engine
            .query("SELECT semantic_mean_i64_v1('1')")
            .await
            .is_err()
    );
    let batches = engine
        .query("SELECT semantic_mean_i64_v1(DISTINCT v) FROM (VALUES (1::bigint),(1),(2)) t(v)")
        .await
        .unwrap();
    assert_eq!(
        values(&batches[0]),
        vec![ScalarValue::Decimal128(
            Some(1_500_000_000_000_000_000),
            38,
            18
        )]
    );
}
