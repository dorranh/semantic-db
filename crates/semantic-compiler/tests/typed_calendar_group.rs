use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, TimestampMicrosecondArray},
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

fn engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "ordered_at",
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(
            TimestampMicrosecondArray::from(vec![
                1_769_903_999_999_999,
                1_769_904_000_000_000,
                1_772_323_199_999_999,
            ])
            .with_timezone_opt(Some("UTC")),
        ) as ArrayRef],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("orders", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query(grain: CalendarUnit, timezone: &str) -> RowQuery {
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "orders".into(),
            instance: "o".into(),
        },
        requirements: vec![
            Requirement {
                id: "month".into(),
                source_text: "by UTC month".into(),
                operation: RowOperation::CalendarGroup {
                    field: FieldRef {
                        instance: "o".into(),
                        field: "ordered_at".into(),
                    },
                    grain,
                    timezone: timezone.into(),
                    alias: "month".into(),
                },
            },
            Requirement {
                id: "count".into(),
                source_text: "count orders".into(),
                operation: RowOperation::Aggregate {
                    function: AggregateFunction::Count,
                    field: None,
                    distinct: false,
                    alias: "orders".into(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> BTreeMap<i64, i64> {
    let mut rows = BTreeMap::new();
    for batch in batches {
        let months = batch
            .column(0)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap();
        let counts = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        for row in 0..batch.num_rows() {
            rows.insert(months.value(row), counts.value(row));
        }
    }
    rows
}

#[tokio::test]
async fn observed_utc_month_grouping_has_sql_direct_parity() {
    let engine = engine();
    let result = compile_rows(
        &engine,
        query(CalendarUnit::Month, "UTC"),
        CompileOptions::default(),
    )
    .await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome);
    };
    assert_eq!(query.sql().target(), "local:datafusion-55");
    assert_eq!(
        query.sql().execution_profile_revision(),
        semantic_engine::MVP_EXECUTION_PROFILE_REVISION
    );
    assert_eq!(
        query
            .sql()
            .expected_output()
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["month", "orders"]
    );
    let expected = BTreeMap::from([(1_767_225_600_000_000, 1), (1_769_904_000_000_000, 2)]);
    let direct = query
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
    assert!(query.sql().statement().contains("semantic_utc_month_us_v1"));
}

#[tokio::test]
async fn other_calendar_profiles_fail_closed() {
    let engine = engine();
    for (grain, timezone) in [
        (CalendarUnit::IsoWeek, "UTC"),
        (CalendarUnit::Month, "Europe/Zurich"),
    ] {
        let result = compile_rows(&engine, query(grain, timezone), CompileOptions::default()).await;
        assert!(
            matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "calendar_group_profile")
        );
    }
}
