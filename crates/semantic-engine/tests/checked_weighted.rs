use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_engine::Engine;

fn value(batch: &RecordBatch) -> ScalarValue {
    ScalarValue::try_from_array(batch.column(0), 0).unwrap()
}

#[tokio::test]
async fn weighted_mean_uses_exact_value_weight_components() {
    let engine = Engine::new();
    let cases = [
        (
            "SELECT semantic_weighted_mean_i64_v1(v,w) FROM (VALUES (10::bigint,1::bigint),(20,3)) t(v,w)",
            Some(17_500_000_000_000_000_000),
        ),
        (
            "SELECT semantic_weighted_mean_i64_v1(v,w) FROM (VALUES (-1::bigint,1::bigint),(0,2)) t(v,w)",
            Some(-333_333_333_333_333_333),
        ),
        (
            "SELECT semantic_weighted_mean_i64_v1(v,w) FROM (VALUES (1::bigint,0::bigint),(2,0)) t(v,w)",
            None,
        ),
        (
            "SELECT semantic_weighted_mean_i64_v1(v,w) FROM (VALUES (NULL::bigint,1::bigint),(1,NULL::bigint)) t(v,w)",
            None,
        ),
        (
            "SELECT semantic_weighted_mean_i64_v1(v,w) FROM (VALUES (1::bigint,1::bigint)) t(v,w) WHERE false",
            None,
        ),
    ];
    for (sql, expected) in cases {
        let batches = engine.query(sql).await.unwrap();
        assert_eq!(
            value(&batches[0]),
            ScalarValue::Decimal128(expected, 38, 18)
        );
    }
}

#[tokio::test]
async fn weighted_mean_merge_is_partition_invariant() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("v", DataType::Int64, true),
        Field::new("w", DataType::Int64, true),
    ]));
    let batch = |values: Vec<Option<i64>>, weights: Vec<Option<i64>>| {
        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(values)) as ArrayRef,
                Arc::new(Int64Array::from(weights)) as ArrayRef,
            ],
        )
        .unwrap()
    };
    let chunks = [
        batch(vec![Some(10), None], vec![Some(1), Some(9)]),
        batch(vec![Some(20), Some(20)], vec![Some(2), Some(1)]),
    ];
    let whole = batch(
        vec![Some(10), None, Some(20), Some(20)],
        vec![Some(1), Some(9), Some(2), Some(1)],
    );
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("partitioned", schema.clone(), "memory"),
            Arc::new(
                MemTable::try_new(
                    schema.clone(),
                    vec![vec![chunks[0].clone()], vec![chunks[1].clone()]],
                )
                .unwrap(),
            ),
        )
        .unwrap();
    engine
        .register_table(
            Relation::base("whole", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![whole]]).unwrap()),
        )
        .unwrap();
    let a = engine
        .query("SELECT semantic_weighted_mean_i64_v1(v,w) FROM partitioned")
        .await
        .unwrap();
    let b = engine
        .query("SELECT semantic_weighted_mean_i64_v1(v,w) FROM whole")
        .await
        .unwrap();
    assert_eq!(value(&a[0]), value(&b[0]));
    assert_eq!(
        value(&a[0]),
        ScalarValue::Decimal128(Some(17_500_000_000_000_000_000), 38, 18)
    );
}

#[tokio::test]
async fn weighted_mean_rejects_negative_weight_wrong_type_and_state_overflow() {
    let engine = Engine::new();
    for sql in [
        "SELECT semantic_weighted_mean_i64_v1(v,w) FROM (VALUES (1::bigint,-1::bigint)) t(v,w)",
        "SELECT semantic_weighted_mean_i64_v1('1',1::bigint)",
        "SELECT semantic_weighted_mean_i64_v1(v,w) FROM (VALUES (9223372036854775807::bigint,9223372036854775807::bigint),(9223372036854775807,9223372036854775807),(9223372036854775807,9223372036854775807)) t(v,w)",
    ] {
        assert!(engine.query(sql).await.is_err());
    }
}
