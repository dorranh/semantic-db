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
