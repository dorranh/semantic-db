use semantic_catalog::{
    ALLOCATION_VERSION, AllocationBridgeObservation, AllocationConservation, AllocationContract,
    AllocationDenominator, AllocationEligibility, AllocationError, AllocationRounding,
    AllocationSourceObservation, Catalog, DataType, Field, NullWeight, Relation, RelationSemantics,
    Schema, ZeroDenominator,
};
use std::sync::Arc;

fn contract() -> AllocationContract {
    AllocationContract {
        version: ALLOCATION_VERSION,
        id: "allocations/order-category".into(),
        source_relation: "orders".into(),
        bridge_relation: "order_category".into(),
        source_entity_fields: vec!["id".into()],
        bridge_source_fields: vec!["order_id".into()],
        target_dimensions: vec!["category".into()],
        source_amount_field: "amount_minor".into(),
        amount_unit: "minor_currency_unit".into(),
        weight_field: "weight".into(),
        eligible_population: AllocationEligibility::Utf8Equals {
            field: "status".into(),
            value: "eligible".into(),
        },
        expected_membership_count_field: "eligible_count".into(),
        expected_weight_total_field: "eligible_weight".into(),
        denominator: AllocationDenominator::SumEligibleWeightsPerSource,
        null_weight: NullWeight::Reject,
        zero_denominator: ZeroDenominator::Reject,
        rounding: AllocationRounding::LargestRemainderMinorUnit,
        conservation: AllocationConservation::ExactPerSource,
        source_refs: vec![],
    }
}

fn snapshot() -> Arc<semantic_catalog::CatalogSnapshot> {
    let mut orders = Relation::base(
        "orders",
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("amount_minor", DataType::Int64, false),
            Field::new("eligible_count", DataType::Int64, false),
            Field::new("eligible_weight", DataType::Int64, false),
        ])),
        "memory",
    );
    orders.semantics = Some(RelationSemantics {
        declared_primary_key: vec!["id".into()],
        ..Default::default()
    });
    let bridge = Relation::base(
        "order_category",
        Arc::new(Schema::new(vec![
            Field::new("order_id", DataType::Int64, false),
            Field::new("category", DataType::Utf8, false),
            Field::new("weight", DataType::Int64, true),
            Field::new("status", DataType::Utf8, false),
        ])),
        "memory",
    );
    Catalog::from_relations([orders, bridge])
        .unwrap()
        .snapshot()
}

fn row(category: &str, weight: Option<i128>, eligible: bool) -> AllocationBridgeObservation {
    AllocationBridgeObservation {
        target: vec![category.into()],
        weight,
        eligibility_value: Some(if eligible { "eligible" } else { "excluded" }.into()),
    }
}
fn source(amount: i128) -> AllocationSourceObservation {
    AllocationSourceObservation {
        amount,
        expected_membership_count: 2,
        expected_weight_total: 3,
    }
}

#[test]
fn authored_schema_and_scope_are_checked_and_revision_changes_with_rules() {
    let snapshot = snapshot();
    let rule = contract();
    rule.validate(
        &snapshot,
        Some(&["orders".into(), "order_category".into()].into()),
    )
    .unwrap();
    assert_eq!(
        rule.validate(&snapshot, Some(&["orders".into()].into())),
        Err(AllocationError::Scope)
    );
    let mut changed = rule.clone();
    changed.eligible_population = AllocationEligibility::AllRows;
    assert_ne!(rule.reference(), changed.reference());
    changed = rule.clone();
    changed.source_entity_fields = vec!["amount_minor".into()];
    assert_eq!(
        changed.validate(&snapshot, None),
        Err(AllocationError::Identity)
    );
    changed = rule.clone();
    changed.bridge_source_fields = vec!["category".into()];
    assert_eq!(
        changed.validate(&snapshot, None),
        Err(AllocationError::Type)
    );
    changed = rule.clone();
    changed.weight_field = "status".into();
    assert_eq!(
        changed.validate(&snapshot, None),
        Err(AllocationError::Type)
    );
    changed.version += 1;
    assert_eq!(
        changed.validate(&snapshot, None),
        Err(AllocationError::Version)
    );
}

#[test]
fn unequal_weights_allocate_minor_units_once_and_conserve_positive_and_negative() {
    let rule = contract();
    let rows = [row("A", Some(1), true), row("B", Some(2), true)];
    let result = rule.verify_and_allocate(&source(101), &rows).unwrap();
    assert_eq!(result[0].target, vec!["A"]);
    assert_eq!(result[0].amount, 34);
    assert_eq!(result[1].target, vec!["B"]);
    assert_eq!(result[1].amount, 67);
    let negative = rule.verify_and_allocate(&source(-101), &rows).unwrap();
    assert_eq!(
        negative.iter().map(|item| item.amount).collect::<Vec<_>>(),
        vec![-34, -67]
    );
    let reversed = [rows[1].clone(), rows[0].clone()];
    assert_eq!(
        result,
        rule.verify_and_allocate(&source(101), &reversed).unwrap()
    );
    let with_ineligible = [rows[0].clone(), row("C", Some(99), false), rows[1].clone()];
    assert_eq!(
        result,
        rule.verify_and_allocate(&source(101), &with_ineligible)
            .unwrap()
    );
}

#[test]
fn missing_duplicate_null_zero_and_policy_filtered_population_fail_closed() {
    let rule = contract();
    let complete = [row("A", Some(1), true), row("B", Some(2), true)];
    assert_eq!(
        rule.verify_and_allocate(&source(101), &complete[..1]),
        Err(AllocationError::IncompletePopulation)
    );
    assert_eq!(
        rule.verify_and_allocate(&source(101), &[complete[0].clone(), complete[0].clone()]),
        Err(AllocationError::Target)
    );
    assert_eq!(
        rule.verify_and_allocate(&source(101), &[row("A", None, true), complete[1].clone()]),
        Err(AllocationError::Weight)
    );
    assert_eq!(
        rule.verify_and_allocate(
            &source(101),
            &[row("A", Some(-1), true), complete[1].clone()]
        ),
        Err(AllocationError::Weight)
    );
    let filtered = [complete[0].clone(), row("B", Some(2), false)];
    assert_eq!(
        rule.verify_and_allocate(&source(101), &filtered),
        Err(AllocationError::IncompletePopulation)
    );
    let zero = AllocationSourceObservation {
        expected_weight_total: 0,
        ..source(101)
    };
    assert_eq!(
        rule.verify_and_allocate(&zero, &[row("A", Some(0), true), row("B", Some(0), true)]),
        Err(AllocationError::ZeroDenominator)
    );
}

#[test]
fn overflow_and_incomplete_denominator_cannot_return_an_allocation() {
    let rule = contract();
    let rows = [row("A", Some(1), true), row("B", Some(2), true)];
    let mismatched = AllocationSourceObservation {
        expected_weight_total: 4,
        ..source(101)
    };
    assert_eq!(
        rule.verify_and_allocate(&mismatched, &rows),
        Err(AllocationError::IncompletePopulation)
    );
    assert_eq!(
        rule.verify_and_allocate(&source(i128::MAX), &rows)
            .unwrap()
            .iter()
            .map(|share| share.amount)
            .sum::<i128>(),
        i128::MAX
    );
    let larger_weight = AllocationSourceObservation {
        expected_weight_total: 4,
        ..source(i128::MAX)
    };
    assert_eq!(
        rule.verify_and_allocate(
            &larger_weight,
            &[row("A", Some(1), true), row("B", Some(3), true)]
        ),
        Err(AllocationError::Overflow)
    );
}
