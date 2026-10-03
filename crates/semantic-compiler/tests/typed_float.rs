use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Float64Array, Int16Array, Int64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::RowQuery;
use serde_json::{Value, json};

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("measurement", DataType::Float64, true),
        Field::new("whole", DataType::Int16, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
            Arc::new(Float64Array::from(vec![
                Some(1.5),
                None,
                Some(-0.0),
                Some(2.5),
            ])),
            Arc::new(Int16Array::from(vec![Some(1), Some(2), None, Some(2)])),
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("measurements", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn proposal(operations: Vec<Value>) -> RowQuery {
    let requirements = operations
        .into_iter()
        .enumerate()
        .map(|(index, operation)| {
            json!({"id":format!("step{index}"),"source_text":format!("step {index}"),"operation":operation})
        })
        .collect::<Vec<_>>();
    serde_json::from_value(json!({
        "version":1,"input":{"relation":"measurements","instance":"m"},
        "requirements":requirements,"unresolved":[]
    }))
    .unwrap()
}

fn rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
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

async fn assert_both(engine: &Engine, proposal: RowQuery, expected: &[&[&str]]) {
    let result = compile_rows(engine, proposal, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("numeric proposal rejected: {:?}", result.outcome)
    };
    let expected = expected
        .iter()
        .map(|row| row.iter().map(|value| (*value).to_owned()).collect())
        .collect::<Vec<Vec<String>>>();
    let direct = query
        .plan_direct(engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(rows(&direct), expected, "direct plan");
    assert_eq!(rows(&sql), expected, "SQL plan");
}

#[tokio::test]
async fn float_filter_sort_sum_min_max_and_average_have_expected_rows() {
    let engine = fixture();
    assert_both(
        &engine,
        proposal(vec![
            json!({"kind":"filter","predicate":{"kind":"compare","field":{"instance":"m","field":"measurement"},"operator":"gt_eq","value":{"type":"float64","value":1.5}}}),
            json!({"kind":"project","field":{"instance":"m","field":"id"},"alias":"id"}),
            json!({"kind":"order","field":{"instance":"m","field":"measurement"},"direction":"desc","nulls":"last"}),
        ]),
        &[&["4"], &["1"]],
    )
    .await;
    assert_both(
        &engine,
        proposal(vec![
            json!({"kind":"aggregate","function":"sum","field":{"instance":"m","field":"measurement"},"distinct":false,"alias":"total"}),
            json!({"kind":"aggregate","function":"avg","field":{"instance":"m","field":"measurement"},"distinct":false,"alias":"mean"}),
            json!({"kind":"aggregate","function":"min","field":{"instance":"m","field":"measurement"},"distinct":false,"alias":"minimum"}),
            json!({"kind":"aggregate","function":"max","field":{"instance":"m","field":"measurement"},"distinct":false,"alias":"maximum"}),
        ]),
        &[&["4.0", "1.3333333333333333", "-0.0", "2.5"]],
    )
    .await;
}

#[tokio::test]
async fn exact_integer_average_and_empty_float_aggregates() {
    let engine = fixture();
    assert_both(
        &engine,
        proposal(vec![json!({"kind":"aggregate","function":"avg","field":{"instance":"m","field":"whole"},"distinct":false,"alias":"mean"})]),
        &[&["1.666666666666666666"]],
    )
    .await;
    assert_both(
        &engine,
        proposal(vec![
            json!({"kind":"filter","predicate":{"kind":"compare","field":{"instance":"m","field":"id"},"operator":"eq","value":{"type":"int64","value":99}}}),
            json!({"kind":"aggregate","function":"sum","field":{"instance":"m","field":"measurement"},"distinct":false,"alias":"total"}),
            json!({"kind":"aggregate","function":"avg","field":{"instance":"m","field":"measurement"},"distinct":false,"alias":"mean"}),
        ]),
        &[&["", ""]],
    )
    .await;
}

#[tokio::test]
async fn nonfinite_literal_and_distinct_exact_integer_average_are_rejected() {
    let engine = fixture();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut query = proposal(vec![
            json!({"kind":"filter","predicate":{"kind":"compare","field":{"instance":"m","field":"measurement"},"operator":"eq","value":{"type":"float64","value":1.0}}}),
        ]);
        let semantic_plan::typed::RowOperation::Filter { predicate } =
            &mut query.requirements[0].operation
        else {
            unreachable!()
        };
        let semantic_plan::typed::RowPredicate::Compare { value: literal, .. } = predicate else {
            unreachable!()
        };
        *literal = semantic_plan::typed::Literal::Float64(value);
        let result = compile_rows(&engine, query, CompileOptions::default()).await;
        assert!(
            matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "float_literal")
        );
    }
    let distinct = proposal(vec![
        json!({"kind":"aggregate","function":"avg","field":{"instance":"m","field":"whole"},"distinct":true,"alias":"mean"}),
    ]);
    let result = compile_rows(&engine, distinct, CompileOptions::default()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "aggregate_arguments")
    );
}
