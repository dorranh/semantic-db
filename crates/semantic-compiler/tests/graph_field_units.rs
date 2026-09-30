use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, Decimal128Array, Int64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    FactResolution, FieldSemantics, Presence, Relation, RelationSemantics, SourceRef, Unit,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_graph};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::{graph::*, typed::*};

fn usd() -> Unit {
    Unit::Currency { code: "USD".into() }
}
fn eur() -> Unit {
    Unit::Currency { code: "EUR".into() }
}
fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(
        ["usd_a", "usd_b", "eur", "bare"]
            .into_iter()
            .map(|name| Field::new(name, DataType::Int64, false))
            .collect::<Vec<_>>(),
    ));
    let batch = RecordBatch::try_new(
        schema.clone(),
        [10, 2, 5, 4]
            .into_iter()
            .map(|value| Arc::new(Int64Array::from(vec![value])) as ArrayRef)
            .collect(),
    )
    .unwrap();
    let mut relation = Relation::base("amounts", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        fields: [("usd_a", usd()), ("usd_b", usd()), ("eur", eur())]
            .into_iter()
            .map(|(name, unit)| {
                (
                    name.into(),
                    FieldSemantics {
                        unit: Some(unit),
                        source_refs: vec![SourceRef {
                            artifact_revision: "authored-v1".into(),
                            path: format!("/fields/{name}/unit"),
                            span: None,
                        }],
                        ..Default::default()
                    },
                )
            })
            .collect(),
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

fn leaf(id: &str, fields: &[&str]) -> QueryNode {
    QueryNode {
        id: id.into(),
        source_text: format!("read {id}"),
        operation: GraphOperation::Rows {
            query: RowQuery {
                version: 1,
                input: RelationInput {
                    relation: "amounts".into(),
                    instance: "a".into(),
                },
                requirements: fields
                    .iter()
                    .map(|name| Requirement {
                        id: (*name).into(),
                        source_text: (*name).into(),
                        operation: RowOperation::Project {
                            field: FieldRef {
                                instance: "a".into(),
                                field: (*name).into(),
                            },
                            alias: (*name).into(),
                        },
                    })
                    .collect(),
                unresolved: vec![],
            },
        },
    }
}

fn ratio(denominator: &str, required_unit: Option<Unit>) -> GraphQuery {
    GraphQuery {
        version: 1,
        nodes: vec![
            leaf("source", &["usd_a", denominator]),
            QueryNode {
                id: "ratio".into(),
                source_text: "exact ratio".into(),
                operation: GraphOperation::Calculate {
                    input: "source".into(),
                    passthrough: vec![GraphProjection {
                        id: "amount".into(),
                        slot: "usd_a".into(),
                        alias: "amount".into(),
                    }],
                    ratios: vec![GraphRatio {
                        id: "share".into(),
                        numerator: "usd_a".into(),
                        denominator: denominator.into(),
                        required_unit,
                        zero: ZeroDivision::Null,
                        alias: "share".into(),
                    }],
                },
            },
        ],
        root: "ratio".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    }
}

fn quotient() -> Unit {
    Unit::Quotient {
        numerator: Box::new(usd()),
        denominator: Box::new(eur()),
    }
}

async fn assert_ratio(engine: &Engine, query: GraphQuery, unit: Unit, expected: i128) {
    let result = compile_graph(engine, query, CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("expected graph ratio: {:?}", result.outcome);
    };
    assert!(matches!(
        &query.slot_meaning("share").unwrap().unit,
        FactResolution::Known { value: Presence::Value(actual), .. } if actual == &unit
    ));
    let FactResolution::Known { contributors, .. } = &query.slot_meaning("amount").unwrap().unit
    else {
        panic!("projected authored unit must have evidence");
    };
    assert_eq!(contributors[0].origins[0].path, "/fields/usd_a/unit");
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
    for batches in [&direct, &sql] {
        let ratio = batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        assert_eq!(ratio.value(0), expected);
    }
}

#[tokio::test]
async fn authored_field_units_produce_exact_dimensionless_and_quotient_ratios() {
    let engine = fixture();
    assert_ratio(
        &engine,
        ratio("usd_b", Some(Unit::Dimensionless)),
        Unit::Dimensionless,
        5_000_000_000_000_000_000,
    )
    .await;
    assert_ratio(
        &engine,
        ratio("eur", Some(quotient())),
        quotient(),
        2_000_000_000_000_000_000,
    )
    .await;
}

#[tokio::test]
async fn grouped_source_field_retains_its_authored_unit_and_origin() {
    let engine = fixture();
    let query = GraphQuery {
        version: 1,
        nodes: vec![QueryNode {
            id: "grouped".into(),
            source_text: "group USD amount".into(),
            operation: GraphOperation::Rows {
                query: RowQuery {
                    version: 1,
                    input: RelationInput {
                        relation: "amounts".into(),
                        instance: "a".into(),
                    },
                    requirements: vec![
                        Requirement {
                            id: "amount".into(),
                            source_text: "amount".into(),
                            operation: RowOperation::Group {
                                field: FieldRef {
                                    instance: "a".into(),
                                    field: "usd_a".into(),
                                },
                                alias: "amount".into(),
                            },
                        },
                        Requirement {
                            id: "count".into(),
                            source_text: "count".into(),
                            operation: RowOperation::Aggregate {
                                function: AggregateFunction::Count,
                                field: None,
                                distinct: false,
                                alias: "count".into(),
                            },
                        },
                    ],
                    unresolved: vec![],
                },
            },
        }],
        root: "grouped".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    };
    let result = compile_graph(&engine, query, CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("expected grouped graph: {:?}", result.outcome);
    };
    let FactResolution::Known {
        value,
        contributors,
    } = &query.slot_meaning("amount").unwrap().unit
    else {
        panic!("grouped field must carry an authored unit");
    };
    assert_eq!(value, &Presence::Value(usd()));
    assert_eq!(contributors[0].origins[0].path, "/fields/usd_a/unit");
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
    assert_eq!(direct[0].num_rows(), 1);
    assert_eq!(sql[0].num_rows(), 1);
}

#[tokio::test]
async fn mismatched_or_unknown_field_units_cannot_claim_dimensionless() {
    let engine = fixture();
    for query in [
        ratio("eur", Some(Unit::Dimensionless)),
        ratio("bare", Some(Unit::Dimensionless)),
    ] {
        assert!(matches!(
            compile_graph(&engine, query, CompileOptions::default()).await.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_ratio_unit"
        ));
    }
    let result = compile_graph(&engine, ratio("bare", None), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("unknown unit should remain executable without a claimed unit");
    };
    assert!(matches!(
        query.slot_meaning("share").unwrap().unit,
        FactResolution::Unknown
    ));
}

fn set(right_field: &str) -> GraphQuery {
    GraphQuery {
        version: 1,
        nodes: vec![
            leaf("left", &["usd_a"]),
            leaf("right", &[right_field]),
            QueryNode {
                id: "set".into(),
                source_text: "align same typed amounts".into(),
                operation: GraphOperation::Set {
                    left: "left".into(),
                    right: "right".into(),
                    operator: SetOperator::Union,
                    duplicates: Duplicates::All,
                    columns: vec![SetColumn {
                        id: "amount".into(),
                        left: "usd_a".into(),
                        right: right_field.into(),
                        alias: "amount".into(),
                    }],
                },
            },
        ],
        root: "set".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    }
}

#[tokio::test]
async fn set_rejects_different_known_units_and_drops_known_plus_unknown_meaning() {
    let engine = fixture();
    assert!(matches!(
        compile_graph(&engine, set("eur"), CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "set_unit"
    ));

    let same = compile_graph(&engine, set("usd_b"), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query: same } = same.outcome else {
        panic!("identical authored units should align");
    };
    let FactResolution::Known {
        value,
        contributors,
    } = &same.slot_meaning("amount").unwrap().unit
    else {
        panic!("same units should retain provenance");
    };
    assert_eq!(value, &Presence::Value(usd()));
    assert_eq!(contributors.len(), 2);

    let result = compile_graph(&engine, set("bare"), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("a known/unknown alignment remains executable but semantically unknown");
    };
    assert!(matches!(
        query.slot_meaning("amount").unwrap().unit,
        FactResolution::Unknown
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
    let values = |batches: Vec<RecordBatch>| {
        let mut values = batches
            .iter()
            .flat_map(|batch| {
                let array = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| array.value(row))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        values.sort();
        values
    };
    assert_eq!(values(direct), vec![4, 10]);
    assert_eq!(values(sql), vec![4, 10]);
}
