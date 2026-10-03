use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, BooleanArray, Decimal128Array, Int64Array, StringArray,
            TimestampMicrosecondArray,
        },
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    CURRENCY_RATE_VERSION, ConversionRounding, CurrencyRateRule, GovernedFilter, RateTimeBasis,
    Relation, RelationSemantics, RowPolicy,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    Comparison, Literal, RelationInput, Requirement, RowOperation, RowQuery,
};

#[derive(Clone, Copy)]
enum Case {
    Complete,
    Missing,
    Duplicate,
    PolicyFiltered,
    ZeroFactor,
    NegativeFactor,
}

fn visible_policy(name: &str) -> RowPolicy {
    RowPolicy {
        id: name.into(),
        filters: vec![GovernedFilter {
            field: "visible".into(),
            operator: Comparison::Eq,
            value: Literal::Boolean(true),
        }],
        source_refs: vec![],
    }
}

fn fixture(case: Case) -> Engine {
    let mut engine = Engine::new();
    let time_type = DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()));
    let rate_schema = Arc::new(Schema::new(vec![
        Field::new("from_currency", DataType::Utf8, false),
        Field::new("to_currency", DataType::Utf8, false),
        Field::new("valid_from", time_type.clone(), false),
        Field::new("valid_to", time_type.clone(), false),
        Field::new("numerator", DataType::Int64, false),
        Field::new("denominator", DataType::Int64, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let mut starts = vec![0, 20];
    let mut ends = vec![20, 30];
    let mut numerators = vec![2, 3];
    let mut denominators = vec![3, 2];
    let mut visibility = vec![true, true];
    match case {
        Case::Missing => {
            starts.pop();
            ends.pop();
            numerators.pop();
            denominators.pop();
            visibility.pop();
        }
        Case::Duplicate => {
            starts.push(15);
            ends.push(25);
            numerators.push(7);
            denominators.push(4);
            visibility.push(true);
        }
        Case::PolicyFiltered => visibility[1] = false,
        Case::ZeroFactor => denominators[1] = 0,
        Case::NegativeFactor => numerators[1] = -3,
        Case::Complete => {}
    }
    let n = starts.len();
    let rates = RecordBatch::try_new(
        rate_schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["USD"; n])) as ArrayRef,
            Arc::new(StringArray::from(vec!["CHF"; n])),
            Arc::new(TimestampMicrosecondArray::from(starts).with_timezone("UTC")),
            Arc::new(TimestampMicrosecondArray::from(ends).with_timezone("UTC")),
            Arc::new(Int64Array::from(numerators)),
            Arc::new(Int64Array::from(denominators)),
            Arc::new(BooleanArray::from(visibility)),
        ],
    )
    .unwrap();
    let mut rate_relation = Relation::base("currency_rates", rate_schema.clone(), "memory");
    rate_relation.semantics = Some(RelationSemantics {
        row_policies: vec![visible_policy("policies/rate-visible")],
        ..Default::default()
    });
    engine
        .register_table(
            rate_relation,
            Arc::new(MemTable::try_new(rate_schema, vec![vec![rates]]).unwrap()),
        )
        .unwrap();
    let source_schema = Arc::new(Schema::new(vec![
        Field::new("amount", DataType::Int64, true),
        Field::new("currency", DataType::Utf8, false),
        Field::new("as_of", time_type, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let source = RecordBatch::try_new(
        source_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![Some(3), Some(2), None, Some(10)])) as ArrayRef,
            Arc::new(StringArray::from(vec!["USD", "USD", "USD", "USD"])),
            Arc::new(TimestampMicrosecondArray::from(vec![10, 20, 10, 40]).with_timezone("UTC")),
            Arc::new(BooleanArray::from(vec![true, true, true, false])),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("payments", source_schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        currency_rates: [(
            "chf".into(),
            CurrencyRateRule {
                version: CURRENCY_RATE_VERSION,
                id: "currency_rate/payments-chf".into(),
                source_relation: "payments".into(),
                rate_relation: "currency_rates".into(),
                source_amount_field: "amount".into(),
                source_currency_field: "currency".into(),
                source_time_field: "as_of".into(),
                to_currency: "CHF".into(),
                rate_from_currency_field: "from_currency".into(),
                rate_to_currency_field: "to_currency".into(),
                rate_valid_from_field: "valid_from".into(),
                rate_valid_to_field: "valid_to".into(),
                rate_numerator_field: "numerator".into(),
                rate_denominator_field: "denominator".into(),
                time_basis: RateTimeBasis::UtcInstantMicros,
                rounding: ConversionRounding::HalfEven,
                source_refs: vec![],
            },
        )]
        .into(),
        row_policies: vec![visible_policy("policies/payment-visible")],
        ..Default::default()
    });
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(source_schema, vec![vec![source]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query(rule: &str) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "payments".into(),
            instance: "p".into(),
        },
        requirements: vec![Requirement {
            id: "converted".into(),
            source_text: "Convert payments to CHF at their timestamp".into(),
            operation: RowOperation::CurrencyConvert {
                rate: rule.into(),
                alias: "chf".into(),
            },
        }],
        unresolved: vec![],
    }
}

fn coefficients(batches: &[RecordBatch]) -> Vec<Option<i128>> {
    let mut values = batches
        .iter()
        .flat_map(|batch| {
            let array = batch
                .column(0)
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .unwrap();
            (0..array.len())
                .map(|index| (!array.is_null(index)).then(|| array.value(index)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    values.sort();
    values
}

#[tokio::test]
async fn dated_currency_rates_match_half_open_intervals_with_sql_direct_parity() {
    let engine = fixture(Case::Complete);
    let result = compile_rows(&engine, query("chf"), CompileOptions::default()).await;
    assert_eq!(result.record.execution_obligations.len(), 1);
    assert!(
        result
            .record
            .definition_refs
            .iter()
            .any(|reference| reference.id == "currency_rate/payments-chf")
    );
    let query = match result.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("{other:?}"),
    };
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
        None,
        Some(2_000_000_000_000_000_000),
        Some(3_000_000_000_000_000_000),
    ];
    assert_eq!(coefficients(&direct), expected);
    assert_eq!(coefficients(&sql), expected);
}

#[tokio::test]
async fn missing_duplicate_and_invalid_policy_visible_rates_fail_at_execution() {
    for case in [
        Case::Missing,
        Case::Duplicate,
        Case::PolicyFiltered,
        Case::ZeroFactor,
        Case::NegativeFactor,
    ] {
        let engine = fixture(case);
        let result = compile_rows(&engine, query("chf"), CompileOptions::default()).await;
        let query = match result.outcome {
            TypedOutcome::Compiled { query } => query,
            other => panic!("{other:?}"),
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
}

#[tokio::test]
async fn unknown_rate_is_rejected() {
    let result = compile_rows(
        &fixture(Case::Complete),
        query("other"),
        CompileOptions::default(),
    )
    .await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "unknown_currency_rate")
    );
}
