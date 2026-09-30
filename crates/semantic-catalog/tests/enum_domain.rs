use std::sync::Arc;

use semantic_catalog::{
    Catalog, DataType, EnumDomain, Field, FieldSemantics, PublicationError, PublicationLimits,
    Relation, RelationSemantics, Schema, ValueMapping,
};

fn relation(field_domain: &str, mapping_domain: Option<&str>) -> Relation {
    let mut relation = Relation::base(
        "events",
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("order_status", DataType::Utf8, false),
            Field::new("risk_status", DataType::Utf8, false),
        ])),
        "memory",
    );
    relation.semantics = Some(RelationSemantics {
        fields: [
            (
                "order_status".into(),
                FieldSemantics {
                    enum_domain: Some(EnumDomain {
                        id: field_domain.into(),
                    }),
                    ..Default::default()
                },
            ),
            (
                "risk_status".into(),
                FieldSemantics {
                    enum_domain: Some(EnumDomain {
                        id: "risk-status".into(),
                    }),
                    ..Default::default()
                },
            ),
        ]
        .into(),
        value_mappings: [(
            "active_orders".into(),
            ValueMapping {
                id: "values/active-orders".into(),
                field: "order_status".into(),
                description: "Exact order status".into(),
                codes: [("active".into(), "A".into())].into(),
                enum_domain: mapping_domain.map(|id| EnumDomain { id: id.into() }),
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    relation
}

#[test]
fn same_utf8_code_does_not_override_authored_enum_identity() {
    for (field_domain, mapping_domain, expected) in [
        (
            "order-status",
            Some("risk-status"),
            "invalid_value_mapping_domain",
        ),
        ("order-status", None, "invalid_value_mapping_domain"),
        ("", Some(""), "invalid_enum_domain"),
    ] {
        let invalid = Catalog::from_relations([relation(field_domain, mapping_domain)]).unwrap();
        assert!(matches!(
            invalid.validate(&PublicationLimits::default()),
            Err(PublicationError::InvalidDefinition { code, .. }) if code == expected
        ));
    }

    let valid = Catalog::from_relations([relation("order-status", Some("order-status"))]).unwrap();
    valid.validate(&PublicationLimits::default()).unwrap();
    let changed =
        Catalog::from_relations([relation("order-status-v2", Some("order-status-v2"))]).unwrap();
    changed.validate(&PublicationLimits::default()).unwrap();
    assert_ne!(
        valid.snapshot().relation("events").unwrap().reference(),
        changed.snapshot().relation("events").unwrap().reference()
    );
}
