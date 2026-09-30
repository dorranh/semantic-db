use datafusion::{
    arrow::{
        array::{Array, Int64Array, TimestampMicrosecondArray},
        datatypes::{DataType, Field, Schema, TimeUnit},
    },
    common::ScalarValue,
    prelude::SessionContext,
};

use super::{plan, sql_query};
use crate::typed::calendar_spine::TypedValues;

fn schema() -> Schema {
    Schema::new(vec![
        Field::new(
            "period",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("count", DataType::Int64, true),
    ])
}

#[tokio::test]
async fn two_column_values_have_identical_direct_and_sql_cells() {
    let values = TypedValues::new(
        schema(),
        vec![
            vec![
                ScalarValue::TimestampMicrosecond(Some(1_767_225_600_000_000), Some("UTC".into())),
                ScalarValue::Int64(Some(3)),
            ],
            vec![
                ScalarValue::TimestampMicrosecond(Some(1_769_904_000_000_000), Some("UTC".into())),
                ScalarValue::Int64(None),
            ],
        ],
        2,
    )
    .unwrap();
    let context = SessionContext::new();
    let seed = context.sql("SELECT 1").await.unwrap();
    let (direct, _) = plan(seed, &values).unwrap();
    let direct = direct.collect().await.unwrap();
    let sql = format!("SELECT * FROM ({}) AS v(period, count)", sql_query(&values));
    let sql = context.sql(&sql).await.unwrap().collect().await.unwrap();
    for batches in [&direct, &sql] {
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        let periods = batch
            .column(0)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap();
        let counts = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(periods.value(0), 1_767_225_600_000_000);
        assert_eq!(periods.value(1), 1_769_904_000_000_000);
        assert_eq!(counts.value(0), 3);
        assert!(counts.is_null(1));
    }
}

#[test]
fn values_reject_invalid_width_type_null_and_budget() {
    let valid_period = ScalarValue::TimestampMicrosecond(Some(0), Some("UTC".into()));
    for row in [
        vec![valid_period.clone()],
        vec![
            ScalarValue::Utf8(Some("today".into())),
            ScalarValue::Int64(Some(1)),
        ],
        vec![
            ScalarValue::TimestampMicrosecond(None, Some("UTC".into())),
            ScalarValue::Int64(Some(1)),
        ],
    ] {
        assert_eq!(
            TypedValues::new(schema(), vec![row], 1).unwrap_err().code,
            "values_contract"
        );
    }
    assert_eq!(
        TypedValues::new(
            schema(),
            vec![vec![valid_period.clone(), ScalarValue::Int64(Some(1))]; 2],
            1,
        )
        .unwrap_err()
        .code,
        "values_contract"
    );
    assert_eq!(
        TypedValues::new(
            schema(),
            vec![vec![valid_period, ScalarValue::Int64(Some(1))]; 121],
            121,
        )
        .unwrap_err()
        .code,
        "values_contract"
    );
    let unsupported = Schema::new(vec![Field::new("label", DataType::Utf8, true)]);
    assert_eq!(
        TypedValues::new(unsupported, vec![vec![ScalarValue::Utf8(None)]], 1)
            .unwrap_err()
            .code,
        "values_contract"
    );
}
