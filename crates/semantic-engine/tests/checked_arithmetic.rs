use datafusion::{
    arrow::{
        array::{Int64Array, UInt64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    datasource::MemTable,
};
use semantic_engine::{Engine, QueryOptions, Relation};
use std::sync::Arc;

async fn fails(engine: &Engine, sql: &str) {
    let error = engine.query(sql).await.expect_err(sql).to_string();
    assert!(error.to_lowercase().contains("overflow"), "{sql}: {error}");
}

#[tokio::test]
async fn checked_arithmetic_preserves_source_expression_nullability() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("value", DataType::Int64, false)
            .with_metadata([("unit".into(), "minor".into())].into()),
        Field::new("optional", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![5, 7])),
            Arc::new(Int64Array::from(vec![None, Some(2)])),
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("numbers", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    for expression in [
        "value + 1::bigint",
        "value - 1::bigint",
        "value * 2::bigint",
        "-value",
    ] {
        let sql = format!("SELECT {expression} AS answer FROM numbers");
        let frame = engine.plan_sql(&sql).await.unwrap();
        assert!(!frame.schema().field(0).is_nullable());
        for batch in frame.collect().await.unwrap() {
            assert_eq!(batch.schema().field(0).name(), "answer");
            assert!(!batch.schema().field(0).is_nullable());
            if expression == "-value" {
                assert_eq!(
                    batch
                        .schema()
                        .field(0)
                        .metadata()
                        .get("unit")
                        .map(String::as_str),
                    Some("minor")
                );
            } else {
                assert!(batch.schema().field(0).metadata().is_empty());
            }
        }
    }
    for expression in ["value + optional", "optional * value", "-optional"] {
        let rows = engine
            .query(&format!("SELECT {expression} AS answer FROM numbers"))
            .await
            .unwrap();
        assert!(rows[0].schema().field(0).is_nullable());
        assert_eq!(rows[0].column(0).null_count(), 1);
    }
}

#[tokio::test]
async fn constants_are_checked_before_folding_at_every_integer_width() {
    let engine = Engine::new();
    for sql in [
        "SELECT 9223372036854775807::bigint * 2::bigint",
        "SELECT 9223372036854775807::bigint + 1::bigint",
        "SELECT (-9223372036854775807::bigint - 1::bigint) - 1::bigint",
        "SELECT (-9223372036854775807::bigint - 1::bigint) * (-1::bigint)",
        "SELECT 32767::smallint + 1::smallint",
        "SELECT 2147483647::int + 1::int",
        "SELECT arrow_cast(127, 'Int8') + arrow_cast(1, 'Int8')",
        "SELECT arrow_cast(255, 'UInt8') + arrow_cast(1, 'UInt8')",
        "SELECT arrow_cast(65535, 'UInt16') + arrow_cast(1, 'UInt16')",
        "SELECT arrow_cast(4294967295, 'UInt32') * arrow_cast(2, 'UInt32')",
        "SELECT arrow_cast('18446744073709551615', 'UInt64') * arrow_cast(2, 'UInt64')",
        "SELECT arrow_cast(0, 'UInt64') - arrow_cast(1, 'UInt64')",
        "SELECT (9223372036854775807::bigint + 1::bigint) * 0::bigint",
    ] {
        fails(&engine, sql).await;
    }
    let rows = engine.query("SELECT 10::smallint + 20::smallint AS result, 10::bigint - 20::bigint AS difference, 7::bigint * 6::bigint AS product").await.unwrap();
    assert_eq!(rows[0].schema().field(0).name(), "result");
    assert_eq!(
        ScalarValue::try_from_array(rows[0].column(0), 0).unwrap(),
        ScalarValue::Int16(Some(30))
    );
    assert_eq!(
        ScalarValue::try_from_array(rows[0].column(1), 0).unwrap(),
        ScalarValue::Int64(Some(-10))
    );
    assert_eq!(
        ScalarValue::try_from_array(rows[0].column(2), 0).unwrap(),
        ScalarValue::Int64(Some(42))
    );
}

fn signed(values: Vec<Option<i64>>, factors: Vec<Option<i64>>) -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("value", DataType::Int64, true),
        Field::new("factor", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(values)),
            Arc::new(Int64Array::from(factors)),
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("numbers", schema.clone(), "memory"),
            Arc::new(
                MemTable::try_new(
                    schema,
                    vec![
                        vec![batch.slice(0, 1)],
                        vec![batch.slice(1, batch.num_rows() - 1)],
                    ],
                )
                .unwrap(),
            ),
        )
        .unwrap();
    engine
}

