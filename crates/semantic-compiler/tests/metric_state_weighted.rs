use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Decimal128Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    EmptyBehavior, METRIC_STATE_VERSION, MetricDefinition, MetricStateContract, MetricStateKind,
    Presence, Relation, RelationSemantics, ZeroWeight,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

#[derive(Clone, Copy)]
enum Case {
    Valid,
    NegativeWeight,
    Overflow,
}

fn metric(id: &str, zero: ZeroWeight) -> MetricDefinition {
    MetricDefinition {
        id: format!("metrics/{id}"),
        description: "Exact weighted amount".into(),
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
            state: MetricStateKind::WeightedAverage {
                weight_field: "weight".into(),
                zero,
            },
            merge_dimensions: ["region".into()].into(),
        }),
        row_filters: vec![],
        result_type: DataType::Decimal128(38, 18),
        unit: Presence::Value(semantic_catalog::Unit::Named {
            id: "points".into(),
        }),
        temporal: Presence::Missing,
        empty_behavior: if zero == ZeroWeight::Zero {
            EmptyBehavior::Zero
        } else {
            EmptyBehavior::Null
        },
        source_refs: vec![],
    }
}

fn fixture(case: Case) -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, true),
        Field::new("weight", DataType::Int64, true),
    ]));
    let (amounts, weights) = match case {
        Case::Valid => (
            vec![Some(10), Some(20), Some(30), Some(100), None, Some(5)],
            vec![Some(1), Some(3), Some(0), Some(2), Some(7), Some(0)],
        ),
        Case::NegativeWeight => (
            vec![Some(10), Some(20), Some(30), Some(100), None, Some(5)],
            vec![Some(1), Some(-3), Some(0), Some(2), Some(7), Some(0)],
        ),
        Case::Overflow => (
            vec![
                Some(i64::MAX),
                Some(i64::MAX),
                Some(i64::MAX),
                Some(100),
                None,
                Some(5),
            ],
            vec![
                Some(i64::MAX),
                Some(i64::MAX),
                Some(i64::MAX),
                Some(2),
                Some(7),
                Some(0),
            ],
        ),
    };
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5, 6])) as ArrayRef,
            Arc::new(StringArray::from(vec!["A", "A", "A", "B", "C", "D"])),
            Arc::new(Int64Array::from(amounts)),
            Arc::new(Int64Array::from(weights)),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("facts", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [
            (
                "weighted_null".into(),
                metric("weighted-null", ZeroWeight::Null),
            ),
            (
                "weighted_zero".into(),
                metric("weighted-zero", ZeroWeight::Zero),
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

fn query(metric: &str) -> RowQuery {
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
                id: "weighted".into(),
                source_text: "weighted amount".into(),
                operation: RowOperation::Metric {
                    name: metric.into(),
                    alias: "weighted".into(),
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
        let regions = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let means = batch
            .column(1)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        for index in 0..batch.num_rows() {
            rows.insert(
                regions.value(index).into(),
                (!means.is_null(index)).then(|| means.value(index)),
            );
        }
    }
    rows
}

#[tokio::test]
async fn weighted_metric_merges_components_with_sql_direct_parity_and_zero_contract() {
    let engine = fixture(Case::Valid);
    for (name, empty) in [
        ("metrics/weighted-null", None),
        ("metrics/weighted-zero", Some(0)),
    ] {
        let result = compile_rows(&engine, query(name), CompileOptions::default()).await;
        assert!(
            result
                .record
                .definition_refs
                .iter()
                .any(|r| r.id == "functions/semantic_weighted_mean_i64_v1")
        );
        let query = match result.outcome {
            TypedOutcome::Compiled { query } => query,
            other => panic!("{other:?}"),
        };
        assert!(
            query
                .sql()
                .statement()
                .contains("semantic_weighted_mean_i64_v1")
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
        let expected = BTreeMap::from([
            ("A".into(), Some(17_500_000_000_000_000_000)),
            ("B".into(), Some(100_000_000_000_000_000_000)),
            ("C".into(), empty),
            ("D".into(), empty),
        ]);
        assert_eq!(values(&direct), expected);
        assert_eq!(values(&sql), expected);
    }
}

#[tokio::test]
async fn weighted_metric_rejects_negative_weight_and_overflow_at_execution() {
    for case in [Case::NegativeWeight, Case::Overflow] {
        let engine = fixture(case);
        let result = compile_rows(
            &engine,
            query("metrics/weighted-null"),
            CompileOptions::default(),
        )
        .await;
        let query = match result.outcome {
            TypedOutcome::Compiled { query } => query,
            other => panic!("{other:?}"),
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
