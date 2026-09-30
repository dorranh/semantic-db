use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, BooleanArray, Int64Array, StringArray, TimestampMicrosecondArray,
        },
        datatypes::{DataType, Field, Schema, TimeUnit},
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

fn fixture(conflict: bool) -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("row_id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("balance", DataType::Int64, true),
        Field::new(
            "observed_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("tie_key", DataType::Int64, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let batch = |ids: Vec<i64>,
                 regions: Vec<&str>,
                 values: Vec<Option<i64>>,
                 times: Vec<i64>,
                 ties: Vec<i64>,
                 visible: Vec<bool>| {
        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(ids)) as ArrayRef,
                Arc::new(StringArray::from(regions)),
                Arc::new(Int64Array::from(values)),
                Arc::new(TimestampMicrosecondArray::from(times).with_timezone("UTC")),
                Arc::new(Int64Array::from(ties)),
                Arc::new(BooleanArray::from(visible)),
            ],
        )
        .unwrap()
    };
    let first = batch(
        vec![1, 2, 3],
        vec!["A", "A", "B"],
        vec![Some(100), Some(200), None],
        vec![10, 20, 5],
        vec![1, 1, 1],
        vec![true, true, true],
    );
    let mut second = vec![batch(
        vec![4, 5, 6],
        vec!["A", "C", "A"],
        vec![Some(50), Some(7), Some(999)],
        vec![20, 30, 40],
        vec![2, 1, 1],
        vec![true, true, false],
    )];
    if conflict {
        second.push(batch(
            vec![7],
            vec!["A"],
            vec![Some(51)],
            vec![20],
            vec![2],
            vec![true],
        ));
    }
    let metric = MetricDefinition {
        id: "metrics/latest-balance".into(),
        description: "Latest authored balance".into(),
        aliases: vec![],
        function: AggregateFunction::Max,
        field: Some("balance".into()),
        distinct: false,
        source_grain: semantic_catalog::SourceGrain {
            entity: None,
            keys: vec![semantic_catalog::GrainKey {
                relation: "balances".into(),
                field: "row_id".into(),
            }],
        },
        compatible_dimensions: ["region".into()].into(),
        compatible_lookup_dimensions: vec![],
        sum_rollup_dimensions: None,
        state: Some(MetricStateContract {
            version: METRIC_STATE_VERSION,
            state: MetricStateKind::SnapshotBalance {
                time_field: "observed_at".into(),
                tie_break_fields: vec!["tie_key".into()],
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
            id: "minor_currency_unit".into(),
        }),
        temporal: Presence::Missing,
        empty_behavior: EmptyBehavior::Null,
        source_refs: vec![],
    };
    let mut relation = Relation::base("balances", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [("latest_balance".into(), metric)].into(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema.clone(), vec![vec![first], second]).unwrap()),
        )
        .unwrap();
    engine
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "balances".into(),
            instance: "b".into(),
        },
        requirements: vec![
            Requirement {
                id: "region".into(),
                source_text: "by region".into(),
                operation: RowOperation::Group {
                    field: FieldRef {
                        instance: "b".into(),
                        field: "region".into(),
                    },
                    alias: "region".into(),
                },
            },
            Requirement {
                id: "balance".into(),
                source_text: "latest balance".into(),
                operation: RowOperation::Metric {
                    name: "metrics/latest-balance".into(),
                    alias: "balance".into(),
                    applicability: MetricApplicability::default(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn values(batches: &[RecordBatch]) -> BTreeMap<String, Option<i64>> {
    let mut result = BTreeMap::new();
    for batch in batches {
        let regions = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let balances = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        for index in 0..batch.num_rows() {
            result.insert(
                regions.value(index).into(),
                balances.is_valid(index).then(|| balances.value(index)),
            );
        }
    }
    result
}

#[tokio::test]
async fn snapshot_balance_selects_latest_policy_visible_value_without_summing_dates() {
    let engine = fixture(false);
    let compilation = compile_rows(&engine, query(), CompileOptions::default()).await;
    assert!(
        compilation
            .record
            .definition_refs
            .iter()
            .any(|r| r.id == "functions/semantic_snapshot_balance_i64_v1")
    );
    let query = match compilation.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("{other:?}"),
    };
    assert!(
        query
            .sql()
            .statement()
            .contains("semantic_snapshot_balance_i64_v1")
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
        ("A".into(), Some(50)),
        ("B".into(), None),
        ("C".into(), Some(7)),
    ]);
    assert_eq!(values(&direct), expected);
    assert_eq!(values(&sql), expected);
}

#[tokio::test]
async fn snapshot_balance_rejects_equal_time_and_tie_conflicts() {
    let engine = fixture(true);
    let compilation = compile_rows(&engine, query(), CompileOptions::default()).await;
    let query = match compilation.outcome {
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
