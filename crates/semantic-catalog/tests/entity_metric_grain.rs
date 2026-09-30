use std::sync::Arc;

use semantic_catalog::{
    Authority, Catalog, DataType, EmptyBehavior, EntityId, EntityIdentity, Fact, FactResolution,
    Field, GrainKey, KeyEvidence, MetricDefinition, Presence, PublicationError, PublicationLimits,
    Relation, RelationSemantics, Schema, SourceGrain, Unit,
};
use semantic_plan::typed::AggregateFunction;

fn relation(with_identity: bool, metric_entity: Option<&str>, metric_key: &str) -> Relation {
    let id = EntityId("entities/customer".into());
    let key = GrainKey {
        relation: "customers".into(),
        field: "id".into(),
    };
    let identity = EntityIdentity {
        id: id.clone(),
        relation: "customers".into(),
        source_grain: SourceGrain {
            entity: Some(id),
            keys: vec![key],
        },
        key_evidence: FactResolution::Known {
            value: KeyEvidence::AuthoredDeclaration,
            contributors: vec![Fact {
                id: "entities/customer/key_evidence".into(),
                scope: "customers".into(),
                value: KeyEvidence::AuthoredDeclaration,
                authority: Authority::Authored,
                origins: vec![],
                evidence: vec![],
            }],
        },
        source_refs: vec![],
    };
    let metric = MetricDefinition {
        id: "metrics/customer-count".into(),
        description: "Customer count".into(),
        aliases: vec![],
        function: AggregateFunction::Count,
        field: None,
        distinct: false,
        source_grain: SourceGrain {
            entity: metric_entity.map(|id| EntityId(id.into())),
            keys: vec![GrainKey {
                relation: "customers".into(),
                field: metric_key.into(),
            }],
        },
        compatible_dimensions: Default::default(),
        compatible_lookup_dimensions: vec![],
        sum_rollup_dimensions: None,
        state: None,
        row_filters: vec![],
        result_type: DataType::Int64,
        unit: Presence::Value(Unit::Named { id: "rows".into() }),
        temporal: Presence::Missing,
        empty_behavior: EmptyBehavior::Zero,
        source_refs: vec![],
    };
    let mut relation = Relation::base(
        "customers",
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("other_id", DataType::Int64, false),
        ])),
        "memory",
    );
    relation.semantics = Some(RelationSemantics {
        declared_primary_key: vec!["id".into()],
        entity_identity: with_identity.then_some(identity),
        metrics: [("count".into(), metric)].into(),
        ..Default::default()
    });
    relation
}

fn invalid_code(relation: Relation) -> &'static str {
    let catalog = Catalog::from_relations([relation]).unwrap();
    let Err(PublicationError::InvalidDefinition { code, .. }) =
        catalog.validate(&PublicationLimits::default())
    else {
        panic!("expected rejected metric grain")
    };
    code
}

#[test]
fn entity_metric_grain_requires_exact_published_authored_identity() {
    Catalog::from_relations([relation(true, Some("entities/customer"), "id")])
        .unwrap()
        .validate(&PublicationLimits::default())
        .unwrap();
    Catalog::from_relations([relation(false, None, "id")])
        .unwrap()
        .validate(&PublicationLimits::default())
        .unwrap();
    assert_eq!(
        invalid_code(relation(false, Some("entities/customer"), "id")),
        "invalid_metric_entity_grain"
    );
    assert_eq!(
        invalid_code(relation(true, Some("entities/other"), "id")),
        "invalid_metric_entity_grain"
    );
    assert_eq!(
        invalid_code(relation(true, Some("entities/customer"), "other_id")),
        "invalid_metric_entity_grain"
    );
}
