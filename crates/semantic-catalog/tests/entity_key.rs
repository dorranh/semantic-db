use std::sync::Arc;

use semantic_catalog::{
    Authority, Catalog, DataType, EntityId, EntityIdentity, Fact, FactResolution, Field, GrainKey,
    KeyEvidence, PublicationError, PublicationLimits, Relation, RelationSemantics,
    RelationshipDefinition, RelationshipKey, Schema, SourceGrain, SourceRef,
};

fn authored_identity() -> EntityIdentity {
    let id = EntityId("entities/customer".into());
    let reference = SourceRef {
        artifact_revision: "authoring-v1".into(),
        path: "/entities/customer".into(),
        span: None,
    };
    EntityIdentity {
        id: id.clone(),
        relation: "customers".into(),
        source_grain: SourceGrain {
            entity: Some(id),
            keys: vec![GrainKey {
                relation: "customers".into(),
                field: "id".into(),
            }],
        },
        key_evidence: FactResolution::Known {
            value: KeyEvidence::AuthoredDeclaration,
            contributors: vec![Fact {
                id: "customer-key-declaration".into(),
                scope: "customers".into(),
                value: KeyEvidence::AuthoredDeclaration,
                authority: Authority::Authored,
                origins: vec![reference.clone()],
                evidence: vec![],
            }],
        },
        source_refs: vec![reference],
    }
}

fn relations(identity: EntityIdentity) -> Vec<Relation> {
    let mut customers = Relation::base(
        "customers",
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
        "memory",
    );
    customers.semantics = Some(RelationSemantics {
        declared_primary_key: vec!["id".into()],
        entity_identity: Some(identity),
        ..Default::default()
    });
    let mut orders = Relation::base(
        "orders",
        Arc::new(Schema::new(vec![Field::new(
            "customer_id",
            DataType::Int64,
            false,
        )])),
        "memory",
    );
    orders.semantics = Some(RelationSemantics {
        relationships: [(
            "buyer".into(),
            RelationshipDefinition {
                id: "relationships/order-buyer".into(),
                ai_context: None,
                right_relation: "customers".into(),
                role: "buyer".into(),
                key_pairs: vec![RelationshipKey {
                    left_field: "customer_id".into(),
                    right_field: "id".into(),
                }],
                null_keys_match: false,
                cardinality: FactResolution::Unknown,
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    vec![customers, orders]
}

fn publication_error(relations: Vec<Relation>, expected: &'static str) {
    let catalog = Catalog::from_relations(relations).unwrap();
    let error = catalog.validate(&PublicationLimits::default()).unwrap_err();
    assert!(matches!(
        error,
        PublicationError::InvalidDefinition { code, .. } if code == expected
    ));
}

#[test]
fn authored_entity_key_preserves_provenance_without_proving_join_uniqueness() {
    let catalog = Catalog::from_relations(relations(authored_identity())).unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let customer = catalog.relation("customers").unwrap();
    let identity = customer
        .semantics
        .as_ref()
        .unwrap()
        .entity_identity
        .as_ref()
        .unwrap();
    assert_eq!(identity.source_grain.entity, Some(identity.id.clone()));
    let FactResolution::Known {
        value,
        contributors,
    } = &identity.key_evidence
    else {
        panic!("authored key declaration was not retained")
    };
    assert_eq!(value, &KeyEvidence::AuthoredDeclaration);
    assert_eq!(contributors[0].authority, Authority::Authored);
    assert_eq!(contributors[0].origins[0].path, "/entities/customer");
    let orders = catalog.relation("orders").unwrap();
    assert!(matches!(
        &orders.semantics.as_ref().unwrap().relationships["buyer"].cardinality,
        FactResolution::Unknown
    ));
}

#[test]
fn publication_rejects_cross_relation_or_undeclared_entity_keys() {
    let mut cross_relation = authored_identity();
    cross_relation.source_grain.keys[0].relation = "orders".into();
    publication_error(relations(cross_relation), "invalid_entity_key");

    let mut absent_field = authored_identity();
    absent_field.source_grain.keys[0].field = "other_id".into();
    publication_error(relations(absent_field), "missing_or_ambiguous_field");

    let mut undeclared = relations(authored_identity());
    undeclared[0]
        .semantics
        .as_mut()
        .unwrap()
        .declared_primary_key
        .clear();
    publication_error(undeclared, "invalid_entity_key");
}

#[test]
fn catalog_cannot_authenticate_source_or_runtime_key_claims() {
    for evidence in [
        KeyEvidence::SourceConstraint {
            constraint_id: "customers_pkey".into(),
            source_revision: "warehouse-v1".into(),
        },
        KeyEvidence::RuntimeVerification {
            verification_id: "check-1".into(),
            execution_snapshot: "old-snapshot".into(),
        },
    ] {
        let mut identity = authored_identity();
        identity.key_evidence = FactResolution::Known {
            value: evidence.clone(),
            contributors: vec![Fact {
                id: "untrusted-attestation".into(),
                scope: "customers".into(),
                value: evidence,
                authority: Authority::Verified,
                origins: vec![],
                evidence: vec![],
            }],
        };
        publication_error(relations(identity), "unauthenticated_key_evidence");
    }
}
