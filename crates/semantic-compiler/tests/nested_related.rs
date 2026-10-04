use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    ConceptDefinition, FactResolution, Relation, RelationSemantics, RelationshipDefinition,
    RelationshipKey,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_intent, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;
use std::{collections::BTreeMap, sync::Arc};
fn relationship(
    name: &str,
    right: &str,
    left_field: &str,
    right_field: &str,
) -> RelationshipDefinition {
    RelationshipDefinition {
        id: format!("relationships/{name}"),
        right_relation: right.into(),
        role: name.into(),
        key_pairs: vec![RelationshipKey {
            left_field: left_field.into(),
            right_field: right_field.into(),
        }],
        null_keys_match: false,
        cardinality: FactResolution::Unknown,
        ai_context: None,
        source_refs: vec![],
    }
}
fn fixture() -> Engine {
    fixture_with_concept("completed")
}
fn fixture_with_concept(value: &str) -> Engine {
    fixture_profile(value, None)
}
fn fixture_profile(value: &str, policy_order: Option<i64>) -> Engine {
    let mut engine = Engine::new();
    let products = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
    let mut relation = Relation::base("products", products.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        relationships: [(
            "items".into(),
            relationship("items", "items", "id", "product"),
        )]
        .into(),
        ..Default::default()
    });
    let batch = RecordBatch::try_new(
        products.clone(),
        vec![Arc::new(Int64Array::from(vec![
            Some(1),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            None,
        ]))],
    )
    .unwrap();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(products, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    let items = Arc::new(Schema::new(vec![
        Field::new("product", DataType::Int64, true),
        Field::new("order_id", DataType::Int64, true),
        Field::new("quantity", DataType::Int64, false),
    ]));
    let mut relation = Relation::base("items", items.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        relationships: [(
            "order".into(),
            relationship("order", "orders", "order_id", "id"),
        )]
        .into(),
        ..Default::default()
    });
    let batch = RecordBatch::try_new(
        items.clone(),
        vec![
            Arc::new(Int64Array::from(vec![
                Some(1),
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                None,
            ])) as ArrayRef,
            Arc::new(Int64Array::from(vec![
                Some(10),
                Some(10),
                Some(11),
                Some(10),
                Some(99),
                None,
                Some(10),
            ])),
            Arc::new(Int64Array::from(vec![1, 2, 1, -1, 1, 1, 1])),
        ],
    )
    .unwrap();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(items, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    let orders = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("status", DataType::Utf8, false),
    ]));
    let mut relation = Relation::base("orders", orders.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        row_policies: policy_order
            .map(|value| semantic_catalog::RowPolicy {
                id: "policies/visible-order".into(),
                filters: vec![semantic_catalog::GovernedFilter {
                    field: "id".into(),
                    operator: Comparison::Eq,
                    value: Literal::Int64(value),
                }],
                source_refs: vec![],
            })
            .into_iter()
            .collect(),
        concepts: [(
            "completed".into(),
            ConceptDefinition {
                id: "concepts/completed".into(),
                description: "Completed orders".into(),
                aliases: vec![],
                alternatives: vec![],
                predicate: RowPredicate::Compare {
                    field: "status".into(),
                    operator: Comparison::Eq,
                    value: Literal::Utf8(value.into()),
                },
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    let batch = RecordBatch::try_new(
        orders.clone(),
        vec![
            Arc::new(Int64Array::from(vec![10, 11])) as ArrayRef,
            Arc::new(StringArray::from(vec!["completed", "pending"])),
        ],
    )
    .unwrap();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(orders, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}
fn requirement(id: &str, operation: RowOperation) -> Requirement {
    Requirement {
        id: id.into(),
        source_text: id.into(),
        operation,
    }
}
fn query(mode: ExistenceMode) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "products".into(),
            instance: "p".into(),
        },
        requirements: vec![
            requirement(
                "id",
                RowOperation::Project {
                    field: FieldRef {
                        instance: "p".into(),
                        field: "id".into(),
                    },
                    alias: "id".into(),
                },
            ),
            requirement(
                "qualified",
                RowOperation::Related {
                    relationship: "items".into(),
                    role: "items".into(),
                    instance: "i".into(),
                    mode,
                    predicate: None,
                    target_requirements: vec![
                        requirement(
                            "positive",
                            RowOperation::Filter {
                                predicate: RowPredicate::Compare {
                                    field: FieldRef {
                                        instance: "i".into(),
                                        field: "quantity".into(),
                                    },
                                    operator: Comparison::Gt,
                                    value: Literal::Int64(0),
                                },
                            },
                        ),
                        requirement(
                            "completed_order",
                            RowOperation::Related {
                                relationship: "order".into(),
                                role: "order".into(),
                                instance: "o".into(),
                                mode: ExistenceMode::Exists,
                                predicate: None,
                                target_requirements: vec![requirement(
                                    "completed",
                                    RowOperation::ConceptFilter {
                                        concept: "completed".into(),
                                        arguments: BTreeMap::new(),
                                    },
                                )],
                            },
                        ),
                    ],
                },
            ),
        ],
        unresolved: vec![],
    }
}
fn values(batches: &[RecordBatch]) -> Vec<String> {
    let mut rows = batches
        .iter()
        .flat_map(|b| {
            (0..b.num_rows())
                .map(|i| array_value_to_string(b.column(0).as_ref(), i).unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}
#[tokio::test]
async fn nested_concepts_preserve_bags_orphans_nulls_and_same_target_conjunction() {
    let engine = fixture();
    for (mode, expected, sql) in [
        (
            ExistenceMode::Exists,
            vec!["1", "1"],
            "SELECT p.id FROM products p WHERE EXISTS (SELECT 1 FROM items i WHERE i.product=p.id AND i.quantity>0 AND EXISTS (SELECT 1 FROM orders o WHERE o.id=i.order_id AND o.status='completed'))",
        ),
        (
            ExistenceMode::Absent,
            vec!["", "2", "3", "4", "5"],
            "SELECT p.id FROM products p WHERE NOT EXISTS (SELECT 1 FROM items i WHERE i.product=p.id AND i.quantity>0 AND EXISTS (SELECT 1 FROM orders o WHERE o.id=i.order_id AND o.status='completed'))",
        ),
    ] {
        let result = compile_rows(&engine, query(mode), CompileOptions::default()).await;
        assert!(
            result
                .record
                .definition_refs
                .iter()
                .any(|r| r.id == "concepts/completed")
        );
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
        let direct = query
            .plan_direct(&engine)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        let generated = query
            .execute(&engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        let reference = engine.query(sql).await.unwrap();
        assert_eq!(values(&direct), expected);
        assert_eq!(values(&generated), values(&reference));
        assert_eq!(values(&direct), values(&reference));
    }
}
#[tokio::test]
async fn nested_target_rejects_forbidden_operations_duplicates_scope_and_missing_concepts() {
    let engine = fixture();
    for mutation in 0..5 {
        let mut q = query(ExistenceMode::Exists);
        let RowOperation::Related {
            target_requirements,
            ..
        } = &mut q.requirements[1].operation
        else {
            unreachable!()
        };
        match mutation {
            0 => {
                target_requirements[0].operation = RowOperation::Project {
                    field: FieldRef {
                        instance: "i".into(),
                        field: "quantity".into(),
                    },
                    alias: "q".into(),
                }
            }
            1 => target_requirements[0].id = "id".into(),
            2 => {
                let RowOperation::Related { instance, .. } = &mut target_requirements[1].operation
                else {
                    unreachable!()
                };
                *instance = "p".into()
            }
            3 => {
                let RowOperation::Related {
                    target_requirements,
                    ..
                } = &mut target_requirements[1].operation
                else {
                    unreachable!()
                };
                let RowOperation::ConceptFilter { concept, .. } =
                    &mut target_requirements[0].operation
                else {
                    unreachable!()
                };
                *concept = "unavailable".into()
            }
            _ => {
                let RowOperation::Filter { predicate } = &mut target_requirements[0].operation
                else {
                    unreachable!()
                };
                let RowPredicate::Compare { field, .. } = predicate else {
                    unreachable!()
                };
                field.instance = "p".into()
            }
        }
        assert!(matches!(
            compile_rows(&engine, q, CompileOptions::default())
                .await
                .outcome,
            TypedOutcome::Rejected { .. }
        ));
    }
    let mut depth_options = CompileOptions::default();
    depth_options.max_depth = 2;
    let mut scope_options = CompileOptions::default();
    scope_options.allowed_relations = Some(["products".into(), "items".into()].into());
    let limited = compile_rows(&engine, query(ExistenceMode::Exists), depth_options).await;
    assert!(
        matches!(&limited.outcome, TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "work_limit"),
        "depth limit must remain nonpassing: {:?}",
        limited.outcome
    );
    let denied = compile_rows(&engine, query(ExistenceMode::Exists), scope_options).await;
    assert!(
        matches!(&denied.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "access_scope"),
        "inner endpoint must remain denied: {:?}",
        denied.outcome
    );
}
#[tokio::test]
async fn nested_evidence_requires_each_child_and_retains_private_structural_location() {
    let engine = fixture();
    let q = query(ExistenceMode::Exists);
    let mut request = String::new();
    let mut spans = BTreeMap::new();
    visit_requirements(&q.requirements, &mut |r, _, _| {
        let start = request.len();
        request.push_str(&r.source_text);
        let end = request.len();
        request.push(' ');
        spans.insert(r.id.clone(), vec![RequestSpan { start, end }]);
        Ok::<(), std::convert::Infallible>(())
    })
    .unwrap();
    let evidence = RequestEvidence {
        version: 1,
        request_id: "nested".into(),
        original_request: request,
        requirement_spans: spans,
        unresolved_alternatives: vec![],
    };
    assert!(matches!(
        compile_intent(
            &engine,
            IntentQuery {
                query: q.clone(),
                evidence: evidence.clone()
            },
            CompileOptions::default()
        )
        .await
        .outcome,
        TypedOutcome::Compiled { .. }
    ));
    let mut missing = evidence;
    missing.requirement_spans.remove("completed");
    assert!(
        matches!(compile_intent(&engine,IntentQuery { query:q,evidence:missing },CompileOptions::default()).await.outcome,TypedOutcome::Rejected { diagnostic } if diagnostic.code=="request_coverage")
    );
}

#[tokio::test]
async fn nested_prepared_parameter_binding_and_inner_concept_pins_survive_lowering() {
    use semantic_compiler::typed::{ParameterDeclaration, PreparedRows, PreparedType};
    let engine = fixture();
    let mut q = query(ExistenceMode::Exists);
    let RowOperation::Related {
        target_requirements,
        ..
    } = &mut q.requirements[1].operation
    else {
        unreachable!()
    };
    target_requirements[0].operation = RowOperation::Filter {
        predicate: RowPredicate::CompareParameter {
            field: FieldRef {
                instance: "i".into(),
                field: "quantity".into(),
            },
            operator: Comparison::Gt,
            parameter: "minimum".into(),
        },
    };
    let prepared = PreparedRows::prepare(
        &engine,
        q,
        vec![ParameterDeclaration {
            name: "minimum".into(),
            value_type: PreparedType::Int64,
        }],
        None,
        CompileOptions::default(),
    )
    .unwrap();
    for (minimum, expected) in [(1, vec!["1", "1"]), (3, vec![])] {
        let result = prepared
            .bind_values(
                &engine,
                [("minimum".into(), Literal::Int64(minimum))].into(),
                None,
            )
            .await
            .unwrap();
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
        let generated = query
            .execute(&engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(values(&generated), expected);
        assert!(
            query
                .execute(&fixture_with_concept("pending"), QueryOptions::default())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn nested_graph_leaf_evidence_and_execution_scope_cover_inner_relations() {
    use semantic_plan::graph::*;
    let engine = fixture();
    let q = query(ExistenceMode::Exists);
    let mut request = String::from("qualified products ");
    let mut entries = vec![GraphRequirementEvidence {
        target: GraphRequirementRef::Node {
            node: "rows".into(),
        },
        source_spans: vec![RequestSpan { start: 0, end: 18 }],
    }];
    visit_requirements(&q.requirements, &mut |r, _, _| {
        let start = request.len();
        request.push_str(&r.source_text);
        let end = request.len();
        request.push(' ');
        entries.push(GraphRequirementEvidence {
            target: GraphRequirementRef::Leaf {
                node: "rows".into(),
                requirement: r.id.clone(),
            },
            source_spans: vec![RequestSpan { start, end }],
        });
        Ok::<(), std::convert::Infallible>(())
    })
    .unwrap();
    let graph = GraphQuery {
        version: 1,
        nodes: vec![QueryNode {
            id: "rows".into(),
            source_text: "qualified products".into(),
            operation: GraphOperation::Rows { query: q },
        }],
        root: "rows".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    };
    let evidence = GraphRequestEvidence {
        version: 1,
        request_id: "nested".into(),
        original_request: request,
        requirements: entries,
        unresolved_alternatives: vec![],
    };
    let result = semantic_compiler::typed::compile_graph_intent(
        &engine,
        GraphIntentQuery {
            query: graph.clone(),
            evidence: evidence.clone(),
        },
        CompileOptions::default(),
    )
    .await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    assert!(
        query
            .execute_authorized(
                &engine,
                &["products".into(), "items".into()].into(),
                QueryOptions::default()
            )
            .await
            .is_err()
    );
    assert_eq!(
        values(
            &query
                .plan_direct(&engine)
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()
        ),
        vec!["1", "1"]
    );
    let mut missing = evidence;
    missing.requirements.retain(|r| {
        r.target
            != GraphRequirementRef::Leaf {
                node: "rows".into(),
                requirement: "completed".into(),
            }
    });
    let result = semantic_compiler::typed::compile_graph_intent(
        &engine,
        GraphIntentQuery {
            query: graph,
            evidence: missing,
        },
        CompileOptions::default(),
    )
    .await;
    assert!(
        matches!(result.outcome,TypedOutcome::Rejected { diagnostic } if diagnostic.code=="request_coverage" && diagnostic.message.contains("target_requirements/1/operation/target_requirements/0"))
    );
}

#[tokio::test]
async fn inner_policy_filters_before_absence_and_stale_policy_cannot_replay() {
    for visible in [10, 11] {
        let engine = fixture_profile("completed", Some(visible));
        let result = compile_rows(
            &engine,
            query(ExistenceMode::Absent),
            CompileOptions::default(),
        )
        .await;
        assert!(
            result
                .record
                .definition_refs
                .iter()
                .any(|r| r.id == "policies/visible-order")
        );
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("{:?}", result.outcome)
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
        let expected = if visible == 10 {
            vec!["", "2", "3", "4", "5"]
        } else {
            vec!["", "1", "1", "2", "3", "4", "5"]
        };
        assert_eq!(values(&direct), expected);
        assert_eq!(values(&sql), expected);
        assert!(
            query
                .execute(
                    &fixture_profile("completed", Some(if visible == 10 { 11 } else { 10 })),
                    QueryOptions::default()
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn public_occurrence_names_cannot_capture_compiler_correlation_aliases() {
    let engine = fixture();
    for legacy in [false, true] {
        let mut q = query(ExistenceMode::Absent);
        q.input.instance = "rhs".into();
        visit_requirements_mut(&mut q.requirements, &mut |r, _, _| {
            match &mut r.operation {
                RowOperation::Project { field, .. } => field.instance = "rhs".into(),
                RowOperation::Filter { predicate } => {
                    let RowPredicate::Compare { field, .. } = predicate else {
                        unreachable!()
                    };
                    field.instance = "src".into();
                }
                RowOperation::Related { instance, .. } => {
                    *instance = if instance == "i" {
                        "src"
                    } else {
                        "__semantic_related_0"
                    }
                    .into()
                }
                _ => {}
            }
            Ok::<(), std::convert::Infallible>(())
        })
        .unwrap();
        if legacy {
            let RowOperation::Related {
                predicate,
                target_requirements,
                ..
            } = &mut q.requirements[1].operation
            else {
                unreachable!()
            };
            target_requirements.clear();
            *predicate = Some(RowPredicate::Compare {
                field: FieldRef {
                    instance: "src".into(),
                    field: "quantity".into(),
                },
                operator: Comparison::Gt,
                value: Literal::Int64(0),
            });
        }
        let result = compile_rows(&engine, q, CompileOptions::default()).await;
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("{:?}", result.outcome)
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
        let reference=engine.query(if legacy { "SELECT p.id FROM products p WHERE NOT EXISTS (SELECT 1 FROM items i WHERE i.product=p.id AND i.quantity>0)" } else { "SELECT p.id FROM products p WHERE NOT EXISTS (SELECT 1 FROM items i WHERE i.product=p.id AND i.quantity>0 AND EXISTS (SELECT 1 FROM orders o WHERE o.id=i.order_id AND o.status='completed'))" }).await.unwrap();
        assert_eq!(values(&direct), values(&reference));
        assert_eq!(values(&sql), values(&reference));
        assert!(values(&sql).contains(&"3".into()));
    }
}
