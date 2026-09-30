use semantic_catalog::{
    Authority, EntityId, Fact, FactResolution, GrainKey, Presence, SlotMeaning, SourceGrain, Unit,
    UnitQuotientError, checked_unit_quotient,
};

fn known_unit(unit: Presence<Unit>) -> FactResolution<Presence<Unit>> {
    FactResolution::Known {
        value: unit.clone(),
        contributors: vec![Fact {
            id: "unit-fact".into(),
            scope: "orders".into(),
            value: unit,
            authority: Authority::Authored,
            origins: vec![],
            evidence: vec![],
        }],
    }
}

#[test]
fn quotient_preserves_known_and_unknown_unit_meaning() {
    let usd = known_unit(Presence::Value(Unit::Currency { code: "USD".into() }));
    let eur = known_unit(Presence::Value(Unit::Currency { code: "EUR".into() }));
    assert_eq!(
        checked_unit_quotient(&usd, &usd),
        Ok(Presence::Value(Unit::Dimensionless))
    );
    assert_eq!(
        checked_unit_quotient(&usd, &eur),
        Ok(Presence::Value(Unit::Quotient {
            numerator: Box::new(Unit::Currency { code: "USD".into() }),
            denominator: Box::new(Unit::Currency { code: "EUR".into() }),
        }))
    );
    assert_eq!(
        checked_unit_quotient(&usd, &FactResolution::Unknown),
        Ok(Presence::Missing)
    );
    assert_eq!(
        checked_unit_quotient(&known_unit(Presence::Missing), &usd),
        Ok(Presence::Missing)
    );
}

#[test]
fn explicit_null_and_conflict_reject_even_with_an_unknown_other_operand() {
    let null = known_unit(Presence::Null);
    let conflict = FactResolution::Conflicting {
        alternatives: ["USD", "EUR"]
            .into_iter()
            .map(|code| Fact {
                id: code.into(),
                scope: "orders".into(),
                value: Presence::Value(Unit::Currency { code: code.into() }),
                authority: Authority::Authored,
                origins: vec![],
                evidence: vec![],
            })
            .collect(),
    };
    assert_eq!(
        checked_unit_quotient(&FactResolution::Unknown, &null),
        Err(UnitQuotientError::NullOperand)
    );
    assert_eq!(
        checked_unit_quotient(&FactResolution::Unknown, &conflict),
        Err(UnitQuotientError::ConflictingOperand)
    );
}

#[test]
fn slot_grain_keeps_entity_and_relation_scope() {
    let customer = EntityId("customer".into());
    let order = EntityId("order".into());
    let order_grain = SourceGrain {
        entity: Some(order.clone()),
        keys: vec![GrainKey {
            relation: "orders".into(),
            field: "id".into(),
        }],
    };
    let customer_grain = SourceGrain {
        entity: Some(customer),
        keys: vec![GrainKey {
            relation: "customers".into(),
            field: "id".into(),
        }],
    };
    assert_ne!(order_grain, customer_grain);
    assert_ne!(order_grain.entity, Some(EntityId("customer".into())));
    let empty = SlotMeaning::default();
    assert!(matches!(empty.unit, FactResolution::Unknown));
    assert!(matches!(empty.source_grain, FactResolution::Unknown));
    assert!(matches!(empty.entity, FactResolution::Unknown));
}
