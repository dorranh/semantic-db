use std::sync::Arc;

use datafusion::arrow::{
    compute::cast,
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
    util::display::array_value_to_string,
};
use datafusion::datasource::MemTable;
use datafusion::prelude::SessionContext;
use semantic_catalog::{FactResolution, PublicationLimits};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_ossie::{OssieDocument, SourceBindings};
use semantic_plan::typed::RowQuery;
use serde_json::{Value, json};

async fn local_drilling(duplicate_well: bool) -> Engine {
    let mut bindings = SourceBindings::new();
    let wells = if duplicate_well {
        "(1,'North'),(1,'Duplicate'),(2,'South'),(3,'Unmeasured')"
    } else {
        "(1,'North'),(2,'South'),(3,'Unmeasured')"
    };
    for (source, sql) in [
        (
            "drilling.samples",
            "SELECT CAST(well_id AS BIGINT) AS well_id, CAST(sample_id AS BIGINT) AS sample_id, CAST(depth_m AS DOUBLE) AS drilled_m, CAST(duration_min AS DOUBLE) AS duration_min, CAST(load_kn AS DOUBLE) AS load_kn FROM (VALUES (1,1,10,2,10),(1,2,20,4,20),(2,1,5,1,30),(1,3,30,6,90),(1,4,0,1,NULL),(2,2,15,3,NULL)) AS t(well_id,sample_id,depth_m,duration_min,load_kn)".to_owned(),
        ),
        (
            "drilling.wells",
            format!("SELECT CAST(well_id AS BIGINT) AS well_id, basin FROM (VALUES {wells}) AS t(well_id,basin)"),
        ),
        (
            "drilling.totals",
            "SELECT CAST(well_id AS BIGINT) AS well_id, CAST(depth_m AS DOUBLE) AS drilled_m, CAST(duration_min AS DOUBLE) AS duration_min FROM (VALUES (1,30,6),(2,5,1),(1,30,7),(2,15,3)) AS t(well_id,depth_m,duration_min)".to_owned(),
        ),
        (
            "drilling.summary",
            "SELECT CAST(well_id AS BIGINT) AS well_id, CAST(depth_m AS DOUBLE) AS drilled_m, CAST(duration_min AS DOUBLE) AS duration_min, CAST(mean_load_kn AS DOUBLE) AS mean_load_kn, CAST(samples AS BIGINT) AS samples FROM (VALUES (1,60,13,40,4),(2,20,4,30,2)) AS t(well_id,depth_m,duration_min,mean_load_kn,samples)".to_owned(),
        ),
    ] {
        let frame = SessionContext::new().sql(&sql).await.unwrap();
        if source == "drilling.summary" {
            let batches = frame.collect().await.unwrap();
            let fields = batches[0].schema().fields().iter().enumerate().map(|(index, field)| {
                if index == 4 {
                    Field::new("samples", DataType::UInt64, field.is_nullable())
                } else {
                    field.as_ref().clone()
                }
            }).collect::<Vec<_>>();
            let schema = Arc::new(Schema::new(fields));
            let batches = batches.into_iter().map(|batch| {
                let mut columns = batch.columns().to_vec();
                columns[4] = cast(&columns[4], &DataType::UInt64).unwrap();
                RecordBatch::try_new(schema.clone(), columns).unwrap()
            }).collect::<Vec<_>>();
            bindings.bind(source, Arc::new(MemTable::try_new(schema, vec![batches]).unwrap())).unwrap();
        } else {
            bindings.bind(source, frame.into_view()).unwrap();
        }
    }
    let imported = OssieDocument::parse(include_str!(
        "../../../examples/clickhouse/drilling.ossie.yaml"
    ))
    .unwrap()
    .load(None, &bindings)
    .unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let samples = imported.engine.catalog().relation("samples").unwrap();
    let wells = imported.engine.catalog().relation("wells").unwrap();
    assert_eq!(
        imported
            .engine
            .catalog()
            .relation("summary")
            .unwrap()
            .schema
            .field_with_name("samples")
            .unwrap()
            .data_type(),
        &DataType::UInt64
    );
    for (relation, relationship) in [
        (samples, "sample_well"),
        (wells, "well_samples"),
        (wells, "well_summary"),
    ] {
        assert!(matches!(
            &relation.semantics.as_ref().unwrap().relationships[relationship].cardinality,
            FactResolution::Unknown
        ));
    }
    imported.engine
}

