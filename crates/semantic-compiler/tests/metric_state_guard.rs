use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Date32Array, Int64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    EmptyBehavior, METRIC_STATE_VERSION, MetricDefinition, MetricStateContract, MetricStateKind,
    Presence, Relation, RelationSemantics,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::Engine;
use semantic_plan::typed::*;

#[tokio::test]
async fn unsupported_snapshot_balance_cannot_run_through_legacy_sum_lowering() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Int64, false),
        Field::new("as_of", DataType::Date32, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(Int64Array::from(vec![10, 20])),
            Arc::new(Date32Array::from(vec![1, 2])),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("facts", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [(
            "mean_amount".into(),
            MetricDefinition {
                id: "metrics/mean-amount".into(),
                description: "Mean amount".into(),
                aliases: vec![],
                function: AggregateFunction::Sum,
                field: Some("amount".into()),
                distinct: false,
                source_grain: semantic_catalog::SourceGrain {
                    entity: None,
                    keys: vec![semantic_catalog::GrainKey {
                        relation: "facts".into(),
                        field: "id".into(),
                    }],
                },
                compatible_dimensions: Default::default(),
                compatible_lookup_dimensions: vec![],
                sum_rollup_dimensions: None,
                state: Some(MetricStateContract {
                    version: METRIC_STATE_VERSION,
                    state: MetricStateKind::SnapshotBalance {
                        time_field: "as_of".into(),
                        tie_break_fields: vec!["id".into()],
                    },
                    merge_dimensions: Default::default(),
                }),
                row_filters: vec![],
                result_type: DataType::Int64,
                unit: Presence::Missing,
                temporal: Presence::Missing,
                empty_behavior: EmptyBehavior::Null,
                source_refs: vec![],
            },
        )]
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
    let query = RowQuery {
        version: 1,
        input: RelationInput {
            relation: "facts".into(),
            instance: "r".into(),
        },
        requirements: vec![Requirement {
            id: "mean".into(),
            source_text: "mean amount".into(),
            operation: RowOperation::Metric {
                name: "metrics/mean-amount".into(),
                alias: "mean".into(),
                applicability: MetricApplicability::default(),
            },
        }],
        unresolved: vec![],
    };
    assert!(
        matches!(compile_rows(&engine, query, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "unsupported_metric_state")
    );
}
