use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("tenant", DataType::Utf8, false),
        Field::new("period", DataType::Int64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5, 6, 7])) as ArrayRef,
            Arc::new(StringArray::from(vec!["A", "A", "A", "A", "A", "B", "B"])),
            Arc::new(Int64Array::from(vec![1, 1, 1, 2, 2, 1, 1])),
            Arc::new(StringArray::from(vec!["a", "a", "b", "a", "b", "a", "b"])),
            Arc::new(Int64Array::from(vec![
                Some(4),
                Some(6),
                Some(20),
                None,
                Some(-5),
                Some(7),
                Some(3),
            ])),
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("facts", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn field(name: &str) -> FieldRef {
    FieldRef {
        instance: "f".into(),
        field: name.into(),
    }
}

fn requirement(id: &str, operation: RowOperation) -> Requirement {
    Requirement {
        id: id.into(),
        source_text: id.into(),
        operation,
    }
}

fn output(slot: &str) -> WindowInput {
    WindowInput::Output { slot: slot.into() }
}

fn window_order(slot: &str) -> WindowOrder {
    WindowOrder {
        input: output(slot),
        direction: Direction::Asc,
        nulls: NullOrder::Last,
    }
}

fn query() -> RowQuery {
    let mut requirements = vec![];
    for name in ["tenant", "period", "category"] {
        requirements.push(requirement(
            name,
            RowOperation::Group {
                field: field(name),
                alias: name.into(),
            },
        ));
    }
    requirements.push(requirement(
        "amount",
        RowOperation::Aggregate {
            function: AggregateFunction::Sum,
            field: Some(field("amount")),
            distinct: false,
            alias: "amount".into(),
        },
    ));
    requirements.push(requirement(
        "running",
        RowOperation::Window {
            window: WindowSpec {
                function: WindowFunction::Sum,
                input: Some(output("amount")),
                partition_by: vec![output("tenant")],
                order_by: vec![window_order("period"), window_order("category")],
                frame: WindowFrame::RowsThroughCurrent,
            },
            alias: "running".into(),
        },
    ));
    for name in ["tenant", "period", "category"] {
        requirements.push(requirement(
            &format!("sort_{name}"),
            RowOperation::OrderOutput {
                slot: name.into(),
                direction: Direction::Asc,
                nulls: NullOrder::Last,
            },
        ));
    }
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "facts".into(),
            instance: "f".into(),
        },
        requirements,
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| {
                (0..batch.num_columns())
                    .map(|column| array_value_to_string(batch.column(column), row).unwrap())
                    .collect()
            })
        })
        .collect()
}

#[tokio::test]
async fn grouped_cumulative_rows_have_strict_tie_order_and_partition_reset() {
    let engine = fixture();
    let compilation = compile_rows(&engine, query(), CompileOptions::default()).await;
    let query = match compilation.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("{other:?}"),
    };
    assert!(
        query
            .sql()
            .statement()
            .contains("ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW")
    );
    let direct = query
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let emitted = query
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = vec![
        vec!["A", "1", "a", "10", "10"],
        vec!["A", "1", "b", "20", "30"],
        vec!["A", "2", "a", "", "30"],
        vec!["A", "2", "b", "-5", "25"],
        vec!["B", "1", "a", "7", "7"],
        vec!["B", "1", "b", "3", "10"],
    ];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&emitted), expected);
}

#[tokio::test]
async fn cumulative_rejects_missing_tie_key_row_grain_and_nonadditive_state() {
    let engine = fixture();
    let mut missing_tie = query();
    let RowOperation::Window { window, .. } = &mut missing_tie.requirements[4].operation else {
        unreachable!()
    };
    window.order_by.pop();
    let result = compile_rows(&engine, missing_tie, CompileOptions::default()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "cumulative_order")
    );

    let row_grain = RowQuery {
        requirements: vec![requirement(
            "running",
            RowOperation::Window {
                window: WindowSpec {
                    function: WindowFunction::Sum,
                    input: Some(WindowInput::Field {
                        field: field("amount"),
                    }),
                    partition_by: vec![],
                    order_by: vec![WindowOrder {
                        input: WindowInput::Field { field: field("id") },
                        direction: Direction::Asc,
                        nulls: NullOrder::Last,
                    }],
                    frame: WindowFrame::RowsThroughCurrent,
                },
                alias: "running".into(),
            },
        )],
        ..query()
    };
    let result = compile_rows(&engine, row_grain, CompileOptions::default()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "cumulative_frame")
    );

    let mut nonadditive = query();
    nonadditive.requirements[3].operation = RowOperation::Aggregate {
        function: AggregateFunction::Sum,
        field: Some(field("amount")),
        distinct: true,
        alias: "amount".into(),
    };
    let result = compile_rows(&engine, nonadditive, CompileOptions::default()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_rollup")
    );
}
