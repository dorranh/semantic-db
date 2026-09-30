use std::sync::Arc;

use semantic_catalog::{
    Catalog, ConversionError, ConversionRounding, DataType, Field, FieldSemantics,
    PublicationError, PublicationLimits, Relation, RelationSemantics, Schema, SearchOptions,
    UNIT_CONVERSION_VERSION, Unit, UnitConversion,
};

fn rule(numerator: i64) -> UnitConversion {
    UnitConversion {
        version: UNIT_CONVERSION_VERSION,
        id: "conversions/metres-to-feet".into(),
        field: "length_m".into(),
        from_unit: Unit::Named { id: "m".into() },
        to_unit: Unit::Named { id: "ft".into() },
        numerator,
        denominator: 1250,
        rounding: ConversionRounding::HalfEven,
        source_refs: vec![],
    }
}
fn catalog(conversion: UnitConversion) -> Catalog {
    let mut relation = Relation::base(
        "measurements",
        Arc::new(Schema::new(vec![Field::new(
            "length_m",
            DataType::Int64,
            false,
        )])),
        "memory",
    );
    relation.semantics = Some(RelationSemantics {
        conversions: [("metres_to_feet".into(), conversion)].into(),
        ..Default::default()
    });
    Catalog::from_relations([relation]).unwrap()
}

#[test]
fn authored_factor_is_validated_and_pinned() {
    let first = catalog(rule(4101));
    first.validate(&PublicationLimits::default()).unwrap();
    let second = catalog(rule(4102));
    second.validate(&PublicationLimits::default()).unwrap();
    let reference = |catalog: &Catalog| {
        catalog
            .snapshot()
            .relation("measurements")
            .unwrap()
            .definition_reference("conversion", "metres_to_feet")
            .unwrap()
            .clone()
    };
    assert_ne!(reference(&first), reference(&second));
    let index = first.snapshot().search_index(|| Ok::<_, ()>(())).unwrap();
    let report = index
        .search("ft", &SearchOptions::default(), || Ok::<_, ()>(()))
        .unwrap();
    assert!(
        report
            .hits
            .iter()
            .any(|hit| hit.object.relation.as_ref() == "measurements")
    );

    let mut zero = rule(0);
    assert_eq!(zero.validate(), Err(ConversionError::Factor));
    assert!(
        catalog(zero.clone())
            .validate(&PublicationLimits::default())
            .is_err()
    );
    zero.numerator = 1;
    zero.denominator = 0;
    assert_eq!(zero.validate(), Err(ConversionError::Factor));
    let mut future = rule(4101);
    future.version += 1;
    assert_eq!(future.validate(), Err(ConversionError::Version));
    assert!(
        catalog(future)
            .validate(&PublicationLimits::default())
            .is_err()
    );
}

#[test]
fn conversion_field_must_be_exact_int64() {
    let mut relation = catalog(rule(4101))
        .relation("measurements")
        .unwrap()
        .clone();
    relation.schema = Arc::new(Schema::new(vec![Field::new(
        "length_m",
        DataType::Utf8,
        false,
    )]));
    let invalid = Catalog::from_relations([relation]).unwrap();
    assert!(invalid.validate(&PublicationLimits::default()).is_err());
}

#[test]
fn authored_source_field_unit_must_match_typed_conversion_input() {
    let mut relation = catalog(rule(4101))
        .relation("measurements")
        .unwrap()
        .clone();
    let semantics = relation.semantics.as_mut().unwrap();
    semantics.fields.insert(
        "length_m".into(),
        FieldSemantics {
            unit: Some(Unit::Named {
                id: "metres".into(),
            }),
            ..Default::default()
        },
    );
    let catalog = Catalog::from_relations([relation.clone()]).unwrap();
    assert!(matches!(
        catalog.validate(&PublicationLimits::default()),
        Err(PublicationError::InvalidDefinition {
            code: "conversion_source_unit",
            ..
        })
    ));
    relation
        .semantics
        .as_mut()
        .unwrap()
        .fields
        .get_mut("length_m")
        .unwrap()
        .unit = Some(Unit::Named { id: "m".into() });
    Catalog::from_relations([relation])
        .unwrap()
        .validate(&PublicationLimits::default())
        .unwrap();

    let mut invalid = rule(4101);
    invalid.to_unit = Unit::Named { id: " ".into() };
    assert_eq!(invalid.validate(), Err(ConversionError::Identity));
}
