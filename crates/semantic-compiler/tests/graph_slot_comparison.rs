use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, BooleanArray, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{FieldSemantics, Relation, RelationSemantics, Unit};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_graph, compile_graph_intent};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::{graph::*, typed::*};

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("left_amount", DataType::Int64, true),
        Field::new("right_amount", DataType::Int64, true),
        Field::new("eur_amount", DataType::Int64, false),
        Field::new("unknown_amount", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
            Arc::new(Int64Array::from(vec![Some(5), Some(2), None, Some(4)])),
            Arc::new(Int64Array::from(vec![Some(3), Some(2), Some(9), None])),
            Arc::new(Int64Array::from(vec![1, 1, 1, 1])),
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
            Arc::new(StringArray::from(vec!["a", "b", "c", "d"])),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("amounts", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        fields: [
            ("left_amount", "USD"),
            ("right_amount", "USD"),
            ("eur_amount", "EUR"),
        ]
        .into_iter()
        .map(|(name, code)| {
            (
                name.into(),
                FieldSemantics {
                    unit: Some(Unit::Currency { code: code.into() }),
                    ..Default::default()
                },
            )
        })
        .collect(),
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

fn query() -> GraphQuery {
    GraphQuery {
        version: 1,
        nodes: vec![
            QueryNode {
                id: "source".into(),
                source_text: "read amounts".into(),
                operation: GraphOperation::Rows {
                    query: RowQuery {
                        version: 1,
                        input: RelationInput {
                            relation: "amounts".into(),
                            instance: "a".into(),
                        },
                        requirements: [
                            "id",
                            "left_amount",
                            "right_amount",
                            "eur_amount",
                            "unknown_amount",
                            "label",
                        ]
                        .into_iter()
                        .map(|name| Requirement {
                            id: name.into(),
                            source_text: name.into(),
                            operation: RowOperation::Project {
                                field: FieldRef {
                                    instance: "a".into(),
                                    field: name.into(),
                                },
                                alias: name.into(),
                            },
                        })
                        .collect(),
                        unresolved: vec![],
                    },
                },
            },
            QueryNode {
                id: "compare".into(),
                source_text: "compare two USD amounts".into(),
                operation: GraphOperation::CompareSlots {
                    input: "source".into(),
                    passthrough: vec![GraphProjection {
                        id: "row_id".into(),
                        slot: "id".into(),
                        alias: "row_id".into(),
                    }],
                    comparisons: vec![
                        GraphSlotComparison {
                            id: "equal".into(),
                            left: "left_amount".into(),
                            right: "right_amount".into(),
                            operator: Comparison::Eq,
                            alias: "equal".into(),
                        },
                        GraphSlotComparison {
                            id: "greater".into(),
                            left: "left_amount".into(),
                            right: "right_amount".into(),
                            operator: Comparison::Gt,
                            alias: "greater".into(),
                        },
                    ],
                },
            },
        ],
        root: "compare".into(),
        ordering: vec![GraphOrder {
            slot: "row_id".into(),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        }],
        limit: None,
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> Vec<(i64, Option<bool>, Option<bool>)> {
    batches
        .iter()
        .flat_map(|batch| {
            let ids = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            let equal = batch
                .column(1)
                .as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap();
            let greater = batch
                .column(2)
                .as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap();
            (0..batch.num_rows())
                .map(|row| {
                    (
                        ids.value(row),
                        (!equal.is_null(row)).then(|| equal.value(row)),
                        (!greater.is_null(row)).then(|| greater.value(row)),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn typed_slot_comparisons_preserve_true_false_unknown_in_sql_and_direct() {
    let engine = fixture();
    let result = compile_graph(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("expected slot comparison graph: {:?}", result.outcome);
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
        (1, Some(false), Some(true)),
        (2, Some(true), Some(false)),
        (3, None, None),
        (4, None, None),
    ];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
    assert_eq!(direct[0].schema().field(1).data_type(), &DataType::Boolean);
    assert!(direct[0].schema().field(1).is_nullable());
    assert!(query.sql().statement().contains("= "));
    assert!(query.sql().statement().contains(" > "));
}

#[tokio::test]
async fn slot_comparison_rejects_wrong_type_scope_unit_and_unknown_operator() {
    let engine = fixture();
    let mut wrong_type = query();
    let GraphOperation::CompareSlots { comparisons, .. } = &mut wrong_type.nodes[1].operation
    else {
        unreachable!()
    };
    comparisons[0].right = "label".into();
    assert!(matches!(
        compile_graph(&engine, wrong_type, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_comparison_type"
    ));

    let mut cross_scope = query();
    let GraphOperation::CompareSlots { comparisons, .. } = &mut cross_scope.nodes[1].operation
    else {
        unreachable!()
    };
    comparisons[0].left = "other.left_amount".into();
    assert!(matches!(
        compile_graph(&engine, cross_scope, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_slot"
    ));

    for right in ["eur_amount", "unknown_amount"] {
        let mut mismatch = query();
        let GraphOperation::CompareSlots { comparisons, .. } = &mut mismatch.nodes[1].operation
        else {
            unreachable!()
        };
        comparisons[0].right = right.into();
        assert!(matches!(
            compile_graph(&engine, mismatch, CompileOptions::default()).await.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_comparison_unit"
        ));
    }

    let mut both_unknown = query();
    let GraphOperation::CompareSlots { comparisons, .. } = &mut both_unknown.nodes[1].operation
    else {
        unreachable!()
    };
    comparisons.truncate(1);
    comparisons[0].left = "id".into();
    comparisons[0].right = "unknown_amount".into();
    assert!(matches!(
        compile_graph(&engine, both_unknown, CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::CompiledGraph { .. }
    ));

    let mut malformed = serde_json::to_value(query()).unwrap();
    malformed["nodes"][1]["operation"]["comparisons"][0]["operator"] =
        serde_json::json!("approximate");
    assert!(serde_json::from_value::<GraphQuery>(malformed).is_err());
}

#[tokio::test]
async fn each_comparison_output_requires_graph_intent_evidence() {
    let engine = fixture();
    let source = "read amounts id left_amount right_amount eur_amount unknown_amount label compare two USD amounts";
    let span = |needle: &str| {
        let start = source.find(needle).unwrap();
        RequestSpan {
            start,
            end: start + needle.len(),
        }
    };
    let mut requirements = vec![GraphRequirementEvidence {
        target: GraphRequirementRef::Node {
            node: "source".into(),
        },
        source_spans: vec![span("read amounts")],
    }];
    for field in [
        "id",
        "left_amount",
        "right_amount",
        "eur_amount",
        "unknown_amount",
        "label",
    ] {
        requirements.push(GraphRequirementEvidence {
            target: GraphRequirementRef::Leaf {
                node: "source".into(),
                requirement: field.into(),
            },
            source_spans: vec![span(field)],
        });
    }
    requirements.push(GraphRequirementEvidence {
        target: GraphRequirementRef::Node {
            node: "compare".into(),
        },
        source_spans: vec![span("compare two USD amounts")],
    });
    for slot in ["row_id", "equal", "greater"] {
        requirements.push(GraphRequirementEvidence {
            target: GraphRequirementRef::Output {
                node: "compare".into(),
                slot: slot.into(),
            },
            source_spans: vec![span("amount")],
        });
    }
    requirements.push(GraphRequirementEvidence {
        target: GraphRequirementRef::Order { index: 0 },
        source_spans: vec![span("id")],
    });
    let evidence = GraphRequestEvidence {
        version: 1,
        request_id: "slot-compare".into(),
        original_request: source.into(),
        requirements,
        unresolved_alternatives: vec![],
    };
    assert!(matches!(
        compile_graph_intent(
            &engine,
            GraphIntentQuery {
                query: query(),
                evidence: evidence.clone(),
            },
            CompileOptions::default(),
        )
        .await
        .outcome,
        TypedOutcome::CompiledGraph { .. }
    ));
    let mut missing = evidence;
    missing.requirements.retain(|entry| {
        entry.target
            != GraphRequirementRef::Output {
                node: "compare".into(),
                slot: "greater".into(),
            }
    });
    assert!(matches!(
        compile_graph_intent(
            &engine,
            GraphIntentQuery {
                query: query(),
                evidence: missing,
            },
            CompileOptions::default(),
        )
        .await
        .outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "request_coverage"
    ));
}

fn integer_fixture(left: DataType, right: DataType) -> Engine {
    use datafusion::arrow::array::*;
    fn values(kind: &DataType, reversed: bool) -> ArrayRef {
        macro_rules! signed {
            ($array:ty, $native:ty) => {{
                let mut values = vec![
                    Some(<$native>::MIN),
                    Some(<$native>::MAX),
                    None,
                    Some(0),
                    Some(0),
                ];
                if reversed {
                    values.swap(0, 1);
                    values[2] = Some(0);
                    values[4] = None;
                }
                Arc::new(<$array>::from(values)) as ArrayRef
            }};
        }
        macro_rules! unsigned {
            ($array:ty, $native:ty) => {{
                let mut values = vec![Some(0), Some(<$native>::MAX), None, Some(0), Some(0)];
                if reversed {
                    values.swap(0, 1);
                    values[2] = Some(0);
                    values[4] = None;
                }
                Arc::new(<$array>::from(values)) as ArrayRef
            }};
        }
        match kind {
            DataType::Int8 => signed!(Int8Array, i8),
            DataType::Int16 => signed!(Int16Array, i16),
            DataType::Int32 => signed!(Int32Array, i32),
            DataType::Int64 => signed!(Int64Array, i64),
            DataType::UInt8 => unsigned!(UInt8Array, u8),
            DataType::UInt16 => unsigned!(UInt16Array, u16),
            DataType::UInt32 => unsigned!(UInt32Array, u32),
            DataType::UInt64 => unsigned!(UInt64Array, u64),
            _ => unreachable!(),
        }
    }
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("left_amount", left.clone(), true),
        Field::new("right_amount", right.clone(), true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])),
            values(&left, false),
            values(&right, true),
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("amounts", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn integer_query() -> GraphQuery {
    let mut query = query();
    let GraphOperation::Rows { query: rows } = &mut query.nodes[0].operation else {
        unreachable!()
    };
    rows.requirements
        .retain(|r| matches!(r.id.as_str(), "id" | "left_amount" | "right_amount"));
    let GraphOperation::CompareSlots { comparisons, .. } = &mut query.nodes[1].operation else {
        unreachable!()
    };
    *comparisons = [
        Comparison::Eq,
        Comparison::NotEq,
        Comparison::Lt,
        Comparison::LtEq,
        Comparison::Gt,
        Comparison::GtEq,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, operator)| GraphSlotComparison {
        id: format!("comparison_{i}"),
        left: "left_amount".into(),
        right: "right_amount".into(),
        operator,
        alias: format!("comparison_{i}"),
    })
    .collect();
    query
}

fn boolean_rows(batches: &[RecordBatch]) -> Vec<Vec<Option<bool>>> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows())
                .map(|row| {
                    batch
                        .columns()
                        .iter()
                        .skip(1)
                        .map(|column| {
                            let values = column.as_any().downcast_ref::<BooleanArray>().unwrap();
                            (!values.is_null(row)).then(|| values.value(row))
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn every_exact_integer_width_compares_extrema_and_nulls_in_direct_and_sql() {
    for kind in [
        DataType::Int8,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
    ] {
        let engine = integer_fixture(kind.clone(), kind.clone());
        let result = compile_graph(&engine, integer_query(), CompileOptions::default()).await;
        let TypedOutcome::CompiledGraph { query } = result.outcome else {
            panic!("{kind:?} failed: {:?}", result.outcome);
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
            vec![
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
            ],
            vec![
                Some(false),
                Some(true),
                Some(false),
                Some(false),
                Some(true),
                Some(true),
            ],
            vec![None; 6],
            vec![
                Some(true),
                Some(false),
                Some(false),
                Some(true),
                Some(false),
                Some(true),
            ],
            vec![None; 6],
        ];
        assert_eq!(boolean_rows(&direct), expected, "direct {kind:?}");
        assert_eq!(boolean_rows(&sql), expected, "SQL {kind:?}");
        assert_eq!(direct[0].schema(), sql[0].schema(), "schema {kind:?}");
        assert!(
            direct[0]
                .schema()
                .fields()
                .iter()
                .skip(1)
                .all(|f| f.data_type() == &DataType::Boolean && f.is_nullable())
        );
    }
}

#[tokio::test]
async fn integer_slot_comparisons_reject_width_and_signedness_coercion() {
    for (left, right) in [
        (DataType::Int16, DataType::Int32),
        (DataType::UInt32, DataType::UInt64),
        (DataType::Int64, DataType::UInt64),
        (DataType::UInt8, DataType::Int8),
    ] {
        let result = compile_graph(
            &integer_fixture(left.clone(), right.clone()),
            integer_query(),
            CompileOptions::default(),
        )
        .await;
        assert!(
            matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_comparison_type"),
            "{left:?}/{right:?}"
        );
    }
}

#[tokio::test]
async fn project_trims_comparison_operands_without_changing_rows_or_nulls() {
    let engine = fixture();
    let mut graph = query();
    graph.nodes.push(QueryNode {
        id: "selected".into(),
        source_text: "show equality".into(),
        operation: GraphOperation::Project {
            input: "compare".into(),
            columns: vec![
                GraphProjection {
                    id: "row_id".into(),
                    slot: "row_id".into(),
                    alias: "row_id".into(),
                },
                GraphProjection {
                    id: "answer".into(),
                    slot: "equal".into(),
                    alias: "answer".into(),
                },
            ],
        },
    });
    graph.root = "selected".into();
    let result = compile_graph(&engine, graph.clone(), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
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
    assert_eq!(direct[0].schema(), sql[0].schema());
    assert_eq!(
        boolean_rows(&direct),
        vec![vec![Some(false)], vec![Some(true)], vec![None], vec![None]]
    );
    assert_eq!(boolean_rows(&sql), boolean_rows(&direct));
    for columns in [
        vec![],
        vec![GraphProjection {
            id: "a".into(),
            slot: "missing".into(),
            alias: "a".into(),
        }],
        vec![
            GraphProjection {
                id: "a".into(),
                slot: "equal".into(),
                alias: "a".into()
            };
            2
        ],
    ] {
        let GraphOperation::Project {
            columns: target, ..
        } = &mut graph.nodes[2].operation
        else {
            unreachable!()
        };
        *target = columns;
        assert!(matches!(
            compile_graph(&engine, graph.clone(), CompileOptions::default())
                .await
                .outcome,
            TypedOutcome::Rejected { .. }
        ));
    }
}

#[tokio::test]
async fn projection_requires_node_and_each_output_evidence() {
    let engine = fixture();
    let mut graph = query();
    graph.ordering.clear();
    graph.nodes.push(QueryNode {
        id: "selected".into(),
        source_text: "selected equality".into(),
        operation: GraphOperation::Project {
            input: "compare".into(),
            columns: vec![GraphProjection {
                id: "answer".into(),
                slot: "equal".into(),
                alias: "answer".into(),
            }],
        },
    });
    graph.root = "selected".into();
    let mut request = String::new();
    let mut requirements = Vec::new();
    let mut add = |target, text: &str| {
        let start = request.len();
        request.push_str(text);
        let end = request.len();
        request.push(' ');
        requirements.push(GraphRequirementEvidence {
            target,
            source_spans: vec![RequestSpan { start, end }],
        });
    };
    for node in &graph.nodes {
        add(
            GraphRequirementRef::Node {
                node: node.id.clone(),
            },
            &node.source_text,
        );
        match &node.operation {
            GraphOperation::Rows { query } => {
                for r in &query.requirements {
                    add(
                        GraphRequirementRef::Leaf {
                            node: node.id.clone(),
                            requirement: r.id.clone(),
                        },
                        &r.source_text,
                    );
                }
            }
            GraphOperation::CompareSlots {
                passthrough,
                comparisons,
                ..
            } => {
                for id in passthrough
                    .iter()
                    .map(|p| &p.id)
                    .chain(comparisons.iter().map(|p| &p.id))
                {
                    add(
                        GraphRequirementRef::Output {
                            node: node.id.clone(),
                            slot: id.clone(),
                        },
                        &node.source_text,
                    );
                }
            }
            GraphOperation::Project { columns, .. } => {
                for p in columns {
                    add(
                        GraphRequirementRef::Output {
                            node: node.id.clone(),
                            slot: p.id.clone(),
                        },
                        &node.source_text,
                    );
                }
            }
            _ => unreachable!(),
        }
    }
    let evidence = GraphRequestEvidence {
        version: 1,
        request_id: "projection".into(),
        original_request: request,
        requirements,
        unresolved_alternatives: vec![],
    };
    assert!(matches!(
        compile_graph_intent(
            &engine,
            GraphIntentQuery {
                query: graph.clone(),
                evidence: evidence.clone()
            },
            CompileOptions::default()
        )
        .await
        .outcome,
        TypedOutcome::CompiledGraph { .. }
    ));
    for target in [
        GraphRequirementRef::Node {
            node: "selected".into(),
        },
        GraphRequirementRef::Output {
            node: "selected".into(),
            slot: "answer".into(),
        },
    ] {
        let mut missing = evidence.clone();
        missing.requirements.retain(|r| r.target != target);
        assert!(
            matches!(compile_graph_intent(&engine, GraphIntentQuery { query: graph.clone(), evidence: missing }, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "request_coverage")
        );
    }
}
