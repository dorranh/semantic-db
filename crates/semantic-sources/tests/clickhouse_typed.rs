#![cfg(feature = "clickhouse")]

#[path = "../../../tests/support/clickhouse.rs"]
mod clickhouse_fixture;

use clickhouse_fixture::{Database, PASSWORD};
use datafusion::arrow::{record_batch::RecordBatch, util::display::array_value_to_string};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::RowQuery;
use semantic_sources::{Project, ProjectConfig, Registry};
use serde_json::{Value, json};
use std::path::PathBuf;

fn proposal(relation: &str, operations: Vec<Value>) -> RowQuery {
    let requirements = operations
        .into_iter()
        .enumerate()
        .map(|(index, operation)| {
            json!({"id":format!("step{index}"),"source_text":format!("step {index}"),"operation":operation})
        })
        .collect::<Vec<_>>();
    serde_json::from_value(json!({
        "version":1,"input":{"relation":relation,"instance":"r"},
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

async fn assert_typed(engine: &Engine, proposal: RowQuery, expected: &[&[&str]]) {
    let result = compile_rows(engine, proposal, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("typed ClickHouse query rejected: {:?}", result.outcome)
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
#[ignore = "connector_integration: requires Docker; run explicitly with --ignored"]
async fn authored_typed_drilling_queries_run_on_clickhouse_with_federation_on_and_off() {
    let database = Database::start().await;
    for federation in [false, true] {
        let mut config: ProjectConfig = serde_saphyr::from_str(include_str!(
            "../../../examples/clickhouse/semantic-db.yaml"
        ))
        .unwrap();
        let connection = config.connections.get_mut("drilling").unwrap();
        connection
            .options
            .insert("endpoint".into(), json!(database.endpoint));
        connection
            .options
            .insert("federation".into(), json!(federation));
        let project = Project::new(
            config,
            semantic_ossie::OssieDocument::parse(include_str!(
                "../../../examples/clickhouse/drilling.ossie.yaml"
            ))
            .unwrap(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/clickhouse"),
        )
        .unwrap();
        let loaded = project
            .load(&Registry::standard(), &|name| {
                (name == "CLICKHOUSE_PASSWORD").then(|| PASSWORD.into())
            })
            .await
            .unwrap();
        let engine = &loaded.engine;
        assert_eq!(
            engine
                .catalog()
                .relation("summary")
                .unwrap()
                .schema
                .field_with_name("samples")
                .unwrap()
                .data_type(),
            &datafusion::arrow::datatypes::DataType::UInt64,
        );

        assert_typed(
            engine,
            proposal(
                "samples",
                vec![
                    json!({"kind":"lookup","relationship":"sample_well","role":"sample_well","instance":"w","field":"basin","alias":"basin","missing":"exclude","usage":"group"}),
                    json!({"kind":"aggregate","function":"sum","field":{"instance":"r","field":"depth_m"},"distinct":false,"alias":"distance_m"}),
                    json!({"kind":"aggregate","function":"count","field":null,"distinct":false,"alias":"samples"}),
                    json!({"kind":"order_output","slot":"step0","direction":"asc","nulls":"last"}),
                ],
            ),
            &[&["North", "60.0", "4"], &["South", "20.0", "2"]],
        )
        .await;
        assert_typed(
            engine,
            proposal(
                "wells",
                vec![
                    json!({"kind":"project","field":{"instance":"r","field":"well_id"},"alias":"well_id"}),
                    json!({"kind":"lookup","relationship":"well_summary","role":"well_summary","instance":"s","field":"mean_load_kn","alias":"mean_load_kn","missing":"null"}),
                    json!({"kind":"lookup","relationship":"well_summary","role":"well_summary","instance":"count","field":"samples","alias":"samples","missing":"null"}),
                    json!({"kind":"order","field":{"instance":"r","field":"well_id"},"direction":"asc","nulls":"last"}),
                ],
            ),
            &[&["1", "40.0", "4"], &["2", "30.0", "2"], &["3", "", ""]],
        )
        .await;
        assert_typed(
            engine,
            proposal(
                "wells",
                vec![
                    json!({"kind":"project","field":{"instance":"r","field":"well_id"},"alias":"well_id"}),
                    json!({"kind":"related","relationship":"well_samples","role":"well_samples","instance":"sample","mode":"absent","predicate":null}),
                    json!({"kind":"order","field":{"instance":"r","field":"well_id"},"direction":"asc","nulls":"last"}),
                ],
            ),
            &[&["3"]],
        )
        .await;
    }
}
