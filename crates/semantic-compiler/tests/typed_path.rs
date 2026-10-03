use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Date32Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    FactResolution, GovernedFilter, Relation, RelationSemantics, RelationshipDefinition,
    RelationshipKey, RowPolicy,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    Comparison, Direction, FieldRef, Literal, MissingMatch, NullOrder, PathAsOf, PathHop,
    RelationInput, Requirement, RowOperation, RowQuery,
};

fn relationship(name: &str, right: &str, left_key: &str) -> RelationshipDefinition {
    RelationshipDefinition {
        ai_context: None,
        id: format!("relationships/{name}"),
        right_relation: right.into(),
        role: name.into(),
        key_pairs: vec![RelationshipKey {
            left_field: left_key.into(),
            right_field: "id".into(),
        }],
        null_keys_match: false,
        cardinality: FactResolution::Unknown,
        source_refs: vec![],
    }
}

fn table(schema: Arc<Schema>, columns: Vec<ArrayRef>) -> Arc<MemTable> {
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap())
}

fn fixture(duplicate_manager: bool) -> Engine {
    let mut engine = Engine::new();
    let orders_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("bill", DataType::Int64, true),
        Field::new("ship", DataType::Int64, true),
    ]));
    let mut orders = Relation::base("orders", orders_schema.clone(), "memory");
    orders.semantics = Some(RelationSemantics {
        relationships: [
            ("bill".into(), relationship("bill", "customers", "bill")),
            ("ship".into(), relationship("ship", "customers", "ship")),
        ]
        .into(),
        ..Default::default()
    });
    engine
        .register_table(
            orders,
            table(
                orders_schema,
                vec![
                    Arc::new(Int64Array::from(vec![1, 2, 3])),
                    Arc::new(Int64Array::from(vec![Some(10), Some(20), None])),
                    Arc::new(Int64Array::from(vec![Some(20), Some(10), Some(99)])),
                ],
            ),
        )
        .unwrap();
    let customers_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("manager", DataType::Int64, true),
    ]));
    let mut customers = Relation::base("customers", customers_schema.clone(), "memory");
    customers.semantics = Some(RelationSemantics {
        relationships: [(
            "manager".into(),
            relationship("manager", "managers", "manager"),
        )]
        .into(),
        ..Default::default()
    });
    engine
        .register_table(
            customers,
            table(
                customers_schema,
                vec![
                    Arc::new(Int64Array::from(vec![10, 20])),
                    Arc::new(Int64Array::from(vec![Some(100), Some(200)])),
                ],
            ),
        )
        .unwrap();
    let managers_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let mut ids = vec![100, 200];
    let mut labels = vec!["Alice", "Bob"];
    if duplicate_manager {
        ids.push(100);
        labels.push("Other");
    }
    engine
        .register_table(
            Relation::base("managers", managers_schema.clone(), "memory"),
            table(
                managers_schema,
                vec![
                    Arc::new(Int64Array::from(ids)),
                    Arc::new(StringArray::from(labels)),
                ],
            ),
        )
        .unwrap();
    engine
}

fn temporal_fixture(overlap: bool) -> Engine {
    let mut engine = Engine::new();
    let orders_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("bill", DataType::Int64, true),
        Field::new("as_of", DataType::Date32, false),
    ]));
    let mut orders = Relation::base("orders", orders_schema.clone(), "memory");
    orders.semantics = Some(RelationSemantics {
        relationships: [("bill".into(), relationship("bill", "customers", "bill"))].into(),
        ..Default::default()
    });
    engine
        .register_table(
            orders,
            table(
                orders_schema,
                vec![
                    Arc::new(Int64Array::from(vec![1, 2, 3])),
                    Arc::new(Int64Array::from(vec![Some(10), Some(10), Some(99)])),
                    Arc::new(Date32Array::from(vec![5, 15, 5])),
                ],
            ),
        )
        .unwrap();
    let customer_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("manager", DataType::Int64, false),
        Field::new("valid_from", DataType::Date32, false),
        Field::new("valid_to", DataType::Date32, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let mut customers = Relation::base("customers", customer_schema.clone(), "memory");
    customers.semantics = Some(RelationSemantics {
        relationships: [(
            "manager".into(),
            relationship("manager", "managers", "manager"),
        )]
        .into(),
        row_policies: vec![RowPolicy {
            id: "policies/visible-customers".into(),
            filters: vec![GovernedFilter {
                field: "visible".into(),
                operator: Comparison::Eq,
                value: Literal::Boolean(true),
            }],
            source_refs: vec![],
        }],
        ..Default::default()
    });
    engine
        .register_table(
            customers,
            table(
                customer_schema,
                vec![
                    Arc::new(Int64Array::from(vec![10, 10, 10])),
                    Arc::new(Int64Array::from(vec![100, 200, 100])),
                    Arc::new(Date32Array::from(vec![0, 10, 0])),
                    Arc::new(Date32Array::from(vec![10, 20, 20])),
                    Arc::new(BooleanArray::from(vec![true, true, overlap])),
                ],
            ),
        )
        .unwrap();
    let managers_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    engine
        .register_table(
            Relation::base("managers", managers_schema.clone(), "memory"),
            table(
                managers_schema,
                vec![
                    Arc::new(Int64Array::from(vec![100, 200])),
                    Arc::new(StringArray::from(vec!["Alice", "Bob"])),
                ],
            ),
        )
        .unwrap();
    engine
}