fn proposal(relation: &str, requirements: Vec<Value>) -> RowQuery {
    let requirements = requirements
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

async fn assert_both(
    engine: &Engine,
    proposal: RowQuery,
    expected: &[&[&str]],
) -> Vec<RecordBatch> {
    let result = compile_rows(engine, proposal, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("typed proposal rejected: {:?}", result.outcome)
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
    direct
}

#[tokio::test]
async fn authored_lookup_group_and_inner_enrichment_have_independent_rows() {
    let engine = local_drilling(false).await;
    let group = proposal(
        "samples",
        vec![
            json!({"kind":"lookup","relationship":"sample_well","role":"sample_well","instance":"w","field":"basin","alias":"basin","missing":"exclude","usage":"group"}),
            json!({"kind":"aggregate","function":"sum","field":{"instance":"r","field":"depth_m"},"distinct":false,"alias":"distance_m"}),
            json!({"kind":"aggregate","function":"count","field":null,"distinct":false,"alias":"samples"}),
            json!({"kind":"order_output","slot":"step0","direction":"asc","nulls":"last"}),
        ],
    );
    let result = assert_both(
        &engine,
        group,
        &[&["North", "60.0", "4"], &["South", "20.0", "2"]],
    )
    .await;
    let schema = result[0].schema();
    assert_eq!(schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(schema.field(1).data_type(), &DataType::Float64);
    assert_eq!(schema.field(2).data_type(), &DataType::Int64);

    let enrichment = proposal(
        "samples",
        vec![
            json!({"kind":"project","field":{"instance":"r","field":"sample_id"},"alias":"sample_id"}),
            json!({"kind":"lookup","relationship":"sample_well","role":"sample_well","instance":"w","field":"basin","alias":"basin","missing":"exclude"}),
            json!({"kind":"order","field":{"instance":"r","field":"well_id"},"direction":"asc","nulls":"last"}),
            json!({"kind":"order","field":{"instance":"r","field":"sample_id"},"direction":"asc","nulls":"last"}),
        ],
    );
    assert_both(
        &engine,
        enrichment,
        &[
            &["1", "North"],
            &["2", "North"],
            &["3", "North"],
            &["4", "North"],
            &["1", "South"],
            &["2", "South"],
        ],
    )
    .await;
}

#[tokio::test]
async fn left_lookup_preserves_unmeasured_well_and_related_absence() {
    let engine = local_drilling(false).await;
    let left = proposal(
        "wells",
        vec![
            json!({"kind":"project","field":{"instance":"r","field":"well_id"},"alias":"well_id"}),
            json!({"kind":"lookup","relationship":"well_summary","role":"well_summary","instance":"s","field":"depth_m","alias":"distance_m","missing":"null"}),
            json!({"kind":"lookup","relationship":"well_summary","role":"well_summary","instance":"mean","field":"mean_load_kn","alias":"mean_load_kn","missing":"null"}),
            json!({"kind":"order","field":{"instance":"r","field":"well_id"},"direction":"asc","nulls":"last"}),
        ],
    );
    let result = assert_both(
        &engine,
        left,
        &[
            &["1", "60.0", "40.0"],
            &["2", "20.0", "30.0"],
            &["3", "", ""],
        ],
    )
    .await;
    assert!(result[0].schema().field(1).is_nullable());
    assert!(result[0].schema().field(2).is_nullable());

    let absent = proposal(
        "wells",
        vec![
            json!({"kind":"project","field":{"instance":"r","field":"well_id"},"alias":"well_id"}),
            json!({"kind":"related","relationship":"well_samples","role":"well_samples","instance":"sample","mode":"absent","predicate":null}),
            json!({"kind":"order","field":{"instance":"r","field":"well_id"},"direction":"asc","nulls":"last"}),
        ],
    );
    assert_both(&engine, absent, &[&["3"]]).await;
}

#[tokio::test]
async fn duplicate_lookup_target_fails_execution_in_both_paths() {
    let engine = local_drilling(true).await;
    let proposal = proposal(
        "samples",
        vec![
            json!({"kind":"lookup","relationship":"sample_well","role":"sample_well","instance":"w","field":"basin","alias":"basin","missing":"exclude"}),
        ],
    );
    let result = compile_rows(&engine, proposal, CompileOptions::default()).await;
    assert!(!result.record.execution_obligations.is_empty());
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("duplicate-target query should compile with a runtime obligation")
    };
    assert!(
        query
            .plan_direct(&engine)
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
    assert!(
        query
            .execute(&engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
}
