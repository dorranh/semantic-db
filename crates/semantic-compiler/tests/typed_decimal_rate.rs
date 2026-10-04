use datafusion::{
    arrow::{
        array::{ArrayRef, Date32Array, Decimal128Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    ExactDecimalRateRule, GovernedFilter, NullRatePolicy, Relation, RelationSemantics, RowPolicy,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;
use serde_json::json;
use std::sync::Arc;
fn engine(policy: bool) -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("currency", DataType::Utf8, false),
        Field::new("day", DataType::Date32, false),
        Field::new("rate", DataType::Decimal128(18, 6), false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["USD", "EUR", "GBP"])) as ArrayRef,
            Arc::new(Date32Array::from(vec![19814, 19814, 19814])),
            Arc::new(
                Decimal128Array::from(vec![950050, 1050050, 1150050])
                    .with_precision_and_scale(18, 6)
                    .unwrap(),
            ),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("rates", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        exact_decimal_rates: [(
            "chf".into(),
            ExactDecimalRateRule {
                version: 1,
                id: "rates/chf".into(),
                rate_relation: "rates".into(),
                source_currency_field: "currency".into(),
                date_field: "day".into(),
                rate_field: "rate".into(),
                target_currency: "CHF".into(),
                positive_only: true,
                null_rate: NullRatePolicy::Unavailable,
                source_refs: vec![],
            },
        )]
        .into(),
        row_policies: if policy {
            vec![RowPolicy {
                id: "visible/usd".into(),
                filters: vec![GovernedFilter {
                    field: "currency".into(),
                    operator: Comparison::Eq,
                    value: Literal::Utf8("USD".into()),
                }],
                source_refs: vec![],
            }]
        } else {
            vec![]
        },
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
fn query(coefficient: &str) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "rates".into(),
            instance: "r".into(),
        },
        requirements: vec![Requirement {
            id: "money".into(),
            source_text: "convert the stated amount to target currency".into(),
            operation: RowOperation::ConvertRate {
                rate: "chf".into(),
                amount: RateAmount::Literal {
                    value: Literal::Decimal128 {
                        coefficient: coefficient.into(),
                        precision: 8,
                        scale: 2,
                    },
                    unit: RateAmountUnit::RateSourceCurrency,
                    basis: RateAmountBasis::Major,
                },
                result_type: DecimalResultType {
                    precision: 18,
                    scale: 2,
                },
                rounding: DecimalRounding::HalfAwayFromZero,
                alias: "converted".into(),
            },
        }],
        unresolved: vec![],
    }
}
fn rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    batches
        .iter()
        .flat_map(|b| {
            (0..b.num_rows()).map(|r| {
                (0..b.num_columns())
                    .map(|c| array_value_to_string(b.column(c), r).unwrap())
                    .collect()
            })
        })
        .collect()
}
#[tokio::test]
async fn literal_rates_preserve_exact_signed_ties_sql_parameters_and_contract_pins() {
    for (amount, expected) in [
        ("10000", vec!["95.01", "105.01", "115.01"]),
        ("-10000", vec!["-95.01", "-105.01", "-115.01"]),
    ] {
        let engine = engine(false);
        let result = compile_rows(&engine, query(amount), CompileOptions::default()).await;
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
        assert!(
            result
                .record
                .definition_refs
                .iter()
                .any(|r| r.id == "rates/chf")
        );
        assert!(
            result
                .record
                .definition_refs
                .iter()
                .any(|r| r.id == "functions/semantic_decimal_rate_v1" && r.revision == "2")
        );
        assert_eq!(
            query.sql().parameters(),
            &[Literal::Decimal128 {
                coefficient: amount.into(),
                precision: 8,
                scale: 2
            }]
        );
        let native = query
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
        assert_eq!(native[0].schema(), sql[0].schema());
        assert_eq!(native[0].schema().field(0).name(), "converted");
        assert!(native[0].schema().field(0).is_nullable());
        assert_eq!(
            native[0].schema().field(0).data_type(),
            &DataType::Decimal128(18, 2)
        );
        assert_eq!(
            rows(&native),
            expected
                .into_iter()
                .map(|v| vec![v.to_string()])
                .collect::<Vec<_>>()
        );
        assert_eq!(rows(&native), rows(&sql));
        let bound = serde_json::to_value(query.bound()).unwrap();
        let meaning: semantic_catalog::SlotMeaning =
            serde_json::from_value(bound["output_meanings"]["money"].clone()).unwrap();
        assert!(
            matches!(meaning.unit,semantic_catalog::FactResolution::Known { value: semantic_catalog::Presence::Value(semantic_catalog::Unit::Currency{code}),.. } if code=="CHF")
        );
    }
}
#[tokio::test]
async fn rate_policies_apply_before_conversion_and_scope_type_alias_contracts_fail_closed() {
    let engine = engine(true);
    let result = compile_rows(&engine, query("10000"), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: compiled } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    assert_eq!(
        rows(
            &compiled
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()
        ),
        vec![vec!["95.01"]]
    );
    for mutation in 0..4 {
        let mut q = query("10000");
        let mut options = CompileOptions::default();
        let RowOperation::ConvertRate {
            amount,
            result_type,
            rate,
            alias,
            ..
        } = &mut q.requirements[0].operation
        else {
            unreachable!()
        };
        match mutation {
            0 => {
                *amount = RateAmount::Literal {
                    value: Literal::Float64(100.0),
                    unit: RateAmountUnit::RateSourceCurrency,
                    basis: RateAmountBasis::Major,
                }
            }
            1 => result_type.scale = 19,
            2 => *rate = "unknown".into(),
            _ => *alias = "".into(),
        };
        assert!(matches!(
            compile_rows(&engine, q, options.clone()).await.outcome,
            TypedOutcome::Rejected { .. }
        ));
        options.allowed_relations = Some(["elsewhere".into()].into());
        assert!(!matches!(
            compile_rows(&engine, query("10000"), options).await.outcome,
            TypedOutcome::Compiled { .. }
        ));
    }
}
#[test]
fn reserved_amount_modes_and_unknown_fields_are_not_wire_contracts() {
    let mut value = serde_json::to_value(query("10000")).unwrap();
    for (key, replacement) in [
        ("kind", json!("field")),
        ("basis", json!("minor")),
        ("unit", json!({"kind":"currency","code":"USD"})),
    ] {
        let mut bad = value.clone();
        bad["requirements"][0]["operation"]["amount"][key] = replacement;
        assert!(serde_json::from_value::<RowQuery>(bad).is_err());
    }
    value["requirements"][0]["operation"]["result_type"]["extra"] = json!(true);
    assert!(serde_json::from_value::<RowQuery>(value).is_err());
}
#[tokio::test]
async fn rate_leaf_retains_target_currency_meaning_in_graph_slots() {
    use semantic_plan::graph::{GraphOperation, GraphQuery, QueryNode};
    let engine = engine(false);
    let graph = GraphQuery {
        version: 1,
        nodes: vec![QueryNode {
            id: "rates".into(),
            source_text: "convert the amount".into(),
            operation: GraphOperation::Rows {
                query: query("10000"),
            },
        }],
        root: "rates".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    };
    let result =
        semantic_compiler::typed::compile_graph(&engine, graph, CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    assert!(
        matches!(&query.slot_meaning("money").unwrap().unit,semantic_catalog::FactResolution::Known{value:semantic_catalog::Presence::Value(semantic_catalog::Unit::Currency{code}),..} if code=="CHF")
    );
}
#[tokio::test]
async fn rate_amount_literal_and_unbound_parameter_share_nullable_metadata() {
    let engine = engine(false);
    let literal=engine.plan_generated_sql("SELECT semantic_decimal_rate_v1(CAST(100 AS DECIMAL(8,0)), rate, 18, 2, 2) AS money FROM rates").await.unwrap();
    let parameter=engine.plan_generated_sql("SELECT semantic_decimal_rate_v1(CAST($1 AS DECIMAL(8,0)), rate, 18, 2, 2) AS money FROM rates").await.unwrap();
    assert!(
        literal.schema().field(0).is_nullable(),
        "literal metadata must match the parameter contract"
    );
    assert!(
        parameter.schema().field(0).is_nullable(),
        "unbound parameter metadata unexpectedly nonnullable"
    );
    assert_eq!(
        literal.schema().field(0).data_type(),
        parameter.schema().field(0).data_type()
    );
}
#[tokio::test]
async fn empty_rate_population_retains_typed_nullable_output_schema() {
    let engine = engine(false);
    let mut proposal = query("10000");
    proposal.requirements.insert(
        0,
        Requirement {
            id: "empty".into(),
            source_text: "select the absent currency".into(),
            operation: RowOperation::Filter {
                predicate: RowPredicate::Compare {
                    field: FieldRef {
                        instance: "r".into(),
                        field: "currency".into(),
                    },
                    operator: Comparison::Eq,
                    value: Literal::Utf8("absent".into()),
                },
            },
        },
    );
    let result = compile_rows(&engine, proposal, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let direct = query.plan_direct(&engine).await.unwrap();
    assert_eq!(
        direct.schema().field(0).data_type(),
        &DataType::Decimal128(18, 2)
    );
    assert!(direct.schema().field(0).is_nullable());
    assert_eq!(query.sql().expected_output()[0].name, "converted");
    assert!(query.sql().expected_output()[0].nullable);
    for batches in [
        direct.collect().await.unwrap(),
        query
            .execute(&engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
    ] {
        assert!(batches.iter().all(|batch| batch.num_rows() == 0));
    }
}
