use std::sync::Arc;

use semantic_catalog::{
    Catalog, DataType, Field, FieldSemantics, PublicationError, PublicationLimits, Relation,
    RelationSemantics, Schema, Unit,
};

fn relation(ty: DataType, unit: Unit) -> Relation {
    let mut relation = Relation::base(
        "invoices",
        Arc::new(Schema::new(vec![Field::new("amount", ty, false)])),
        "memory",
    );
    relation.semantics = Some(RelationSemantics {
        fields: [(
            "amount".into(),
            FieldSemantics {
                unit: Some(unit),
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    });
    relation
}

fn code(relation: Relation) -> &'static str {
    let catalog = Catalog::from_relations([relation]).unwrap();
    let Err(PublicationError::InvalidDefinition { code, .. }) =
        catalog.validate(&PublicationLimits::default())
    else {
        panic!("invalid field unit was published")
    };
    code
}

#[test]
fn valid_numeric_field_units_publish_and_change_snapshot_identity() {
    for ty in [
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::Decimal128(18, 2),
    ] {
        let usd =
            Catalog::from_relations([relation(ty.clone(), Unit::Currency { code: "USD".into() })])
                .unwrap();
        usd.validate(&PublicationLimits::default()).unwrap();
        let eur =
            Catalog::from_relations([relation(ty, Unit::Currency { code: "EUR".into() })]).unwrap();
        eur.validate(&PublicationLimits::default()).unwrap();
        assert_ne!(usd.snapshot().id(), eur.snapshot().id());
    }
}

#[test]
fn nonnumeric_physical_fields_and_invalid_units_reject_publication() {
    for ty in [DataType::Utf8, DataType::Float64, DataType::Int8] {
        assert_eq!(
            code(relation(ty, Unit::Currency { code: "USD".into() })),
            "invalid_field_unit"
        );
    }
    assert_eq!(
        code(relation(
            DataType::Int64,
            Unit::Currency { code: "usd".into() },
        )),
        "invalid_field_unit"
    );
}
