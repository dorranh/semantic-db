use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, Int64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_graph, compile_graph_intent};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::{graph::*, typed::*};

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("value", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Int64Array::from(vec![Some(5), Some(1), None])),
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("values", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn proposal() -> GraphQuery {
    GraphQuery {
        version: 1,
        nodes: vec![
            QueryNode {
                id: "source".into(),
                source_text: "id and value".into(),
                operation: GraphOperation::Rows {
                    query: RowQuery {
                        version: 1,
                        input: RelationInput {
                            relation: "values".into(),
                            instance: "v".into(),
                        },
                        requirements: ["id", "value"]
                            .into_iter()
                            .map(|name| Requirement {
                                id: name.into(),
                                source_text: name.into(),
                                operation: RowOperation::Project {
                                    field: FieldRef {
                                        instance: "v".into(),
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
                id: "choice".into(),
                source_text: "ten if value exceeds two, otherwise twenty".into(),
                operation: GraphOperation::Conditional {
                    input: "source".into(),
                    passthrough: vec![GraphProjection {
                        id: "row_id".into(),
                        slot: "id".into(),
                        alias: "row_id".into(),
                    }],
                    outputs: vec![GraphConditional {
                        id: "chosen".into(),
                        alias: "chosen".into(),
                        when: RowPredicate::Compare {
                            field: OutputRef {
                                slot: "value".into(),
                            },
                            operator: Comparison::Gt,
                            value: Literal::Int64(2),
                        },
                        then_value: Literal::Int64(10),
                        else_value: Literal::Int64(20),
                    }],
                },
            },
        ],
        root: "choice".into(),
        ordering: vec![GraphOrder {
            slot: "row_id".into(),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        }],
        limit: None,
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> Vec<(i64, i64)> {
    batches
        .iter()
        .flat_map(|batch| {
            let ids = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            let values = batch
                .column(1)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            (0..batch.num_rows())
                .map(|row| {
                    assert!(!values.is_null(row));
                    (ids.value(row), values.value(row))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn case_conditional_true_false_and_unknown_match_sql_and_direct() {
    let engine = fixture();
    let result = compile_graph(&engine, proposal(), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("expected graph conditional: {:?}", result.outcome);
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
    let expected = vec![(1, 10), (2, 20), (3, 20)];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
    assert_eq!(query.sql().parameters().len(), 3);
    assert!(query.sql().statement().contains("CASE WHEN"));
}

#[tokio::test]
async fn conditional_rejects_wrong_branch_type_missing_slot_and_unchecked_function() {
    let engine = fixture();
    let mut wrong_type = proposal();
    let GraphOperation::Conditional { outputs, .. } = &mut wrong_type.nodes[1].operation else {
        unreachable!()
    };
    outputs[0].else_value = Literal::Utf8("twenty".into());
    assert!(matches!(
        compile_graph(&engine, wrong_type, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_conditional_type"
    ));

    let mut wrong_scope = proposal();
    let GraphOperation::Conditional { outputs, .. } = &mut wrong_scope.nodes[1].operation else {
        unreachable!()
    };
    let RowPredicate::Compare { field, .. } = &mut outputs[0].when else {
        unreachable!()
    };
    field.slot = "other_node.value".into();
    assert!(matches!(
        compile_graph(&engine, wrong_scope, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_slot"
    ));

    let mut wire = serde_json::to_value(proposal()).unwrap();
    wire["nodes"][1]["operation"]["outputs"][0]["function"] = serde_json::json!("unchecked_sql");
    assert!(serde_json::from_value::<GraphQuery>(wire).is_err());
}

#[tokio::test]
async fn conditional_output_has_independent_graph_evidence_requirement() {
    let engine = fixture();
    let source = "id and value id value ten if value exceeds two, otherwise twenty";
    let span = |needle: &str| {
        let start = source.find(needle).unwrap();
        RequestSpan {
            start,
            end: start + needle.len(),
        }
    };
    let evidence = GraphRequestEvidence {
        version: 1,
        request_id: "condition".into(),
        original_request: source.into(),
        requirements: vec![
            GraphRequirementEvidence {
                target: GraphRequirementRef::Node {
                    node: "source".into(),
                },
                source_spans: vec![span("id and value")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Leaf {
                    node: "source".into(),
                    requirement: "id".into(),
                },
                source_spans: vec![span("id")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Leaf {
                    node: "source".into(),
                    requirement: "value".into(),
                },
                source_spans: vec![span("value")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Node {
                    node: "choice".into(),
                },
                source_spans: vec![span("ten if value exceeds two, otherwise twenty")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Output {
                    node: "choice".into(),
                    slot: "row_id".into(),
                },
                source_spans: vec![span("id")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Output {
                    node: "choice".into(),
                    slot: "chosen".into(),
                },
                source_spans: vec![span("ten")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Order { index: 0 },
                source_spans: vec![span("id")],
            },
        ],
        unresolved_alternatives: vec![],
    };
    let result = compile_graph_intent(
        &engine,
        GraphIntentQuery {
            query: proposal(),
            evidence: evidence.clone(),
        },
        CompileOptions::default(),
    )
    .await;
    assert!(matches!(result.outcome, TypedOutcome::CompiledGraph { .. }));
    let mut missing = evidence;
    missing.requirements.retain(|item| {
        item.target
            != (GraphRequirementRef::Output {
                node: "choice".into(),
                slot: "chosen".into(),
            })
    });
    assert!(matches!(
        compile_graph_intent(
            &engine,
            GraphIntentQuery {
                query: proposal(),
                evidence: missing,
            },
            CompileOptions::default(),
        )
        .await
        .outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "request_coverage"
    ));
}
