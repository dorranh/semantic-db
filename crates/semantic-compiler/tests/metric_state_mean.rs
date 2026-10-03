use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{ArrayRef, Decimal128Array, Int64Array, StringArray},
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
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

fn engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])) as ArrayRef,
            Arc::new(StringArray::from(vec!["A", "A", "A", "B", "C"])) as ArrayRef,
            Arc::new(Int64Array::from(vec![
                Some(10),
                Some(20),
                Some(30),
                Some(100),
                None,
            ])) as ArrayRef,
        ],
    )
    .unwrap();
    let mut relation = Relation::base("facts", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [(
            "mean_amount".into(),
            MetricDefinition {
                id: "metrics/mean-amount".into(),
                description: "Exact mean amount".into(),
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
                compatible_dimensions: ["region".into()].into(),
                compatible_lookup_dimensions: vec![],
                sum_rollup_dimensions: None,
                state: Some(MetricStateContract {
                    version: METRIC_STATE_VERSION,
                    state: MetricStateKind::SumCountAverage,
                    merge_dimensions: ["region".into()].into(),
                }),
                row_filters: vec![],
                result_type: DataType::Decimal128(38, 18),
                unit: Presence::Value(semantic_catalog::Unit::Named {
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
    let mut engine = Engine::new();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "facts".into(),
            instance: "f".into(),
        },
        requirements: vec![
            Requirement {
                id: "region".into(),
                source_text: "by region".into(),
                operation: RowOperation::Group {
                    field: FieldRef {
                        instance: "f".into(),
                        field: "region".into(),
                    },
                    alias: "region".into(),
                },
            },
            Requirement {
                id: "mean".into(),
                source_text: "mean amount".into(),
                operation: RowOperation::Metric {
                    name: "metrics/mean-amount".into(),
                    alias: "mean".into(),
                    applicability: MetricApplicability::default(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn values(batches: &[RecordBatch]) -> BTreeMap<String, Option<i128>> {
    let mut rows = BTreeMap::new();
    for batch in batches {
        let names = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let means = batch
            .column(1)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        for row in 0..batch.num_rows() {
            use datafusion::arrow::array::Array;
            rows.insert(
                names.value(row).into(),
                (!means.is_null(row)).then(|| means.value(row)),
            );
        }
    }
    rows
}

#[tokio::test]
async fn authored_sum_count_mean_executes_exactly_in_sql_and_direct_paths() {
    let engine = engine();
    let result = compile_rows(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome);
    };
    let expected = BTreeMap::from([
        ("A".into(), Some(20_000_000_000_000_000_000)),
        ("B".into(), Some(100_000_000_000_000_000_000)),
        ("C".into(), None),
    ]);
    let direct = query
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let emitted = query
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(values(&direct), expected);
    assert_eq!(values(&emitted), expected);
    assert!(query.sql().statement().contains("semantic_mean_i64_v1"));
}
