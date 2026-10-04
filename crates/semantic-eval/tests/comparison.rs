use datafusion::arrow::{
    array::{Float64Array, Int64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use semantic_eval::*;
use serde_json::json;
use std::sync::Arc;
fn col(kind: &str) -> Column {
    Column {
        name: "value".into(),
        kind: kind.into(),
        nullable: true,
        physical_nullable: None,
        tolerance: None,
    }
}
fn expected(kind: &str, rows: Vec<Vec<serde_json::Value>>) -> Expected {
    Expected::Result {
        columns: vec![col(kind)],
        rows,
    }
}
#[test]
fn bags_retain_duplicates_and_order_is_opt_in() {
    let e = expected(
        "int64",
        vec![vec![json!("7")], vec![json!("7")], vec![json!("3")]],
    );
    let a = TypedResult {
        columns: vec![col("int32")],
        rows: vec![vec![json!("3")], vec![json!("7")], vec![json!("7")]],
    };
    assert!(compare(&e, &a, &Comparison::default()).unwrap().is_empty());
    assert!(
        !compare(
            &e,
            &a,
            &Comparison {
                ordered: true,
                ..Default::default()
            }
        )
        .unwrap()
        .is_empty()
    );
    let a = TypedResult {
        rows: vec![vec![json!("3")], vec![json!("7")]],
        ..a
    };
    assert!(!compare(&e, &a, &Default::default()).unwrap().is_empty());
}
#[test]
fn decimals_are_exact_with_precision_promotion() {
    let e = expected("decimal128(18,2)", vec![vec![json!("123456789012.34")]]);
    let mut a = TypedResult {
        columns: vec![col("decimal128(38,4)")],
        rows: vec![vec![json!("123456789012.3400")]],
    };
    assert!(compare(&e, &a, &Default::default()).unwrap().is_empty());
    a.rows[0][0] = json!("123456789012.3401");
    assert!(!compare(&e, &a, &Default::default()).unwrap().is_empty());
}
#[test]
fn null_empty_and_schema_are_distinct() {
    let e = expected("utf8", vec![vec![json!(null)]]);
    let a = TypedResult {
        columns: vec![col("utf8")],
        rows: vec![vec![json!("")]],
    };
    assert!(!compare(&e, &a, &Default::default()).unwrap().is_empty());
    let e = expected("int64", vec![]);
    let a = TypedResult {
        columns: vec![col("utf8")],
        rows: vec![],
    };
    assert!(!compare(&e, &a, &Default::default()).unwrap().is_empty());
}
#[test]
fn float_tolerance_uses_complete_matching() {
    let mut c = col("float64");
    c.tolerance = Some(Tolerance {
        absolute: 1.,
        relative: 0.,
    });
    let e = Expected::Result {
        columns: vec![c],
        rows: vec![vec![json!(1.)], vec![json!(0.)]],
    };
    let a = TypedResult {
        columns: vec![col("float32")],
        rows: vec![vec![json!(0.)], vec![json!(2.)]],
    };
    assert!(compare(&e, &a, &Default::default()).unwrap().is_empty());
}
#[test]
fn rejects_nan_and_malformed_integer() {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Float64,
        true,
    )]));
    let b = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Float64Array::from(vec![f64::NAN]))],
    )
    .unwrap();
    assert!(result_from_batches(&schema, &[b]).is_err());
    assert!(validate_expected(&expected("int16", vec![vec![json!("32768")]])).is_err());
}
#[test]
fn batch_boundaries_and_empty_schema_preserved() {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let a =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1, 2]))]).unwrap();
    let b =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1]))]).unwrap();
    let c =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![2]))]).unwrap();
    assert_eq!(
        serde_json::to_value(result_from_batches(&schema, &[a]).unwrap()).unwrap(),
        serde_json::to_value(result_from_batches(&schema, &[b, c]).unwrap()).unwrap()
    );
    assert_eq!(result_from_batches(&schema, &[]).unwrap().columns.len(), 1);
}
#[test]
fn temporal_precision_and_naive_vs_instant() {
    assert!(
        validate_expected(&expected(
            "time64(us)",
            vec![vec![json!("12:00:00.000000001")]]
        ))
        .is_err()
    );
    assert!(validate_expected(&expected("timestamp(us,Fake)", vec![])).is_err());
    let e = expected(
        "timestamp(us)",
        vec![vec![json!("2024-02-29T12:00:00.000001")]],
    );
    let a = TypedResult {
        columns: vec![col("timestamp(ns)")],
        rows: vec![vec![json!("2024-02-29T12:00:00.000001000")]],
    };
    assert!(compare(&e, &a, &Default::default()).unwrap().is_empty());
    let instant = TypedResult {
        columns: vec![col("timestamp(us,UTC)")],
        rows: vec![vec![json!("2024-02-29T12:00:00.000001Z")]],
    };
    assert!(
        !compare(&e, &instant, &Default::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unsigned_rank_and_large_uint64_are_lossless_with_range_validation() {
    use datafusion::arrow::array::UInt64Array;
    let schema = Arc::new(Schema::new(vec![Field::new(
        "rank",
        DataType::UInt64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(UInt64Array::from(vec![u64::MAX]))],
    )
    .unwrap();
    let actual = result_from_batches(&schema, &[batch]).unwrap();
    assert_eq!(actual.rows[0][0], json!("18446744073709551615"));
    let gold = expected("uint64", vec![vec![json!("18446744073709551615")]]);
    assert!(
        compare(&gold, &actual, &Default::default())
            .unwrap()
            .is_empty()
    );
    for (kind, value) in [
        ("uint8", "256"),
        ("uint16", "65536"),
        ("uint32", "4294967296"),
        ("uint64", "18446744073709551616"),
        ("uint64", "-1"),
        ("int64", "9223372036854775808"),
    ] {
        assert!(
            validate_expected(&expected(kind, vec![vec![json!(value)]])).is_err(),
            "{kind} {value}"
        );
    }
    let rank = TypedResult {
        columns: vec![col("uint64")],
        rows: vec![vec![json!("1")]],
    };
    let signed = expected("int64", vec![vec![json!("1")]]);
    assert!(
        compare(&signed, &rank, &Default::default())
            .unwrap()
            .is_empty()
    );
    assert!(
        !compare(
            &signed,
            &rank,
            &Comparison {
                assert_physical_types: true,
                ..Default::default()
            }
        )
        .unwrap()
        .is_empty()
    );
}