#[tokio::test]
async fn source_arithmetic_checks_all_partitions_and_preserves_null_propagation() {
    let engine = signed(vec![Some(1), Some(i64::MAX)], vec![Some(2), Some(2)]);
    for sql in [
        "SELECT value + factor FROM numbers",
        "SELECT value * factor FROM numbers",
        "SELECT value - (-factor) FROM numbers",
    ] {
        fails(&engine, sql).await;
    }
    let engine = signed(
        vec![Some(i64::MAX), None, Some(6)],
        vec![None, Some(i64::MAX), Some(7)],
    );
    let rows = engine
        .query("SELECT value * factor AS product FROM numbers ORDER BY product NULLS FIRST")
        .await
        .unwrap();
    let values = rows
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|i| ScalarValue::try_from_array(batch.column(0), i).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec![
            ScalarValue::Int64(None),
            ScalarValue::Int64(None),
            ScalarValue::Int64(Some(42))
        ]
    );
}

#[tokio::test]
async fn unsigned_columns_parameters_and_generated_plans_use_checked_policy() {
    let schema = Arc::new(Schema::new(vec![Field::new("u", DataType::UInt64, false)]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(UInt64Array::from(vec![u64::MAX]))],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("unsigned_values", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    fails(
        &engine,
        "SELECT u + arrow_cast(1, 'UInt64') FROM unsigned_values",
    )
    .await;
    fails(
        &engine,
        "SELECT u * arrow_cast(2, 'UInt64') FROM unsigned_values",
    )
    .await;
    let error = engine
        .execute_parameters(
            "SELECT $1 * $2",
            vec![
                ScalarValue::Int64(Some(i64::MAX)),
                ScalarValue::Int64(Some(2)),
            ],
            QueryOptions::default(),
        )
        .await;
    match error {
        Err(error) => assert!(error.to_string().to_lowercase().contains("overflow")),
        Ok(execution) => assert!(
            execution
                .collect()
                .await
                .unwrap_err()
                .to_string()
                .to_lowercase()
                .contains("overflow")
        ),
    }
    let frame = engine
        .plan_generated_sql("SELECT u * arrow_cast(2, 'UInt64') FROM unsigned_values")
        .await
        .unwrap();
    assert!(
        frame
            .collect()
            .await
            .unwrap_err()
            .to_string()
            .to_lowercase()
            .contains("overflow")
    );
    let rows = engine
        .query("SELECT 32767::smallint + 1::bigint AS widened")
        .await
        .unwrap();
    assert_eq!(
        ScalarValue::try_from_array(rows[0].column(0), 0).unwrap(),
        ScalarValue::Int64(Some(32768))
    );
}

#[tokio::test]
async fn signed_negation_checks_minimum_and_preserves_nullable_boundaries() {
    let engine = Engine::new();
    for sql in [
        "SELECT -arrow_cast(-128, 'Int8')",
        "SELECT -arrow_cast(-32768, 'Int16')",
        "SELECT -arrow_cast(-2147483648, 'Int32')",
        "SELECT -arrow_cast('-9223372036854775808', 'Int64')",
    ] {
        fails(&engine, sql).await;
    }
    let engine = signed(vec![Some(0), Some(i64::MIN)], vec![Some(1), Some(1)]);
    fails(&engine, "SELECT -value FROM numbers").await;
    fails(&engine, "SELECT (-value) * 0::bigint FROM numbers").await;
    let engine = signed(
        vec![None, Some(-i64::MAX), Some(i64::MAX)],
        vec![None, Some(1), Some(1)],
    );
    let rows = engine
        .query("SELECT -value AS negated FROM numbers ORDER BY negated NULLS FIRST")
        .await
        .unwrap();
    let values = rows
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|i| ScalarValue::try_from_array(batch.column(0), i).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec![
            ScalarValue::Int64(None),
            ScalarValue::Int64(Some(-i64::MAX)),
            ScalarValue::Int64(Some(i64::MAX))
        ]
    );
    let rows = engine
        .query("SELECT -1.25::double AS float_value, -(12.34::decimal(6,2)) AS decimal_value")
        .await
        .unwrap();
    assert_eq!(
        ScalarValue::try_from_array(rows[0].column(0), 0).unwrap(),
        ScalarValue::Float64(Some(-1.25))
    );
    assert_eq!(
        ScalarValue::try_from_array(rows[0].column(1), 0).unwrap(),
        ScalarValue::Decimal128(Some(-1234), 6, 2)
    );
}