fn second_temporal_fixture(visible_overlap: bool) -> Engine {
    let mut engine = Engine::new();
    let orders_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("bill", DataType::Int64, false),
    ]));
    let mut orders = Relation::base("orders", orders_schema.clone(), "memory");
    orders.semantics = Some(RelationSemantics {
        relationships: [("bill".into(), relationship("bill", "customers", "bill"))].into(),
        ..Default::default()
    });
    engine
        .register_table(
            orders,
            table(
                orders_schema,
                vec![
                    Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
                    Arc::new(Int64Array::from(vec![10, 20, 30, 40])),
                ],
            ),
        )
        .unwrap();

    let customer_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("manager", DataType::Int64, false),
        Field::new("requested_at", DataType::Date32, false),
    ]));
    let mut customers = Relation::base("customers", customer_schema.clone(), "memory");
    customers.semantics = Some(RelationSemantics {
        relationships: [(
            "manager".into(),
            relationship("manager", "managers", "manager"),
        )]
        .into(),
        ..Default::default()
    });
    engine
        .register_table(
            customers,
            table(
                customer_schema,
                vec![
                    Arc::new(Int64Array::from(vec![10, 20, 30, 40])),
                    Arc::new(Int64Array::from(vec![100, 100, 200, 100])),
                    Arc::new(Date32Array::from(vec![5, 10, 25, 25])),
                ],
            ),
        )
        .unwrap();

    let manager_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("valid_from", DataType::Date32, false),
        Field::new("valid_to", DataType::Date32, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let mut managers = Relation::base("managers", manager_schema.clone(), "memory");
    managers.semantics = Some(RelationSemantics {
        row_policies: vec![RowPolicy {
            id: "policies/visible-managers".into(),
            filters: vec![GovernedFilter {
                field: "visible".into(),
                operator: Comparison::Eq,
                value: Literal::Boolean(true),
            }],
            source_refs: vec![],
        }],
        ..Default::default()
    });
    engine
        .register_table(
            managers,
            table(
                manager_schema,
                vec![
                    Arc::new(Int64Array::from(vec![100, 100, 100, 200])),
                    Arc::new(StringArray::from(vec!["Old", "New", "Hidden", "Alive"])),
                    Arc::new(Date32Array::from(vec![0, 10, 5, 20])),
                    Arc::new(Date32Array::from(vec![10, 20, 15, 30])),
                    Arc::new(BooleanArray::from(vec![true, true, visible_overlap, true])),
                ],
            ),
        )
        .unwrap();
    engine
}

fn second_temporal_query() -> RowQuery {
    let mut proposal = query("bill", MissingMatch::Null);
    let RowOperation::PathLookup { hops, .. } = &mut proposal.requirements[1].operation else {
        unreachable!()
    };
    hops[1].as_of = Some(PathAsOf {
        fact_time: "requested_at".into(),
        valid_from: "valid_from".into(),
        valid_to: "valid_to".into(),
        timezone: None,
    });
    proposal
}

fn query(role: &str, missing: MissingMatch) -> RowQuery {
    let requirement = |id: &str, operation| Requirement {
        id: id.into(),
        source_text: id.into(),
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
                "id",
                RowOperation::Project {
                    field: FieldRef {
                        instance: "o".into(),
                        field: "id".into(),
                    },
                    alias: "id".into(),
                },
            ),
            requirement(
                "manager_label",
                RowOperation::PathLookup {
                    hops: vec![
                        PathHop {
                            relationship: role.into(),
                            role: role.into(),
                            instance: "customer".into(),
                            as_of: None,
                        },
                        PathHop {
                            relationship: "manager".into(),
                            role: "manager".into(),
                            instance: "manager".into(),
                            as_of: None,
                        },
                    ],
                    field: "label".into(),
                    alias: "manager_label".into(),
                    missing,
                },
            ),
            requirement(
                "sort",
                RowOperation::Order {
                    field: FieldRef {
                        instance: "o".into(),
                        field: "id".into(),
                    },
                    direction: Direction::Asc,
                    nulls: NullOrder::Last,
                },
            ),
        ],
        unresolved: vec![],
    }
}

