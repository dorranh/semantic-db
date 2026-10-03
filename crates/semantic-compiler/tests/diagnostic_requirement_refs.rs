use std::sync::Arc;

use datafusion::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::Engine;
use semantic_plan::typed::*;

fn engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1, 2]))]).unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("items", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn field(name: &str) -> FieldRef {
    FieldRef {
        instance: "i".into(),
        field: name.into(),
    }
}

fn requirement(id: &str, operation: RowOperation) -> Requirement {
    Requirement {
        id: id.into(),
        source_text: id.into(),
        operation,
    }
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "items".into(),
            instance: "i".into(),
        },
        requirements: vec![requirement(
            "project_id",
            RowOperation::Project {
                field: field("id"),
                alias: "id".into(),
            },
        )],
        unresolved: vec![],
    }
}

async fn rejection(engine: &Engine, query: RowQuery, code: &str, requirement: Option<&str>) {
    let result = compile_rows(engine, query, CompileOptions::default()).await;
    let TypedOutcome::Rejected { diagnostic } = result.outcome else {
        panic!("expected rejection: {:?}", result.outcome);
    };
    assert_eq!(diagnostic.code, code);
    assert_eq!(diagnostic.details.requirement_ref.as_deref(), requirement);
}

#[tokio::test]
async fn main_requirement_missing_field_has_its_own_ref() {
    let engine = engine();
    let mut query = query();
    query.requirements.push(requirement(
        "missing_project",
        RowOperation::Project {
            field: field("absent"),
            alias: "absent".into(),
        },
    ));
    rejection(&engine, query, "unknown_field", Some("missing_project")).await;
}

#[tokio::test]
async fn output_prepass_missing_aggregate_field_and_metric_have_precise_refs() {
    let engine = engine();
    let mut aggregate = query();
    aggregate.requirements.push(requirement(
        "missing_sum",
        RowOperation::Aggregate {
            function: AggregateFunction::Sum,
            field: Some(field("absent")),
            distinct: false,
            alias: "amount".into(),
        },
    ));
    rejection(&engine, aggregate, "unknown_field", Some("missing_sum")).await;

    let mut metric = query();
    metric.requirements.push(requirement(
        "missing_metric",
        RowOperation::Metric {
            name: "unpublished_metric".into(),
            alias: "metric".into(),
            applicability: MetricApplicability::default(),
        },
    ));
    rejection(&engine, metric, "unknown_metric", Some("missing_metric")).await;
}

#[tokio::test]
async fn window_prepass_and_temporal_prepass_keep_the_owning_requirement() {
    let engine = engine();
    let mut window = query();
    window.requirements.push(requirement(
        "missing_window_key",
        RowOperation::Window {
            window: WindowSpec {
                function: WindowFunction::Rank,
                input: None,
                partition_by: vec![],
                order_by: vec![WindowOrder {
                    input: WindowInput::Field {
                        field: field("absent"),
                    },
                    direction: Direction::Asc,
                    nulls: NullOrder::Last,
                }],
                frame: WindowFrame::ThroughCurrentPeer,
            },
            alias: "rank".into(),
        },
    ));
    rejection(&engine, window, "unknown_field", Some("missing_window_key")).await;

    let mut contract = query();
    contract.requirements.push(requirement(
        "invalid_rank_frame",
        RowOperation::Window {
            window: WindowSpec {
                function: WindowFunction::Rank,
                input: None,
                partition_by: vec![],
                order_by: vec![WindowOrder {
                    input: WindowInput::Field { field: field("id") },
                    direction: Direction::Asc,
                    nulls: NullOrder::Last,
                }],
                frame: WindowFrame::EntirePartition,
            },
            alias: "rank".into(),
        },
    ));
    rejection(
        &engine,
        contract,
        "window_contract",
        Some("invalid_rank_frame"),
    )
    .await;

    let mut temporal = query();
    temporal.requirements.push(requirement(
        "missing_time",
        RowOperation::CalendarFilter {
            field: field("absent"),
            period: CalendarPeriod {
                unit: CalendarUnit::Month,
                offset: -1,
                count: 1,
            },
        },
    ));
    rejection(&engine, temporal, "unknown_field", Some("missing_time")).await;
}

#[tokio::test]
async fn input_global_failure_has_no_invented_requirement_ref() {
    let engine = engine();
    let mut query = query();
    query.input.relation = "not_published".into();
    rejection(&engine, query, "unknown_relation", None).await;
}
