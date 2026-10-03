use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, Decimal128Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

fn engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("group_name", DataType::Utf8, false),
        Field::new("amount", DataType::Decimal128(38, 18), true),
    ]));
    let amounts = Decimal128Array::from(vec![
        Some(1_000_000_000_000_000_000),
        Some(2_000_000_000_000_000_000),
        Some(4_000_000_000_000_000_000),
        Some(1_000_000_000_000_000_000),
        None,
    ])
    .with_precision_and_scale(38, 18)
    .unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["a", "a", "b", "b", "c"])),
            Arc::new(amounts),
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

fn requirement(id: &str, operation: RowOperation) -> Requirement {
    Requirement {
        id: id.into(),
        source_text: id.into(),
        operation,
    }
}

fn compare(slot: &str, operator: Comparison, value: Literal) -> OutputPredicate {
    RowPredicate::Compare {
        field: OutputRef { slot: slot.into() },
        operator,
        value,
    }
}

fn decimal(value: i128) -> Literal {
    Literal::Decimal128 {
        coefficient: value.to_string(),
        precision: 38,
        scale: 18,
    }
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "amounts".into(),
            instance: "a".into(),
        },
        requirements: vec![
            requirement(
                "group",
                RowOperation::Group {
                    field: FieldRef {
                        instance: "a".into(),
                        field: "group_name".into(),
                    },
                    alias: "group_name".into(),
                },
            ),
            requirement(
                "sum",
                RowOperation::Aggregate {
                    function: AggregateFunction::Sum,
                    field: Some(FieldRef {
                        instance: "a".into(),
                        field: "amount".into(),
                    }),
                    distinct: false,
                    alias: "total".into(),
                },
            ),
            requirement(
                "keep",
                RowOperation::FilterOutput {
                    stage: OutputFilterStage::AfterAggregate,
                    predicate: RowPredicate::Any {
                        predicates: vec![
                            RowPredicate::All {
                                predicates: vec![
                                    RowPredicate::Not {
                                        predicate: Box::new(compare(
                                            "sum",
                                            Comparison::LtEq,
                                            decimal(4_000_000_000_000_000_000),
                                        )),
                                    },
                                    RowPredicate::IsNull {
                                        field: OutputRef {
                                            slot: "group".into(),
                                        },
                                        negated: true,
                                    },
                                ],
                            },
                            RowPredicate::IsNull {
                                field: OutputRef { slot: "sum".into() },
                                negated: false,
                            },
                        ],
                    },
                },
            ),
        ],
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> Vec<(String, Option<i128>)> {
    let mut rows = batches
        .iter()
        .flat_map(|batch| {
            let names = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let totals = batch
                .column(1)
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .unwrap();
            (0..batch.num_rows()).map(move |row| {
                (
                    names.value(row).to_owned(),
                    (!totals.is_null(row)).then(|| totals.value(row)),
                )
            })
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

#[tokio::test]
async fn nested_decimal_boolean_output_filter_matches_sql_and_direct() {
    let engine = engine();
    let result = compile_rows(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("expected compiled output filter: {:?}", result.outcome);
    };
    assert_eq!(
        query.sql().parameters(),
        &[decimal(4_000_000_000_000_000_000)]
    );
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
        ("b".into(), Some(5_000_000_000_000_000_000)),
        ("c".into(), None),
    ];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
}

#[tokio::test]
async fn output_filter_rejects_wrong_physical_type_and_unavailable_stage_slot() {
    let engine = engine();
    let mut wrong = query();
    wrong.requirements[2].operation = RowOperation::FilterOutput {
        stage: OutputFilterStage::AfterAggregate,
        predicate: compare("sum", Comparison::Gt, Literal::Int64(4)),
    };
    let result = compile_rows(&engine, wrong, CompileOptions::default()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "comparison_type")
    );

    let mut late = query();
    late.requirements[2].operation = RowOperation::FilterOutput {
        stage: OutputFilterStage::AfterAggregate,
        predicate: compare("later_window", Comparison::Gt, Literal::UInt64(1)),
    };
    let result = compile_rows(&engine, late, CompileOptions::default()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "output_filter_scope")
    );
}
