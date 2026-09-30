use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Decimal128Array, Int64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_graph};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::{graph::*, typed::*};

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("score", DataType::Decimal128(38, 18), false),
        Field::new("optional", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(
                Decimal128Array::from(vec![
                    Some(5_000_000_000_000_000_000),
                    Some(3_000_000_000_000_000_000),
                    Some(1_000_000_000_000_000_000),
                ])
                .with_precision_and_scale(38, 18)
                .unwrap(),
            ),
            Arc::new(Int64Array::from(vec![None, Some(7), None])),
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("scores", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn decimal(coefficient: &str) -> Literal {
    Literal::Decimal128 {
        coefficient: coefficient.into(),
        precision: 38,
        scale: 18,
    }
}

fn query() -> GraphQuery {
    GraphQuery {
        version: 1,
        nodes: vec![
            QueryNode {
                id: "source".into(),
                source_text: "scores and optional values".into(),
                operation: GraphOperation::Rows {
                    query: RowQuery {
                        version: 1,
                        input: RelationInput {
                            relation: "scores".into(),
                            instance: "s".into(),
                        },
                        requirements: ["score", "optional"]
                            .into_iter()
                            .map(|name| Requirement {
                                id: name.into(),
                                source_text: name.into(),
                                operation: RowOperation::Project {
                                    field: FieldRef {
                                        instance: "s".into(),
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
                id: "filtered".into(),
                source_text: "score above four with missing optional, or score at least two".into(),
                operation: GraphOperation::Filter {
                    input: "source".into(),
                    predicate: RowPredicate::Any {
                        predicates: vec![
                            RowPredicate::All {
                                predicates: vec![
                                    RowPredicate::Compare {
                                        field: OutputRef {
                                            slot: "score".into(),
                                        },
                                        operator: Comparison::Gt,
                                        value: decimal("4000000000000000000"),
                                    },
                                    RowPredicate::IsNull {
                                        field: OutputRef {
                                            slot: "optional".into(),
                                        },
                                        negated: false,
                                    },
                                ],
                            },
                            RowPredicate::All {
                                predicates: vec![
                                    RowPredicate::Not {
                                        predicate: Box::new(RowPredicate::Compare {
                                            field: OutputRef {
                                                slot: "score".into(),
                                            },
                                            operator: Comparison::Lt,
                                            value: decimal("2000000000000000000"),
                                        }),
                                    },
                                    RowPredicate::IsNull {
                                        field: OutputRef {
                                            slot: "optional".into(),
                                        },
                                        negated: true,
                                    },
                                ],
                            },
                        ],
                    },
                },
            },
        ],
        root: "filtered".into(),
        ordering: vec![GraphOrder {
            slot: "score".into(),
            direction: Direction::Desc,
            nulls: NullOrder::Last,
        }],
        limit: None,
        unresolved: vec![],
    }
}

#[tokio::test]
async fn checked_nested_boolean_decimal_and_null_filter_has_sql_direct_parity() {
    let engine = fixture();
    let result = compile_graph(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query: artifact } = result.outcome else {
        panic!("expected accepted graph: {:?}", result.outcome);
    };
    let direct = artifact
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = artifact
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let direct = pretty_format_batches(&direct).unwrap().to_string();
    let sql = pretty_format_batches(&sql).unwrap().to_string();
    assert_eq!(direct, sql);
    assert!(sql.contains("5.000000000000000000"));
    assert!(sql.contains("3.000000000000000000"));
    assert!(!sql.contains("1.000000000000000000"));
}

#[tokio::test]
async fn graph_filter_rejects_wrong_literal_type_and_missing_input_slot() {
    let engine = fixture();
    let mut wrong_type = query();
    let GraphOperation::Filter { predicate, .. } = &mut wrong_type.nodes[1].operation else {
        unreachable!()
    };
    let RowPredicate::Any { predicates } = predicate else {
        unreachable!()
    };
    let RowPredicate::All { predicates } = &mut predicates[0] else {
        unreachable!()
    };
    let RowPredicate::Compare { value, .. } = &mut predicates[0] else {
        unreachable!()
    };
    *value = Literal::Int64(4);
    assert!(matches!(
        compile_graph(&engine, wrong_type, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "comparison_type"
    ));

    let mut missing = query();
    let GraphOperation::Filter { predicate, .. } = &mut missing.nodes[1].operation else {
        unreachable!()
    };
    let RowPredicate::Any { predicates } = predicate else {
        unreachable!()
    };
    let RowPredicate::All { predicates } = &mut predicates[0] else {
        unreachable!()
    };
    let RowPredicate::IsNull { field, .. } = &mut predicates[1] else {
        unreachable!()
    };
    field.slot = "from_another_node".into();
    assert!(matches!(
        compile_graph(&engine, missing, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_slot"
    ));
}
