use std::{collections::BTreeSet, sync::Arc};

use chrono::{DateTime, NaiveDate};
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, BooleanArray, Date32Array, Int16Array, Int32Array,
            TimestampMicrosecondArray,
        },
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    BUSINESS_CALENDAR_VERSION, BusinessCalendarRule, CalendarSourceBasis, GovernedFilter, Relation,
    RelationSemantics, RowPolicy,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    BusinessCalendarField, Comparison, Literal, RelationInput, Requirement, RowOperation, RowQuery,
};

fn micros(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_micros()
}
fn date32(year: i32, month: u32, day: u32) -> i32 {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .signed_duration_since(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
        .num_days() as i32
}

#[derive(Clone, Copy)]
enum Mapping {
    Complete,
    Missing,
    Duplicate,
    PolicyHiddenDuplicate,
}

fn engine(case: Mapping, mapping_revision: &str) -> Engine {
    let source_schema = Arc::new(Schema::new(vec![Field::new(
        "occurred_at",
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        false,
    )]));
    // The spring transition changes the Zurich UTC offset. These instants
    // resolve to March 28, 29 and 30 in the authored local calendar.
    let source_rows = RecordBatch::try_new(
        source_schema.clone(),
        vec![Arc::new(
            TimestampMicrosecondArray::from(vec![
                micros("2026-03-28T22:30:00Z"),
                micros("2026-03-28T23:30:00Z"),
                micros("2026-03-29T22:30:00Z"),
            ])
            .with_timezone("UTC"),
        ) as ArrayRef],
    )
    .unwrap();
    let mut source = Relation::base("events", source_schema.clone(), "memory");
    source.semantics = Some(RelationSemantics {
        business_calendars: [(
            "fiscal".into(),
            BusinessCalendarRule {
                version: BUSINESS_CALENDAR_VERSION,
                id: "calendar/events-fiscal".into(),
                source_relation: "events".into(),
                calendar_relation: "fiscal_days".into(),
                source_date_field: "occurred_at".into(),
                calendar_date_field: "date".into(),
                fiscal_year_field: "fiscal_year".into(),
                fiscal_period_field: "fiscal_period".into(),
                business_day_field: "business_day".into(),
                source_basis: CalendarSourceBasis::UtcInstantMicros,
                timezone: "Europe/Zurich".into(),
                mapping_revision: mapping_revision.into(),
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    let mut dates = vec![
        date32(2026, 3, 28),
        date32(2026, 3, 29),
        date32(2026, 3, 30),
    ];
    let mut periods = vec![28, 29, 30];
    let mut visible = vec![true, true, true];
    match case {
        Mapping::Missing => {
            dates.remove(1);
            periods.remove(1);
            visible.remove(1);
        }
        Mapping::Duplicate | Mapping::PolicyHiddenDuplicate => {
            dates.push(date32(2026, 3, 29));
            periods.push(99);
            visible.push(matches!(case, Mapping::Duplicate));
        }
        Mapping::Complete => {}
    }
    let mapping_schema = Arc::new(Schema::new(vec![
        Field::new("date", DataType::Date32, false),
        Field::new("fiscal_year", DataType::Int32, false),
        Field::new("fiscal_period", DataType::Int16, false),
        Field::new("business_day", DataType::Boolean, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let mapping_rows = RecordBatch::try_new(
        mapping_schema.clone(),
        vec![
            Arc::new(Date32Array::from(dates)) as ArrayRef,
            Arc::new(Int32Array::from(vec![2026; periods.len()])),
            Arc::new(Int16Array::from(periods)),
            Arc::new(BooleanArray::from(vec![true; visible.len()])),
            Arc::new(BooleanArray::from(visible)),
        ],
    )
    .unwrap();
    let mut calendar = Relation::base("fiscal_days", mapping_schema.clone(), "memory");
    calendar.semantics = Some(RelationSemantics {
        row_policies: vec![RowPolicy {
            id: "calendar/visible".into(),
            filters: vec![GovernedFilter {
                field: "visible".into(),
                operator: Comparison::Eq,
                value: Literal::Boolean(true),
            }],
            source_refs: vec![],
        }],
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            calendar,
            Arc::new(MemTable::try_new(mapping_schema, vec![vec![mapping_rows]]).unwrap()),
        )
        .unwrap();
    engine
        .register_table(
            source,
            Arc::new(MemTable::try_new(source_schema, vec![vec![source_rows]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "events".into(),
            instance: "e".into(),
        },
        requirements: vec![Requirement {
            id: "fiscal_period".into(),
            source_text: "Fiscal period in the Zurich calendar".into(),
            operation: RowOperation::BusinessCalendar {
                calendar: "fiscal".into(),
                field: BusinessCalendarField::FiscalPeriod,
                alias: "period".into(),
            },
        }],
        unresolved: vec![],
    }
}

fn periods(batches: &[RecordBatch]) -> Vec<i16> {
    let mut result = batches
        .iter()
        .flat_map(|batch| {
            let values = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int16Array>()
                .unwrap();
            (0..values.len())
                .map(|index| {
                    assert!(!values.is_null(index));
                    values.value(index)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}

#[tokio::test]
async fn utc_instants_map_across_zurich_dst_with_sql_direct_parity() {
    for case in [Mapping::Complete, Mapping::PolicyHiddenDuplicate] {
        let engine = engine(case, "FY26-v1");
        let result = compile_rows(&engine, query(), CompileOptions::default()).await;
        assert!(result.record.definition_refs.iter().any(|reference| {
            reference.id == "functions/semantic_local_date_us_v1"
                && reference.revision == "1-chrono-tz-0.10.4"
        }));
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
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
        assert_eq!(periods(&direct), vec![28, 29, 30]);
        assert_eq!(periods(&sql), vec![28, 29, 30]);
    }
}

#[tokio::test]
async fn utc_local_date_missing_and_visible_duplicate_fail_same_query() {
    for case in [Mapping::Missing, Mapping::Duplicate] {
        let engine = engine(case, "FY26-v1");
        let result = compile_rows(&engine, query(), CompileOptions::default()).await;
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
        assert!(
            query
                .plan_direct(&engine)
                .await
                .unwrap()
                .collect()
                .await
                .is_err()
        );
        assert!(
            query
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn mapping_revision_changes_dependency_and_scope_must_include_calendar_relation() {
    let original = engine(Mapping::Complete, "FY26-v1");
    let revised = engine(Mapping::Complete, "FY26-v2");
    let original_result = compile_rows(&original, query(), CompileOptions::default()).await;
    let revised_result = compile_rows(&revised, query(), CompileOptions::default()).await;
    let calendar_revision = |record: &semantic_compiler::typed::CompilationRecord| {
        record
            .definition_refs
            .iter()
            .find(|reference| reference.id == "calendar/events-fiscal")
            .unwrap()
            .revision
            .clone()
    };
    assert_ne!(
        calendar_revision(&original_result.record),
        calendar_revision(&revised_result.record)
    );

    let mut options = CompileOptions::default();
    options.allowed_relations = Some(BTreeSet::from(["events".into()]));
    let scoped = compile_rows(&original, query(), options).await;
    assert!(matches!(
        scoped.outcome,
        TypedOutcome::Rejected { diagnostic } | TypedOutcome::Unresolved { diagnostic }
            if diagnostic.code == "access_scope"
    ));
}
