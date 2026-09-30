use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, BooleanArray, Date32Array, Int16Array, Int32Array},
        datatypes::{DataType, Field, Schema},
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

#[derive(Clone, Copy)]
enum Case {
    Complete,
    Missing,
    Duplicate,
    PolicyFiltered,
}

fn fixture(case: Case) -> Engine {
    let mut dates = vec![20_000, 20_001, 20_002];
    let mut years = vec![2026, 2026, 2026];
    let mut periods = vec![1, 2, 3];
    let mut business = vec![true, false, true];
    let mut visible = vec![true, true, true];
    match case {
        Case::Missing => {
            dates.remove(1);
            years.remove(1);
            periods.remove(1);
            business.remove(1);
            visible.remove(1);
        }
        Case::Duplicate => {
            dates.push(20_001);
            years.push(2026);
            periods.push(9);
            business.push(true);
            visible.push(true);
        }
        Case::PolicyFiltered => visible[1] = false,
        Case::Complete => {}
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
            Arc::new(Int32Array::from(years)),
            Arc::new(Int16Array::from(periods)),
            Arc::new(BooleanArray::from(business)),
            Arc::new(BooleanArray::from(visible)),
        ],
    )
    .unwrap();
    let mut mapping = Relation::base("fiscal_days", mapping_schema.clone(), "memory");
    mapping.semantics = Some(RelationSemantics {
        row_policies: vec![RowPolicy {
            id: "policies/calendar-visible".into(),
            filters: vec![GovernedFilter {
                field: "visible".into(),
                operator: Comparison::Eq,
                value: Literal::Boolean(true),
            }],
            source_refs: vec![],
        }],
        ..Default::default()
    });
    let source_schema = Arc::new(Schema::new(vec![Field::new(
        "booked_date",
        DataType::Date32,
        false,
    )]));
    let source_rows = RecordBatch::try_new(
        source_schema.clone(),
        vec![Arc::new(Date32Array::from(vec![20_000, 20_001, 20_002])) as ArrayRef],
    )
    .unwrap();
    let mut source = Relation::base("orders", source_schema.clone(), "memory");
    source.semantics = Some(RelationSemantics {
        business_calendars: [(
            "fiscal".into(),
            BusinessCalendarRule {
                version: BUSINESS_CALENDAR_VERSION,
                id: "calendar/orders-fiscal".into(),
                source_relation: "orders".into(),
                calendar_relation: "fiscal_days".into(),
                source_date_field: "booked_date".into(),
                calendar_date_field: "date".into(),
                fiscal_year_field: "fiscal_year".into(),
                fiscal_period_field: "fiscal_period".into(),
                business_day_field: "business_day".into(),
                source_basis: CalendarSourceBasis::Date32,
                timezone: "Europe/Zurich".into(),
                mapping_revision: "FY26-v1".into(),
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            mapping,
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

fn query(calendar: &str, field: BusinessCalendarField) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "orders".into(),
            instance: "o".into(),
        },
        requirements: vec![Requirement {
            id: "period".into(),
            source_text: "Fiscal period from the authored calendar".into(),
            operation: RowOperation::BusinessCalendar {
                calendar: calendar.into(),
                field,
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
async fn authored_fiscal_period_has_sql_direct_parity_and_pinned_obligation() {
    let engine = fixture(Case::Complete);
    let result = compile_rows(
        &engine,
        query("fiscal", BusinessCalendarField::FiscalPeriod),
        CompileOptions::default(),
    )
    .await;
    assert_eq!(result.record.execution_obligations.len(), 1);
    assert!(
        result
            .record
            .definition_refs
            .iter()
            .any(|reference| reference.id == "calendar/orders-fiscal")
    );
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
    assert_eq!(periods(&direct), vec![1, 2, 3]);
    assert_eq!(periods(&sql), vec![1, 2, 3]);
}

#[tokio::test]
async fn missing_duplicate_or_policy_hidden_mapping_fails_during_execution() {
    for case in [Case::Missing, Case::Duplicate, Case::PolicyFiltered] {
        let engine = fixture(case);
        let result = compile_rows(
            &engine,
            query("fiscal", BusinessCalendarField::FiscalPeriod),
            CompileOptions::default(),
        )
        .await;
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
