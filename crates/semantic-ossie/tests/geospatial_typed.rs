use datafusion::arrow::{record_batch::RecordBatch, util::display::array_value_to_string};
use semantic_catalog::PublicationLimits;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_ossie::{OssieDocument, SourceBindings};
use semantic_plan::typed::RowQuery;
use serde_json::{Value, json};

async fn wells() -> Engine {
    let mut sources = SourceBindings::new();
    sources
        .bind_csv(
            "fixtures.geospatial.wells",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/geospatial/wells.csv"
            ),
        )
        .await
        .unwrap();
    let engine = OssieDocument::parse(include_str!(
        "../../../examples/geospatial/wells.ossie.yaml"
    ))
    .unwrap()
    .load(None, &sources)
    .unwrap()
    .engine;
    engine
        .catalog()
        .validate(&PublicationLimits::default())
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
        "version":1,"input":{"relation":"wells","instance":"w"},
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
        panic!("geospatial query rejected: {:?}", result.outcome)
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
async fn wells_have_exact_count_extrema_and_float_mean() {
    let engine = wells().await;
    assert_both(
        &engine,
        proposal(vec![json!({"kind":"aggregate","function":"count","field":null,"distinct":false,"alias":"count"})]),
        &[&["5"]],
    )
    .await;
    assert_both(
        &engine,
        proposal(vec![
            json!({"kind":"project","field":{"instance":"w","field":"well_name"},"alias":"name"}),
            json!({"kind":"project","field":{"instance":"w","field":"latitude_deg"},"alias":"latitude"}),
            json!({"kind":"order","field":{"instance":"w","field":"latitude_deg"},"direction":"desc","nulls":"last"}),
            json!({"kind":"limit","count":1}),
        ]),
        &[&["Birch-4", "56.25"]],
    )
    .await;
    assert_both(
        &engine,
        proposal(vec![
            json!({"kind":"project","field":{"instance":"w","field":"well_name"},"alias":"name"}),
            json!({"kind":"project","field":{"instance":"w","field":"total_depth_m"},"alias":"depth"}),
            json!({"kind":"order","field":{"instance":"w","field":"total_depth_m"},"direction":"desc","nulls":"last"}),
            json!({"kind":"limit","count":1}),
        ]),
        &[&["Willow-3", "4100.0"]],
    )
    .await;
    assert_both(
        &engine,
        proposal(vec![json!({"kind":"aggregate","function":"avg","field":{"instance":"w","field":"total_depth_m"},"distinct":false,"alias":"mean_depth_m"})]),
        &[&["2870.0"]],
    )
    .await;
}

#[tokio::test]
async fn basin_status_and_depth_filters_preserve_exact_wells() {
    let engine = wells().await;
    let query = proposal(vec![
        json!({"kind":"filter","predicate":{"kind":"all","predicates":[
            {"kind":"compare","field":{"instance":"w","field":"basin"},"operator":"eq","value":{"type":"utf8","value":"North Basin"}},
            {"kind":"compare","field":{"instance":"w","field":"status"},"operator":"eq","value":{"type":"utf8","value":"active"}},
            {"kind":"compare","field":{"instance":"w","field":"total_depth_m"},"operator":"gt_eq","value":{"type":"float64","value":2500.0}}
        ]}}),
        json!({"kind":"project","field":{"instance":"w","field":"well_id"},"alias":"well_id"}),
        json!({"kind":"order","field":{"instance":"w","field":"well_id"},"direction":"asc","nulls":"last"}),
    ]);
    assert_both(&engine, query, &[&["W-001"], &["W-004"]]).await;
}
