//! Architecture §12.3: an order amount cannot be copied into item categories.

use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    EmptyBehavior, FactResolution, GrainKey, MetricDefinition, MetricLookupDimension, Presence,
    Relation, RelationSemantics, RelationshipDefinition, RelationshipKey, SourceGrain,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    AggregateFunction, LookupUsage, MetricApplicability, MissingMatch, RelationInput, Requirement,
    RowOperation, RowQuery,
};

fn fixture(authorize_dimension: bool) -> Engine {
    let mut engine = Engine::new();
    let item_schema = Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("category", DataType::Utf8, false),
    ]));
    let items = RecordBatch::try_new(
        item_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 1, 2])),
            Arc::new(StringArray::from(vec!["A", "B", "A"])),
        ],
    )
    .unwrap();
    engine
        .register_table(
            Relation::base("order_items", item_schema.clone(), "memory"),
            Arc::new(MemTable::try_new(item_schema, vec![vec![items]]).unwrap()),
        )
        .unwrap();

    let order_schema = Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("net_amount", DataType::Int64, false),
    ]));
    // Equal amounts on distinct orders make SUM(DISTINCT net_amount) incorrect.
    let orders = RecordBatch::try_new(
        order_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(Int64Array::from(vec![100, 100])),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("orders", order_schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        declared_primary_key: vec!["order_id".into()],
        metrics: [(
            "net_revenue".into(),
            MetricDefinition {
                id: "metrics/net-revenue".into(),
                description: "Order-level net amount".into(),
                aliases: vec![],
                function: AggregateFunction::Sum,
                field: Some("net_amount".into()),
                distinct: false,
                source_grain: SourceGrain {
                    entity: None,
                    keys: vec![GrainKey {
                        relation: "orders".into(),
                        field: "order_id".into(),
                    }],
                },
                compatible_dimensions: Default::default(),
                compatible_lookup_dimensions: authorize_dimension
                    .then_some(MetricLookupDimension {
                        relationship: "items".into(),
                        field: "category".into(),
                        missing: MissingMatch::Null,
                    })
                    .into_iter()
                    .collect(),
                sum_rollup_dimensions: None,
                state: None,
                row_filters: vec![],
                result_type: DataType::Int64,
                unit: Presence::Missing,
                temporal: Presence::Missing,
                empty_behavior: EmptyBehavior::Null,
                source_refs: vec![],
            },
        )]
        .into(),
        relationships: [(
            "items".into(),
            RelationshipDefinition {
                ai_context: None,
                id: "relationships/order-items".into(),
                right_relation: "order_items".into(),
                role: "items".into(),
                key_pairs: vec![RelationshipKey {
                    left_field: "order_id".into(),
                    right_field: "order_id".into(),
                }],
                null_keys_match: false,
                cardinality: FactResolution::Unknown,
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(order_schema, vec![vec![orders]]).unwrap()),
        )
        .unwrap();
    engine
}

fn proposal() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "orders".into(),
            instance: "o".into(),
        },
        requirements: vec![
            Requirement {
                id: "category".into(),
                source_text: "by item category".into(),
                operation: RowOperation::Lookup {
                    relationship: "items".into(),
                    role: "items".into(),
                    instance: "i".into(),
                    field: "category".into(),
                    alias: "category".into(),
                    missing: MissingMatch::Null,
                    usage: LookupUsage::Group,
                },
            },
            Requirement {
                id: "revenue".into(),
                source_text: "net revenue".into(),
                operation: RowOperation::Metric {
                    name: "net_revenue".into(),
                    alias: "revenue".into(),
                    applicability: MetricApplicability::default(),
                },
            },
        ],
        unresolved: vec![],
    }
}

#[tokio::test]
async fn unapproved_item_category_is_rejected_before_lowering() {
    let result = compile_rows(&fixture(false), proposal(), CompileOptions::default()).await;
    assert!(matches!(
        result.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_dimensions"
    ));
}

#[tokio::test]
async fn a_dimension_whitelist_cannot_turn_a_one_to_many_join_into_an_allocation() {
    let engine = fixture(true);
    let result = compile_rows(&engine, proposal(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!(
            "guarded lookup rejected before execution: {:?}",
            result.outcome
        )
    };
    assert_eq!(result.record.execution_obligations.len(), 1);
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
