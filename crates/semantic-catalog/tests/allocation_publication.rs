use std::sync::Arc;

use semantic_catalog::{
    ALLOCATION_VERSION, AllocationConservation, AllocationContract, AllocationDenominator,
    AllocationEligibility, AllocationRounding, Catalog, CatalogMutation, DataType, Field,
    NullWeight, PublicationLimits, Relation, RelationSemantics, Schema, ZeroDenominator,
};

fn rule() -> AllocationContract {
    AllocationContract {
        version: ALLOCATION_VERSION,
        id: "allocation/order-category".into(),
        source_relation: "orders".into(),
        bridge_relation: "order_category".into(),
        source_entity_fields: vec!["id".into()],
        bridge_source_fields: vec!["order_id".into()],
        target_dimensions: vec!["category".into()],
        source_amount_field: "amount_minor".into(),
        amount_unit: "minor_currency_unit".into(),
        weight_field: "weight".into(),
        eligible_population: AllocationEligibility::AllRows,
        expected_membership_count_field: "expected_count".into(),
        expected_weight_total_field: "expected_weight".into(),
        denominator: AllocationDenominator::SumEligibleWeightsPerSource,
        null_weight: NullWeight::Reject,
        zero_denominator: ZeroDenominator::Reject,
        rounding: AllocationRounding::LargestRemainderMinorUnit,
        conservation: AllocationConservation::ExactPerSource,
        source_refs: vec![],
    }
}

fn relations() -> [Relation; 2] {
    let mut orders = Relation::base(
        "orders",
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("amount_minor", DataType::Int64, false),
            Field::new("expected_count", DataType::Int64, false),
            Field::new("expected_weight", DataType::Int64, false),
        ])),
        "memory",
    );
    orders.semantics = Some(RelationSemantics {
        declared_primary_key: vec!["id".into()],
        allocations: [("category".into(), rule())].into(),
        ..Default::default()
    });
    let bridge = Relation::base(
        "order_category",
        Arc::new(Schema::new(vec![
            Field::new("order_id", DataType::Int64, false),
            Field::new("category", DataType::Utf8, false),
            Field::new("weight", DataType::Int64, false),
        ])),
        "memory",
    );
    [orders, bridge]
}

#[test]
fn authored_allocation_is_published_and_bridge_changes_invalidate_source() {
    let [orders, bridge] = relations();
    let mut catalog = Catalog::from_relations([orders, bridge.clone()]).unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let snapshot = catalog.snapshot();
    let reference = snapshot
        .relation("orders")
        .unwrap()
        .definition_reference("allocation", "category")
        .unwrap();
    assert_eq!(reference.id, "allocation/order-category");
    let mut changed_bridge = bridge;
    changed_bridge.description = Some("new bridge revision".into());
    let report = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(changed_bridge))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert!(report.changed.contains("order_category"));
    assert!(report.affected.contains("orders"));
}

#[test]
fn allocation_contract_requires_a_published_bridge_and_exact_schema() {
    let [orders, mut bridge] = relations();
    let missing = Catalog::from_relations([orders.clone()]).unwrap();
    assert!(missing.validate(&PublicationLimits::default()).is_err());
    bridge.schema = Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("weight", DataType::Utf8, false),
    ]));
    let invalid = Catalog::from_relations([orders, bridge]).unwrap();
    assert!(invalid.validate(&PublicationLimits::default()).is_err());
}
