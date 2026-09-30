use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, TimestampMicrosecondArray},
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    CalendarReference, CalendarSystem, FieldSemantics, Relation, RelationSemantics,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    AggregateFunction, CalendarUnit, FieldRef, ROW_QUERY_VERSION, RelationInput, Requirement,
    RowOperation, RowQuery,
};

fn engine() -> Engine {
    let timestamp = DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()));
    let schema = Arc::new(Schema::new(vec![
        Field::new("gregorian_at", timestamp.clone(), false),
        Field::new("fiscal_at", timestamp, false),
    ]));
    let values = vec![
        1_769_903_999_999_999,
        1_769_904_000_000_000,
        1_772_323_199_999_999,
    ];
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(TimestampMicrosecondArray::from(values.clone()).with_timezone_opt(Some("UTC")))
                as ArrayRef,
            Arc::new(TimestampMicrosecondArray::from(values).with_timezone_opt(Some("UTC")))
                as ArrayRef,
        ],
    )
    .unwrap();
    let mut relation = Relation::base("events", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        fields: [
            (
                "gregorian_at".into(),
                FieldSemantics {
                    calendar_reference: Some(CalendarReference {
                        system: CalendarSystem::Gregorian,
                        timezone: "UTC".into(),
                    }),
                    ..Default::default()
                },
            ),
            (
                "fiscal_at".into(),
                FieldSemantics {
                    calendar_reference: Some(CalendarReference {
                        system: CalendarSystem::Fiscal {
                            id: "company-445".into(),
                        },
                        timezone: "Europe/Zurich".into(),
                    }),
                    ..Default::default()
                },
            ),
        ]
        .into(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query(field: &str) -> RowQuery {
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "events".into(),
            instance: "e".into(),
        },
        requirements: vec![
            Requirement {
                id: "month".into(),
                source_text: "by UTC month".into(),
                operation: RowOperation::CalendarGroup {
                    field: FieldRef {
                        instance: "e".into(),
                        field: field.into(),
                    },
                    grain: CalendarUnit::Month,
                    timezone: "UTC".into(),
                    alias: "month".into(),
                },
            },
            Requirement {
                id: "count".into(),
                source_text: "count events".into(),
                operation: RowOperation::Aggregate {
                    function: AggregateFunction::Count,
                    field: None,
                    distinct: false,
                    alias: "events".into(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> BTreeMap<i64, i64> {
    let mut result = BTreeMap::new();
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
            result.insert(months.value(row), counts.value(row));
        }
    }
    result
}

#[tokio::test]
async fn identical_physical_timestamps_respect_authored_calendar_meaning() {
    let engine = engine();
    let accepted = compile_rows(&engine, query("gregorian_at"), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: compiled } = accepted.outcome else {
        panic!("Gregorian UTC month rejected: {:?}", accepted.outcome)
    };
    let direct = compiled
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = compiled
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = BTreeMap::from([(1_767_225_600_000_000, 1), (1_769_904_000_000_000, 2)]);
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);

    let rejected = compile_rows(&engine, query("fiscal_at"), CompileOptions::default()).await;
    assert!(matches!(
        rejected.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "calendar_reference"
    ));
}
