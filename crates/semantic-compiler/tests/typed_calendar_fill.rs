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

const JAN: i64 = 1_767_225_600_000_000;
const FEB: i64 = 1_769_904_000_000_000;
const MAR: i64 = 1_772_323_200_000_000;
const APR: i64 = 1_775_001_600_000_000;

fn engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "ordered_at",
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(
            TimestampMicrosecondArray::from(vec![JAN + 1, MAR + 1, MAR + 2])
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

fn query(fill: bool) -> RowQuery {
    let mut requirements = vec![
        Requirement {
            id: "month".into(),
            source_text: "by UTC month".into(),
            operation: RowOperation::CalendarGroup {
                field: FieldRef {
                    instance: "o".into(),
                    field: "ordered_at".into(),
                },
                grain: CalendarUnit::Month,
                timezone: "UTC".into(),
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
    ];
    if fill {
        requirements.push(Requirement {
            id: "fill".into(),
            source_text: "include empty February as zero".into(),
            operation: RowOperation::CalendarFill {
                month_slot: "month".into(),
                count_slot: "count".into(),
                start_us: JAN,
                end_us: APR,
                fill: 0,
            },
        });
    }
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "orders".into(),
            instance: "o".into(),
        },
        requirements,
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
async fn explicit_fill_adds_absent_month_with_sql_direct_parity() {
    let engine = engine();
    let expected = BTreeMap::from([(JAN, 1), (FEB, 0), (MAR, 2)]);
    let observed = compile_rows(&engine, query(false), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: observed } = observed.outcome else {
        panic!("{:?}", observed.outcome)
    };
    let observed_rows = observed
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(rows(&observed_rows), BTreeMap::from([(JAN, 1), (MAR, 2)]));

    let filled = compile_rows(&engine, query(true), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: filled } = filled.outcome else {
        panic!("{:?}", filled.outcome)
    };
    assert!(filled.sql().statement().contains("VALUES"));
    let direct = filled
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = filled
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
}

#[tokio::test]
async fn unaligned_unbounded_and_nonzero_fill_reject() {
    let engine = engine();
    for (start, end, value, code) in [
        (JAN + 1, APR, 0, "calendar_spine_range"),
        (JAN, APR, 1, "calendar_fill_profile"),
        (JAN, 2_114_380_800_000_000, 0, "calendar_spine_limit"),
    ] {
        let mut request = query(true);
        let RowOperation::CalendarFill {
            start_us,
            end_us,
            fill,
            ..
        } = &mut request.requirements[2].operation
        else {
            unreachable!()
        };
        *start_us = start;
        *end_us = end;
        *fill = value;
        let result = compile_rows(&engine, request, CompileOptions::default()).await;
        let matches_expected = match &result.outcome {
            TypedOutcome::Rejected { diagnostic } if code != "calendar_spine_limit" => {
                diagnostic.code == code
            }
            TypedOutcome::Unresolved { diagnostic } if code == "calendar_spine_limit" => {
                diagnostic.code == code
            }
            _ => false,
        };
        assert!(matches_expected, "{code}: {:?}", result.outcome);
    }
}
