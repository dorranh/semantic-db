use std::sync::Arc;

use semantic_catalog::{
    Catalog, DataType, FactResolution, Field, FieldSemantics, PathStep, PathUsage,
    PublicationError, PublicationLimits, RELATIONSHIP_PATH_VERSION, ReferenceSystem, Relation,
    RelationSemantics, RelationshipDefinition, RelationshipKey, RelationshipPath,
    RelationshipPathError, Schema,
};

fn relation_pair(left_id: Option<&str>, right_id: Option<&str>) -> [Relation; 2] {
    let mut orders = Relation::base(
        "orders",
        Arc::new(Schema::new(vec![Field::new("code", DataType::Utf8, false)])),
        "memory",
    );
    orders.semantics = Some(RelationSemantics {
        fields: left_id
            .map(|id| {
                [(
                    "code".into(),
                    FieldSemantics {
                        reference_system: Some(ReferenceSystem { id: id.into() }),
                        ..Default::default()
                    },
                )]
                .into()
            })
            .unwrap_or_default(),
        relationships: [(
            "region".into(),
            RelationshipDefinition {
                ai_context: None,
                id: "relationships/order-region".into(),
                right_relation: "regions".into(),
                role: "region".into(),
                key_pairs: vec![RelationshipKey {
                    left_field: "code".into(),
                    right_field: "code".into(),
                }],
                null_keys_match: false,
                cardinality: FactResolution::Unknown,
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    let mut regions = Relation::base(
        "regions",
        Arc::new(Schema::new(vec![Field::new("code", DataType::Utf8, false)])),
        "memory",
    );
    regions.semantics = Some(RelationSemantics {
        fields: right_id
            .map(|id| {
                [(
                    "code".into(),
                    FieldSemantics {
                        reference_system: Some(ReferenceSystem { id: id.into() }),
                        ..Default::default()
                    },
                )]
                .into()
            })
            .unwrap_or_default(),
        ..Default::default()
    });
    [orders, regions]
}

#[test]
fn path_validation_checks_reference_systems_even_for_unpublished_catalogs() {
    let path = RelationshipPath {
        version: RELATIONSHIP_PATH_VERSION,
        start_relation: "orders".into(),
        start_occurrence: "o".into(),
        usage: PathUsage::LookupUnique,
        steps: vec![PathStep {
            from_occurrence: "o".into(),
            to_occurrence: "r".into(),
            right_relation: "regions".into(),
            relationship: "region".into(),
            role: "region".into(),
            as_of: None,
        }],
    };
    let mismatched = Catalog::from_relations(relation_pair(
        Some("customer-codes"),
        Some("supplier-codes"),
    ))
    .unwrap();
    assert!(matches!(
        path.validate(&mismatched.snapshot(), None, 1),
        Err(RelationshipPathError::KeyReferenceSystem)
    ));
    let matching = Catalog::from_relations(relation_pair(
        Some("customer-codes"),
        Some("customer-codes"),
    ))
    .unwrap();
    path.validate(&matching.snapshot(), None, 1).unwrap();
}

#[test]
fn same_physical_key_type_does_not_override_authored_reference_systems() {
    for (left, right, code) in [
        (
            Some("customer-codes"),
            Some("supplier-codes"),
            "relationship_reference_system",
        ),
        (
            Some("customer-codes"),
            None,
            "relationship_reference_system",
        ),
        (Some(" "), Some(" "), "invalid_reference_system"),
    ] {
        let catalog = Catalog::from_relations(relation_pair(left, right)).unwrap();
        let error = catalog.validate(&PublicationLimits::default()).unwrap_err();
        assert!(matches!(
            error,
            PublicationError::InvalidDefinition { code: actual, .. } if actual == code
        ));
    }

    let catalog = Catalog::from_relations(relation_pair(
        Some("customer-codes"),
        Some("customer-codes"),
    ))
    .unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    Catalog::from_relations(relation_pair(None, None))
        .unwrap()
        .validate(&PublicationLimits::default())
        .unwrap();
}
