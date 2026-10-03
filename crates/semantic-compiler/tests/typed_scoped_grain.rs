use std::sync::Arc;

use datafusion::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    EmptyBehavior, GrainKey, MetricDefinition, Presence, Relation, RelationSemantics, SourceGrain,
    Unit,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    AggregateFunction, MetricApplicability, RelationInput, Requirement, RowOperation, RowQuery,
};

fn fixture() -> Engine {
    let mut engine = Engine::new();
    for relation_name in ["scores", "other"] {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("score", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(Int64Array::from(vec![10, 20])),
            ],
        )
        .unwrap();
        let mut relation = Relation::base(relation_name, schema.clone(), "memory");
        relation.semantics = Some(RelationSemantics {
            metrics: [(
                "total".into(),
                MetricDefinition {
                    id: format!("metrics/{relation_name}-total"),
                    description: "Total score".into(),
                    aliases: vec![],
                    function: AggregateFunction::Sum,
                    field: Some("score".into()),
                    distinct: false,
                    source_grain: semantic_catalog::SourceGrain {
                        entity: None,
                        keys: vec![semantic_catalog::GrainKey {
                            relation: relation_name.into(),
                            field: "id".into(),
                        }],
                    },
                    compatible_dimensions: Default::default(),
                    compatible_lookup_dimensions: vec![],
                    sum_rollup_dimensions: None,
                    state: None,
                    row_filters: vec![],
                    result_type: DataType::Int64,
                    unit: Presence::Value(Unit::Named {
                        id: "points".into(),
                    }),
                    temporal: Presence::Missing,
                    empty_behavior: EmptyBehavior::Null,
                    source_refs: vec![],
                },
            )]
            .into(),
            ..Default::default()
        });
        engine
            .register_table(
                relation,
                Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
            )
            .unwrap();
    }
    engine
}

fn query(grain_relation: &str) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "scores".into(),
            instance: "s".into(),
        },
        requirements: vec![Requirement {
            id: "score".into(),
            source_text: "metrics/scores-total".into(),
            operation: RowOperation::Metric {
                name: "metrics/scores-total".into(),
                alias: "score".into(),
                applicability: MetricApplicability {
                    required_unit: Some(Unit::Named {
                        id: "points".into(),
                    }),
                    required_source_grain: Some(SourceGrain {
                        entity: None,
                        keys: vec![GrainKey {
                            relation: grain_relation.into(),
                            field: "id".into(),
                        }],
                    }),
                },
            },
        }],
        unresolved: vec![],
    }
}

#[tokio::test]
async fn scoped_grain_distinguishes_identical_field_names_across_relations() {
    let engine = fixture();
    let accepted = compile_rows(&engine, query("scores"), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: compiled } = accepted.outcome else {
        panic!("correctly scoped grain rejected: {:?}", accepted.outcome)
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
    for batches in [&direct, &sql] {
        assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
        assert_eq!(
            array_value_to_string(batches[0].column(0), 0).unwrap(),
            "30"
        );
    }
    let rejected = compile_rows(&engine, query("other"), CompileOptions::default()).await;
    assert!(matches!(
        rejected.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_grain"
    ));
}
