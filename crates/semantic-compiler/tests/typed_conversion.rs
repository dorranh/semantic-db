use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Decimal128Array, Int64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    ConversionRounding, Relation, RelationSemantics, UNIT_CONVERSION_VERSION, Unit, UnitConversion,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

fn engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "amount",
        DataType::Int64,
        true,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int64Array::from(vec![Some(1), Some(2), Some(-1), None])) as ArrayRef],
    )
    .unwrap();
    let mut relation = Relation::base("amounts", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        conversions: [(
            "thirds".into(),
            UnitConversion {
                version: UNIT_CONVERSION_VERSION,
                id: "conversion/thirds".into(),
                field: "amount".into(),
                from_unit: Unit::Named { id: "whole".into() },
                to_unit: Unit::Named { id: "third".into() },
                numerator: 1,
                denominator: 3,
                rounding: ConversionRounding::HalfEven,
                source_refs: vec![],
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

fn query(conversion: &str) -> RowQuery {
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "amounts".into(),
            instance: "a".into(),
        },
        requirements: vec![Requirement {
            id: "converted".into(),
            source_text: "convert the whole amount to thirds".into(),
            operation: RowOperation::Convert {
                conversion: conversion.into(),
                alias: "thirds".into(),
            },
        }],
        unresolved: vec![],
    }
}

fn coefficients(batches: &[RecordBatch]) -> Vec<Option<i128>> {
    batches
        .iter()
        .flat_map(|batch| {
            let values = batch
                .column(0)
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .unwrap();
            (0..values.len()).map(|index| (!values.is_null(index)).then(|| values.value(index)))
        })
        .collect()
}

#[tokio::test]
async fn authored_conversion_executes_exactly_in_sql_and_direct_paths() {
    let engine = engine();
    let result = compile_rows(
        &engine,
        query("conversion/thirds"),
        CompileOptions::default(),
    )
    .await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome);
    };
    assert!(query.sql().statement().contains("semantic_scale_i64_v1"));
    assert!(
        result
            .record
            .definition_refs
            .iter()
            .any(|reference| reference.id == "conversion/thirds")
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
        Some(333_333_333_333_333_333),
        Some(666_666_666_666_666_667),
        Some(-333_333_333_333_333_333),
        None,
    ];
    assert_eq!(coefficients(&direct), expected);
    assert_eq!(coefficients(&emitted), expected);
}

#[tokio::test]
async fn unknown_conversion_is_rejected_with_exact_diagnostic() {
    let result = compile_rows(&engine(), query("other"), CompileOptions::default()).await;
    let TypedOutcome::Rejected { diagnostic } = result.outcome else {
        panic!("{:?}", result.outcome);
    };
    assert_eq!(diagnostic.code, "unknown_conversion");
}
