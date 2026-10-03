//! Architecture §12.4: two authored roles over one relation retain distinct instances.

use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    FactResolution, Relation, RelationSemantics, RelationshipDefinition, RelationshipKey,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    Direction, FieldRef, LookupUsage, MissingMatch, NullOrder, RelationInput, Requirement,
    RowOperation, RowQuery,
};

fn fixture(include_shipping: bool) -> Engine {
    let mut engine = Engine::new();
    let customer_schema = Arc::new(Schema::new(vec![
        Field::new("customer_id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
    ]));
    let customer_batch = RecordBatch::try_new(
        customer_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![10, 20, 30])),
            Arc::new(StringArray::from(vec!["North", "South", "West"])),
        ],
    )
    .unwrap();
    engine
        .register_table(
            Relation::base("customers", customer_schema.clone(), "memory"),
            Arc::new(MemTable::try_new(customer_schema, vec![vec![customer_batch]]).unwrap()),
        )
        .unwrap();

    let order_schema = Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("billing_customer_id", DataType::Int64, false),
        Field::new("shipping_customer_id", DataType::Int64, false),
    ]));
    let order_batch = RecordBatch::try_new(
        order_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Int64Array::from(vec![10, 20, 20])),
            Arc::new(Int64Array::from(vec![20, 10, 30])),
        ],
    )
    .unwrap();
    let relationship = |name: &str, key: &str| RelationshipDefinition {
        ai_context: None,
        id: format!("relationships/{name}"),
        right_relation: "customers".into(),
        role: name.into(),
        key_pairs: vec![RelationshipKey {
            left_field: key.into(),
            right_field: "customer_id".into(),
        }],
        null_keys_match: false,
        cardinality: FactResolution::Unknown,
        source_refs: vec![],
    };
    let mut relationships = [(
        "billing_customer".into(),
        relationship("billing_customer", "billing_customer_id"),
    )]
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    if include_shipping {
        relationships.insert(
            "shipping_customer".into(),
            relationship("shipping_customer", "shipping_customer_id"),
        );
    }
    let mut orders = Relation::base("orders", order_schema.clone(), "memory");
    orders.semantics = Some(RelationSemantics {
        relationships,
        ..Default::default()
    });
    engine
        .register_table(
            orders,
            Arc::new(MemTable::try_new(order_schema, vec![vec![order_batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn proposal() -> RowQuery {
    let requirement = |id: &str, operation| Requirement {
        id: id.into(),
        source_text: id.replace('_', " "),
        operation,
    };
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "orders".into(),
            instance: "o".into(),
        },
        requirements: vec![
            requirement(
                "order_id",
                RowOperation::Project {
                    field: FieldRef {
                        instance: "o".into(),
                        field: "order_id".into(),
                    },
                    alias: "order_id".into(),
                },
            ),
            requirement(
                "billing_region",
                RowOperation::Lookup {
                    relationship: "billing_customer".into(),
                    role: "billing_customer".into(),
                    instance: "c_billing".into(),
                    field: "region".into(),
                    alias: "billing_region".into(),
                    missing: MissingMatch::Null,
                    usage: LookupUsage::Project,
                },
            ),
            requirement(
                "shipping_region",
                RowOperation::Lookup {
                    relationship: "shipping_customer".into(),
                    role: "shipping_customer".into(),
                    instance: "c_shipping".into(),
                    field: "region".into(),
                    alias: "shipping_region".into(),
                    missing: MissingMatch::Null,
                    usage: LookupUsage::Project,
                },
            ),
            requirement(
                "sort",
                RowOperation::Order {
                    field: FieldRef {
                        instance: "o".into(),
                        field: "order_id".into(),
                    },
                    direction: Direction::Asc,
                    nulls: NullOrder::Last,
                },
            ),
        ],
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| {
                batch
                    .columns()
                    .iter()
                    .map(|column| array_value_to_string(column, row).unwrap())
                    .collect()
            })
        })
        .collect()
}

#[tokio::test]
async fn billing_and_shipping_roles_keep_separate_instances_and_exact_rows() {
    let engine = fixture(true);
    let result = compile_rows(&engine, proposal(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("two-role proposal rejected: {:?}", result.outcome)
    };
    let expected: Vec<Vec<String>> = vec![
        vec!["1".into(), "North".into(), "South".into()],
        vec!["2".into(), "South".into(), "North".into()],
        vec!["3".into(), "South".into(), "West".into()],
    ];
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
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
    assert_eq!(result.record.execution_obligations.len(), 2);
}

#[tokio::test]
async fn unavailable_shipping_role_never_reuses_billing_relationship() {
    let engine = fixture(false);
    let result = compile_rows(&engine, proposal(), CompileOptions::default()).await;
    let TypedOutcome::Rejected { diagnostic } = result.outcome else {
        panic!(
            "missing shipping relationship was accepted: {:?}",
            result.outcome
        )
    };
    assert_eq!(diagnostic.code, "unknown_relationship");
    assert_eq!(
        diagnostic.details.requirement_ref.as_deref(),
        Some("shipping_region")
    );
    assert_eq!(
        diagnostic.details.object_ref.as_deref(),
        Some("relationship/orders/shipping_customer")
    );
    assert!(result.record.execution_obligations.is_empty());
}
