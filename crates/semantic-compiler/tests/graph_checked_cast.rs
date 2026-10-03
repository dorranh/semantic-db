use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, Decimal128Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    FactResolution, FieldSemantics, Presence, Relation, RelationSemantics, Unit,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_graph, compile_graph_intent};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::{graph::*, typed::*};

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Int64, true),
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
            Arc::new(Int64Array::from(vec![
                Some(7),
                Some(-3),
                Some(i64::MAX),
                None,
            ])),
            Arc::new(StringArray::from(vec!["a", "b", "c", "d"])),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("amounts", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        fields: [(
            "amount".into(),
            FieldSemantics {
                unit: Some(Unit::Currency { code: "USD".into() }),
                ..Default::default()
            },
        )]
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
                        requirements: ["id", "amount", "label"]
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
                id: "cast".into(),
                source_text: "cast amount to decimal".into(),
                operation: GraphOperation::Cast {
                    input: "source".into(),
                    passthrough: vec![GraphProjection {
                        id: "row_id".into(),
                        slot: "id".into(),
                        alias: "row_id".into(),
                    }],
                    casts: vec![GraphCast {
                        id: "decimal".into(),
                        slot: "amount".into(),
                        target: GraphCastTarget::Decimal128Scale0,
                        alias: "decimal".into(),
                    }],
                },
            },
        ],
        root: "cast".into(),
        ordering: vec![GraphOrder {
            slot: "row_id".into(),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        }],
        limit: None,
        unresolved: vec![],
    }
}

fn values(batches: &[RecordBatch]) -> Vec<(i64, Option<i128>)> {
    batches
        .iter()
        .flat_map(|batch| {
            let ids = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            let amounts = batch
                .column(1)
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .unwrap();
            (0..batch.num_rows())
                .map(|row| {
                    (
                        ids.value(row),
                        (!amounts.is_null(row)).then(|| amounts.value(row)),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn checked_cast_preserves_nulls_large_negative_values_and_authored_unit() {
    let engine = fixture();
    let result = compile_graph(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("expected checked cast: {:?}", result.outcome);
    };
    assert!(matches!(
        &query.slot_meaning("decimal").unwrap().unit,
        FactResolution::Known {
            value: Presence::Value(Unit::Currency { code }),
            ..
        } if code == "USD"
    ));
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
        (1, Some(7)),
        (2, Some(-3)),
        (3, Some(i64::MAX as i128)),
        (4, None),
    ];
    assert_eq!(values(&direct), expected);
    assert_eq!(values(&sql), expected);
    assert_eq!(
        direct[0].schema().field(1).data_type(),
        &DataType::Decimal128(38, 0)
    );
    assert!(direct[0].schema().field(1).is_nullable());
    assert!(query.sql().statement().contains("CAST("));
    assert!(query.sql().statement().contains("DECIMAL(38"));
}

#[tokio::test]
async fn checked_cast_rejects_wrong_physical_type_cross_node_slot_and_unsupported_target() {
    let engine = fixture();
    let mut wrong_type = query();
    let GraphOperation::Cast { casts, .. } = &mut wrong_type.nodes[1].operation else {
        unreachable!()
    };
    casts[0].slot = "label".into();
    assert!(matches!(
        compile_graph(&engine, wrong_type, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_cast_type"
    ));

    let mut cross_scope = query();
    let GraphOperation::Cast { casts, .. } = &mut cross_scope.nodes[1].operation else {
        unreachable!()
    };
    casts[0].slot = "other.amount".into();
    assert!(matches!(
        compile_graph(&engine, cross_scope, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_slot"
    ));

    let mut unsupported = serde_json::to_value(query()).unwrap();
    unsupported["nodes"][1]["operation"]["casts"][0]["target"] = serde_json::json!("float64");
    assert!(serde_json::from_value::<GraphQuery>(unsupported).is_err());
}

#[tokio::test]
async fn checked_cast_output_requires_its_own_intent_evidence() {
    let engine = fixture();
    let source = "read amounts id amount label cast amount to decimal";
    let span = |needle: &str| {
        let start = source.find(needle).unwrap();
        RequestSpan {
            start,
            end: start + needle.len(),
        }
    };
    let evidence = GraphRequestEvidence {
        version: 1,
        request_id: "cast-request".into(),
        original_request: source.into(),
        requirements: vec![
            GraphRequirementEvidence {
                target: GraphRequirementRef::Node {
                    node: "source".into(),
                },
                source_spans: vec![span("read amounts")],
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
                    requirement: "amount".into(),
                },
                source_spans: vec![span("amount")],
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
                    node: "cast".into(),
                },
                source_spans: vec![span("cast amount to decimal")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Output {
                    node: "cast".into(),
                    slot: "row_id".into(),
                },
                source_spans: vec![span("id")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Output {
                    node: "cast".into(),
                    slot: "decimal".into(),
                },
                source_spans: vec![span("decimal")],
            },
            GraphRequirementEvidence {
                target: GraphRequirementRef::Order { index: 0 },
                source_spans: vec![span("id")],
            },
        ],
        unresolved_alternatives: vec![],
    };
    let accepted = compile_graph_intent(
        &engine,
        GraphIntentQuery {
            query: query(),
            evidence: evidence.clone(),
        },
        CompileOptions::default(),
    )
    .await;
    assert!(matches!(
        accepted.outcome,
        TypedOutcome::CompiledGraph { .. }
    ));
    let mut removed = evidence;
    removed.requirements.retain(|item| {
        item.target
            != GraphRequirementRef::Output {
                node: "cast".into(),
                slot: "decimal".into(),
            }
    });
    assert!(matches!(
        compile_graph_intent(
            &engine,
            GraphIntentQuery {
                query: query(),
                evidence: removed,
            },
            CompileOptions::default(),
        )
        .await
        .outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "request_coverage"
    ));
}
