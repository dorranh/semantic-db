use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray, TimestampMicrosecondArray},
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_engine::Engine;

fn table(conflict: bool, partitions: bool) -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("value", DataType::Int64, true),
        Field::new(
            "observed_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("tie_key", DataType::Int64, false),
    ]));
    let batch = |regions: Vec<&str>, values: Vec<Option<i64>>, times: Vec<i64>, ties: Vec<i64>| {
        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(regions)) as ArrayRef,
                Arc::new(Int64Array::from(values)),
                Arc::new(TimestampMicrosecondArray::from(times).with_timezone("UTC")),
                Arc::new(Int64Array::from(ties)),
            ],
        )
        .unwrap()
    };
    let mut chunks = vec![
        batch(
            vec!["A", "A"],
            vec![Some(100), Some(200)],
            vec![10, 20],
            vec![1, 1],
        ),
        batch(
            vec!["A", "B"],
            vec![Some(50), None],
            vec![20, 5],
            vec![2, 1],
        ),
    ];
    if conflict {
        chunks.push(batch(vec!["A"], vec![Some(51)], vec![20], vec![2]));
    }
    let batches = if partitions {
        chunks.into_iter().map(|chunk| vec![chunk]).collect()
    } else {
        vec![chunks]
    };
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("balances", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, batches).unwrap()),
        )
        .unwrap();
    engine
}

fn rows(batches: &[RecordBatch]) -> Vec<(String, ScalarValue)> {
    let mut rows = batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows())
                .map(|index| {
                    let key = batch
                        .column(0)
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .unwrap();
                    (
                        key.value(index).to_owned(),
                        ScalarValue::try_from_array(batch.column(1), index).unwrap(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

#[tokio::test]
async fn snapshot_balance_selects_latest_tie_without_summing_and_merges_partitions() {
    let sql = "SELECT region, semantic_snapshot_balance_i64_v1(value, observed_at, tie_key) AS balance FROM balances GROUP BY region";
    let partitioned = table(false, true).query(sql).await.unwrap();
    let single = table(false, false).query(sql).await.unwrap();
    let expected = vec![
        ("A".into(), ScalarValue::Int64(Some(50))),
        ("B".into(), ScalarValue::Int64(None)),
    ];
    assert_eq!(rows(&partitioned), expected);
    assert_eq!(rows(&single), expected);
    let empty = table(false, true).query("SELECT semantic_snapshot_balance_i64_v1(value, observed_at, tie_key) FROM balances WHERE false").await.unwrap();
    assert_eq!(
        ScalarValue::try_from_array(empty[0].column(0), 0).unwrap(),
        ScalarValue::Int64(None)
    );
}

#[tokio::test]
async fn snapshot_balance_rejects_equal_order_conflicts_and_wrong_time_type() {
    assert!(
        table(true, true)
            .query(
                "SELECT semantic_snapshot_balance_i64_v1(value, observed_at, tie_key) FROM balances"
            )
            .await
            .is_err()
    );
    assert!(
        Engine::new()
            .query("SELECT semantic_snapshot_balance_i64_v1(1::bigint, 1::bigint, 1::bigint)")
            .await
            .is_err()
    );
}
