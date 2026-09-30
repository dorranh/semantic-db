use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{BooleanArray, Int64Array, StringArray},
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
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Int64Array::from(vec![None, Some(0), Some(-5)])),
            Arc::new(StringArray::from(vec!["a", "b", "c"])),
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("items", schema.clone(), "memory"),
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
                source_text: "read items".into(),
                operation: GraphOperation::Rows {
                    query: RowQuery {
                        version: 1,
                        input: RelationInput {
                            relation: "items".into(),
                            instance: "i".into(),
                        },
                        requirements: ["id", "value", "label"]
                            .into_iter()
                            .map(|name| Requirement {
                                id: name.into(),
                                source_text: name.into(),
                                operation: RowOperation::Project {
                                    field: FieldRef {
                                        instance: "i".into(),
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
                id: "checks".into(),
                source_text: "check missing value".into(),
                operation: GraphOperation::NullTest {
                    input: "source".into(),
                    passthrough: vec![GraphProjection {
                        id: "row_id".into(),
                        slot: "id".into(),
                        alias: "row_id".into(),
                    }],
                    tests: vec![
                        GraphNullTest {
                            id: "missing".into(),
                            slot: "value".into(),
                            operator: GraphNullOperator::IsNull,
                            alias: "missing".into(),
                        },
                        GraphNullTest {
                            id: "present".into(),
                            slot: "value".into(),
                            operator: GraphNullOperator::IsNotNull,
                            alias: "present".into(),
                        },
                    ],
                },
            },
        ],
        root: "checks".into(),
        ordering: vec![GraphOrder {
            slot: "row_id".into(),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        }],
        limit: None,
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> Vec<(i64, bool, bool)> {
    batches
        .iter()
        .flat_map(|batch| {
            let ids = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            let missing = batch
                .column(1)
                .as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap();
            let present = batch
                .column(2)
                .as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap();
            (0..batch.num_rows())
                .map(|row| (ids.value(row), missing.value(row), present.value(row)))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn checked_null_values_are_nonnullable_boolean_and_match_sql_direct() {
    let engine = fixture();
    let result = compile_graph(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("expected checked null-test graph: {:?}", result.outcome);
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
    let expected = vec![(1, true, false), (2, false, true), (3, false, true)];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
    assert_eq!(direct[0].schema().field(1).data_type(), &DataType::Boolean);
    assert!(!direct[0].schema().field(1).is_nullable());
    assert!(query.sql().statement().contains("IS NULL"));
    assert!(query.sql().statement().contains("IS NOT NULL"));
}

#[tokio::test]
async fn null_test_rejects_wrong_type_cross_node_slot_and_malformed_wire() {
    let engine = fixture();
    let mut wrong_type = query();
    let GraphOperation::NullTest { tests, .. } = &mut wrong_type.nodes[1].operation else {
        unreachable!()
    };
    tests[0].slot = "label".into();
    assert!(matches!(
        compile_graph(&engine, wrong_type, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_null_test_type"
    ));

    let mut cross_scope = query();
    let GraphOperation::NullTest { tests, .. } = &mut cross_scope.nodes[1].operation else {
        unreachable!()
    };
    tests[0].slot = "other.value".into();
    assert!(matches!(
        compile_graph(&engine, cross_scope, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_slot"
    ));

    let mut empty = query();
    let GraphOperation::NullTest { tests, .. } = &mut empty.nodes[1].operation else {
        unreachable!()
    };
    tests.clear();
    assert!(matches!(
        compile_graph(&engine, empty, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_null_test"
    ));

    let mut unsupported = serde_json::to_value(query()).unwrap();
    unsupported["nodes"][1]["operation"]["tests"][0]["operator"] =
        serde_json::json!("unchecked_function");
    assert!(serde_json::from_value::<GraphQuery>(unsupported).is_err());
}

#[tokio::test]
async fn null_test_outputs_each_require_intent_evidence() {
    let engine = fixture();
    let source = "read items id value label check missing value";
    let span = |needle: &str| {
        let start = source.find(needle).unwrap();
        RequestSpan {
            start,
            end: start + needle.len(),
        }
    };
    let evidence = GraphRequestEvidence {
        version: 1,
        request_id: "null-check".into(),
        original_request: source.into(),
        requirements: vec![
            GraphRequirementEvidence {
                target: GraphRequirementRef::Node {
                    node: "source".into(),
                },
                source_spans: vec![span("read items")],
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
                target: GraphRequirementRef::Leaf {
                    node: "source".into(),
                    requirement: "label".into(),
                },
                source_spans: vec![span("label")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Node {
                    node: "checks".into(),
                },
                source_spans: vec![span("check missing value")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Output {
                    node: "checks".into(),
                    slot: "row_id".into(),
                },
                source_spans: vec![span("id")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Output {
                    node: "checks".into(),
                    slot: "missing".into(),
                },
                source_spans: vec![span("missing")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Output {
                    node: "checks".into(),
                    slot: "present".into(),
                },
                source_spans: vec![span("check")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Order { index: 0 },
                source_spans: vec![span("id")],
            },
        ],
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
    missing.requirements.retain(|item| {
        item.target
            != GraphRequirementRef::Output {
                node: "checks".into(),
                slot: "present".into(),
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
