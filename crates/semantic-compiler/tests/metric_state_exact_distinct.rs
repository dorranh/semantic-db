use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, BooleanArray, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    EmptyBehavior, GovernedFilter, METRIC_STATE_VERSION, MetricDefinition, MetricStateContract,
    MetricStateKind, Presence, Relation, RelationSemantics,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("row_id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("person_id", DataType::Int64, true),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let batch =
        |rows: Vec<i64>, regions: Vec<&str>, people: Vec<Option<i64>>, visible: Vec<bool>| {
            RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(Int64Array::from(rows)) as ArrayRef,
                    Arc::new(StringArray::from(regions)),
                    Arc::new(Int64Array::from(people)),
                    Arc::new(BooleanArray::from(visible)),
                ],
            )
            .unwrap()
        };
    let first = batch(
        vec![1, 2, 3, 4],
        vec!["A", "A", "A", "B"],
        vec![Some(1), Some(2), Some(1), Some(2)],
        vec![true, true, true, true],
    );
    let second = batch(
        vec![5, 6, 7, 8],
        vec!["A", "B", "C", "A"],
        vec![Some(2), Some(3), None, Some(4)],
        vec![true, true, true, false],
    );
    let metric = MetricDefinition {
        id: "metrics/unique-people".into(),
        description: "Exact unique visible people".into(),
        aliases: vec![],
        function: AggregateFunction::Count,
        field: Some("person_id".into()),
        distinct: false,
        source_grain: semantic_catalog::SourceGrain {
            entity: None,
            keys: vec![semantic_catalog::GrainKey {
                relation: "events".into(),
                field: "row_id".into(),
            }],
        },
        compatible_dimensions: ["region".into()].into(),
        compatible_lookup_dimensions: vec![],
        sum_rollup_dimensions: None,
        state: Some(MetricStateContract {
            version: METRIC_STATE_VERSION,
            state: MetricStateKind::ExactDistinct {
                identity_fields: vec!["person_id".into()],
            },
            merge_dimensions: ["region".into()].into(),
        }),
        row_filters: vec![GovernedFilter {
            field: "visible".into(),
            operator: Comparison::Eq,
            value: Literal::Boolean(true),
        }],
        result_type: DataType::Int64,
        unit: Presence::Value(semantic_catalog::Unit::Named {
            id: "people".into(),
        }),
        temporal: Presence::Missing,
        empty_behavior: EmptyBehavior::Zero,
        source_refs: vec![],
    };
    let mut relation = Relation::base("events", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [("unique_people".into(), metric)].into(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema.clone(), vec![vec![first], vec![second]]).unwrap()),
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
        requirements: vec![
            Requirement {
                id: "region".into(),
                source_text: "by region".into(),
                operation: RowOperation::Group {
                    field: FieldRef {
                        instance: "e".into(),
                        field: "region".into(),
                    },
                    alias: "region".into(),
                },
            },
            Requirement {
                id: "unique".into(),
                source_text: "unique visible people".into(),
                operation: RowOperation::Metric {
                    name: "metrics/unique-people".into(),
                    alias: "unique_people".into(),
                    applicability: MetricApplicability::default(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn values(batches: &[RecordBatch]) -> BTreeMap<String, i64> {
    let mut rows = BTreeMap::new();
    for batch in batches {
        let regions = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let counts = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        for index in 0..batch.num_rows() {
            assert!(!counts.is_null(index));
            rows.insert(regions.value(index).into(), counts.value(index));
        }
    }
    rows
}

#[tokio::test]
async fn exact_distinct_metric_unions_identities_after_governed_filter_across_partitions() {
    let engine = fixture();
    let compilation = compile_rows(&engine, query(), CompileOptions::default()).await;
    assert!(
        compilation
            .record
            .definition_refs
            .iter()
            .any(|r| r.id == "functions/semantic_exact_count_i64_v1")
    );
    let query = match compilation.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("{other:?}"),
    };
    assert!(
        query
            .sql()
            .statement()
            .contains("semantic_exact_count_i64_v1")
    );
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
    let expected = BTreeMap::from([("A".into(), 2), ("B".into(), 2), ("C".into(), 0)]);
    assert_eq!(values(&direct), expected);
    assert_eq!(values(&sql), expected);
}
