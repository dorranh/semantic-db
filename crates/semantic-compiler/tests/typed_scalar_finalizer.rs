use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{Array, Decimal128Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    EmptyBehavior, GrainKey, METRIC_STATE_VERSION, MetricDefinition, MetricStateContract,
    MetricStateKind, Presence, Relation, RelationSemantics, SourceGrain, Unit, ZeroWeight,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

fn metric(id: &str, zero: ZeroWeight) -> MetricDefinition {
    MetricDefinition {
        id: format!("metrics/{id}"),
        description: "Checked exact weighted mean".into(),
        aliases: vec![],
        function: AggregateFunction::Sum,
        field: Some("amount".into()),
        distinct: false,
        source_grain: SourceGrain {
            entity: None,
            keys: vec![GrainKey {
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
        unit: Presence::Value(Unit::Named {
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

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, true),
        Field::new("weight", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])) as _,
            Arc::new(StringArray::from(vec!["A", "A", "B", "C"])) as _,
            Arc::new(Int64Array::from(vec![Some(10), Some(20), Some(5), None])) as _,
            Arc::new(Int64Array::from(vec![Some(1), Some(3), Some(0), Some(1)])) as _,
        ],
    )
    .unwrap();
    let mut relation = Relation::base("facts", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [
            ("mean_zero".into(), metric("mean-zero", ZeroWeight::Zero)),
            ("mean_null".into(), metric("mean-null", ZeroWeight::Null)),
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

fn query(name: &str) -> RowQuery {
    RowQuery {
        version: ROW_QUERY_VERSION,
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
                source_text: "weighted mean".into(),
                operation: RowOperation::Metric {
                    name: name.into(),
                    alias: "mean".into(),
                    applicability: MetricApplicability::default(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> BTreeMap<String, Option<i128>> {
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
        for row in 0..batch.num_rows() {
            rows.insert(
                regions.value(row).into(),
                (!means.is_null(row)).then(|| means.value(row)),
            );
        }
    }
    rows
}

#[tokio::test]
async fn checked_decimal_zero_finalizer_preserves_exact_sql_direct_and_null_policy() {
    let engine = fixture();
    for (name, empty) in [("metrics/mean-zero", Some(0)), ("metrics/mean-null", None)] {
        let result = compile_rows(&engine, query(name), CompileOptions::default()).await;
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("expected compiled weighted mean: {:?}", result.outcome);
        };
        assert_eq!(
            query
                .sql()
                .statement()
                .to_ascii_lowercase()
                .contains("coalesce"),
            empty.is_some()
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
            ("B".into(), empty),
            ("C".into(), empty),
        ]);
        assert_eq!(rows(&direct), expected);
        assert_eq!(rows(&sql), expected);
    }
}
