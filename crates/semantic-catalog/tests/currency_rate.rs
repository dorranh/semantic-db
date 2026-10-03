use std::sync::Arc;

use arrow_schema::TimeUnit;
use semantic_catalog::{
    CURRENCY_RATE_VERSION, Catalog, CatalogMutation, ConversionRounding, CurrencyRateRule,
    DataType, Field, PublicationLimits, RateTimeBasis, Relation, RelationSemantics, Schema,
};

fn rule() -> CurrencyRateRule {
    CurrencyRateRule {
        version: CURRENCY_RATE_VERSION,
        id: "currency-rate/order-eur".into(),
        source_relation: "orders".into(),
        rate_relation: "exchange_rates".into(),
        source_amount_field: "amount_minor".into(),
        source_currency_field: "currency".into(),
        source_time_field: "booked_at".into(),
        to_currency: "EUR".into(),
        rate_from_currency_field: "from_currency".into(),
        rate_to_currency_field: "to_currency".into(),
        rate_valid_from_field: "valid_from".into(),
        rate_valid_to_field: "valid_to".into(),
        rate_numerator_field: "numerator".into(),
        rate_denominator_field: "denominator".into(),
        time_basis: RateTimeBasis::UtcInstantMicros,
        rounding: ConversionRounding::HalfEven,
        source_refs: vec![],
    }
}

fn relations() -> [Relation; 2] {
    let timestamp = DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()));
    let mut orders = Relation::base(
        "orders",
        Arc::new(Schema::new(vec![
            Field::new("amount_minor", DataType::Int64, false),
            Field::new("currency", DataType::Utf8, false),
            Field::new("booked_at", timestamp.clone(), false),
        ])),
        "memory",
    );
    orders.semantics = Some(RelationSemantics {
        currency_rates: [("to_eur".into(), rule())].into(),
        ..Default::default()
    });
    let rate = Relation::base(
        "exchange_rates",
        Arc::new(Schema::new(vec![
            Field::new("from_currency", DataType::Utf8, false),
            Field::new("to_currency", DataType::Utf8, false),
            Field::new("valid_from", timestamp.clone(), false),
            Field::new("valid_to", timestamp, false),
            Field::new("numerator", DataType::Int64, false),
            Field::new("denominator", DataType::Int64, false),
        ])),
        "memory",
    );
    [orders, rate]
}

#[test]
fn published_rate_rule_is_pinned_and_rate_changes_invalidate_source() {
    let [orders, rate] = relations();
    let mut catalog = Catalog::from_relations([orders, rate.clone()]).unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let snapshot = catalog.snapshot();
    let reference = snapshot
        .relation("orders")
        .unwrap()
        .definition_reference("currency_rate", "to_eur")
        .unwrap();
    assert_eq!(reference.id, "currency-rate/order-eur");

    let mut changed_rate = rate;
    changed_rate.description = Some("new rate revision".into());
    let report = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(changed_rate))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert!(report.changed.contains("exchange_rates"));
    assert!(report.affected.contains("orders"));
}

#[test]
fn rate_rule_requires_a_published_relation_and_exact_physical_types() {
    let [orders, mut rate] = relations();
    let missing = Catalog::from_relations([orders.clone()]).unwrap();
    assert!(missing.validate(&PublicationLimits::default()).is_err());
    rate.schema = Arc::new(Schema::new(vec![
        Field::new("from_currency", DataType::Utf8, false),
        Field::new("to_currency", DataType::Utf8, false),
        Field::new("valid_from", DataType::Date32, false),
        Field::new("valid_to", DataType::Date32, false),
        Field::new("numerator", DataType::Int64, false),
        Field::new("denominator", DataType::Int64, false),
    ]));
    let invalid = Catalog::from_relations([orders, rate]).unwrap();
    assert!(invalid.validate(&PublicationLimits::default()).is_err());
}

#[test]
fn business_date_basis_requires_a_real_iana_timezone() {
    let [mut orders, rate] = relations();
    orders
        .semantics
        .as_mut()
        .unwrap()
        .currency_rates
        .get_mut("to_eur")
        .unwrap()
        .time_basis = RateTimeBasis::BusinessDate {
        timezone: "Not/AZone".into(),
    };
    let invalid = Catalog::from_relations([orders, rate]).unwrap();
    assert!(invalid.validate(&PublicationLimits::default()).is_err());
}
