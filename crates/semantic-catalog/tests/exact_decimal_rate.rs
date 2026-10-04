use semantic_catalog::{
    Catalog, DataType, ExactDecimalRateRule, Field, NullRatePolicy, PublicationLimits, Relation,
    RelationSemantics, Schema,
};
use std::sync::Arc;
fn rule() -> ExactDecimalRateRule {
    ExactDecimalRateRule {
        version: 1,
        id: "rates/exact".into(),
        rate_relation: "rates".into(),
        source_currency_field: "currency".into(),
        date_field: "date".into(),
        rate_field: "value".into(),
        target_currency: "CHF".into(),
        positive_only: true,
        null_rate: NullRatePolicy::Unavailable,
        source_refs: vec![],
    }
}
fn catalog(currency_nullable: bool, date_nullable: bool, rate_type: DataType) -> Catalog {
    let mut catalog = Catalog::default();
    let mut relation = Relation::base(
        "rates",
        Arc::new(Schema::new(vec![
            Field::new("currency", DataType::Utf8, currency_nullable),
            Field::new("date", DataType::Date32, date_nullable),
            Field::new("value", rate_type, true),
        ])),
        "memory",
    );
    relation.semantics = Some(RelationSemantics {
        exact_decimal_rates: [("exact".into(), rule())].into(),
        ..Default::default()
    });
    catalog.register(relation).unwrap();
    catalog
}
#[test]
fn exact_decimal_rate_publishes_real_types_and_pins_contract() {
    let catalog = catalog(false, false, DataType::Decimal128(18, 6));
    catalog.validate(&PublicationLimits::default()).unwrap();
    let snapshot = catalog.snapshot();
    assert_eq!(
        snapshot
            .relation("rates")
            .unwrap()
            .definition_reference("exact_decimal_rate", "exact")
            .unwrap(),
        &rule().reference()
    );
    assert!(
        rule()
            .validate(&snapshot, Some(&["elsewhere".into()].into()))
            .is_err()
    );
    let mut invalid = rule();
    invalid.positive_only = false;
    assert!(invalid.validate_contract().is_err());
    invalid = rule();
    invalid.version = 2;
    assert!(invalid.validate_contract().is_err());
}
#[test]
fn nullable_unit_date_or_inexact_rate_contracts_are_rejected() {
    for catalog in [
        catalog(true, false, DataType::Decimal128(18, 6)),
        catalog(false, true, DataType::Decimal128(18, 6)),
        catalog(false, false, DataType::Float64),
        catalog(false, false, DataType::Decimal128(18, -1)),
    ] {
        assert!(catalog.validate(&PublicationLimits::default()).is_err());
    }
}