fn values(batches: &[RecordBatch]) -> Vec<Vec<String>> {
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
async fn two_hop_roles_missing_and_sql_direct_parity() {
    let engine = fixture(false);
    for (role, missing, expected) in [
        (
            "bill",
            MissingMatch::Null,
            vec![vec!["1", "Alice"], vec!["2", "Bob"], vec!["3", ""]],
        ),
        (
            "ship",
            MissingMatch::Exclude,
            vec![vec!["1", "Bob"], vec!["2", "Alice"]],
        ),
    ] {
        let result = compile_rows(&engine, query(role, missing), CompileOptions::default()).await;
        assert_eq!(result.record.execution_obligations.len(), 2);
        let query = match result.outcome {
            TypedOutcome::Compiled { query } => query,
            other => panic!("path rejected: {other:?}"),
        };
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
        assert_eq!(values(&direct), expected);
        assert_eq!(values(&sql), expected);
    }
}

#[tokio::test]
async fn second_hop_duplicate_fails_each_execution() {
    let engine = fixture(true);
    let result = compile_rows(
        &engine,
        query("bill", MissingMatch::Null),
        CompileOptions::default(),
    )
    .await;
    let query = match result.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("path rejected: {other:?}"),
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

#[tokio::test]
async fn second_temporal_hop_rejects_missing_intermediate_time_field() {
    let engine = fixture(false);
    let mut proposal = query("bill", MissingMatch::Null);
    let RowOperation::PathLookup { hops, .. } = &mut proposal.requirements[1].operation else {
        unreachable!();
    };
    hops[1].as_of = Some(PathAsOf {
        fact_time: "requested_at".into(),
        valid_from: "valid_from".into(),
        valid_to: "valid_to".into(),
        timezone: None,
    });
    assert!(matches!(
        compile_rows(&engine, proposal, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "path_lookup_contract"
    ));
}

#[tokio::test]
async fn first_temporal_hop_uses_half_open_intervals_and_policy_visible_uniqueness() {
    let engine = temporal_fixture(false);
    let mut proposal = query("bill", MissingMatch::Null);
    let RowOperation::PathLookup { hops, .. } = &mut proposal.requirements[1].operation else {
        unreachable!()
    };
    hops[0].as_of = Some(PathAsOf {
        fact_time: "as_of".into(),
        valid_from: "valid_from".into(),
        valid_to: "valid_to".into(),
        timezone: None,
    });
    let result = compile_rows(&engine, proposal.clone(), CompileOptions::default()).await;
    assert_eq!(result.record.execution_obligations.len(), 2);
    let query = match result.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("temporal path rejected: {other:?}"),
    };
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
    let expected = vec![vec!["1", "Alice"], vec!["2", "Bob"], vec!["3", ""]];
    assert_eq!(values(&direct), expected);
    assert_eq!(values(&sql), expected);
    let duplicate = temporal_fixture(true);
    let compiled = compile_rows(&duplicate, proposal, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = compiled.outcome else {
        panic!("duplicate fixture did not bind")
    };
    assert!(
        query
            .plan_direct(&duplicate)
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
    assert!(
        query
            .execute(&duplicate, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
}

#[tokio::test]
async fn second_temporal_hop_projects_intermediate_time_with_half_open_policy_checked_match() {
    let engine = second_temporal_fixture(false);
    let proposal = second_temporal_query();
    let result = compile_rows(&engine, proposal.clone(), CompileOptions::default()).await;
    let query = match result.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("second temporal path rejected: {other:?}"),
    };
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
    let expected = vec![
        vec!["1", "Old"],
        vec!["2", "New"],
        vec!["3", "Alive"],
        vec!["4", ""],
    ];
    assert_eq!(values(&direct), expected);
    assert_eq!(values(&sql), expected);

    let overlap = second_temporal_fixture(true);
    let result = compile_rows(&overlap, proposal, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("overlap fixture did not bind")
    };
    assert!(
        query
            .plan_direct(&overlap)
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
    assert!(
        query
            .execute(&overlap, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
}
