use semantic_catalog::{
    AsOfJoin, Catalog, DataType, FactResolution, Field, PathMissing, PathObligationKind, PathStep,
    PathUsage, RELATIONSHIP_PATH_VERSION, Relation, RelationSemantics, RelationshipDefinition,
    RelationshipKey, RelationshipPath, RelationshipPathError, Schema,
};
use std::sync::Arc;

fn relationship(
    id: &str,
    right: &str,
    role: &str,
    left_field: &str,
    right_field: &str,
) -> RelationshipDefinition {
    RelationshipDefinition {
        ai_context: None,
        id: format!("relationships/{id}"),
        right_relation: right.into(),
        role: role.into(),
        key_pairs: vec![RelationshipKey {
            left_field: left_field.into(),
            right_field: right_field.into(),
        }],
        null_keys_match: false,
        cardinality: FactResolution::Unknown,
        source_refs: vec![],
    }
}
fn relations() -> [Relation; 2] {
    let mut invoices = Relation::base(
        "invoices",
        Arc::new(Schema::new(vec![
            Field::new("billing_id", DataType::Int64, false),
            Field::new("shipping_id", DataType::Int64, false),
            Field::new("as_of", DataType::Date32, false),
        ])),
        "memory",
    );
    invoices.semantics = Some(RelationSemantics {
        relationships: [
            (
                "billing".into(),
                relationship("billing", "accounts", "billing", "billing_id", "id"),
            ),
            (
                "shipping".into(),
                relationship("shipping", "accounts", "shipping", "shipping_id", "id"),
            ),
        ]
        .into(),
        ..Default::default()
    });
    let mut accounts = Relation::base(
        "accounts",
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("manager_id", DataType::Int64, true),
            Field::new("valid_from", DataType::Date32, false),
            Field::new("valid_to", DataType::Date32, false),
        ])),
        "memory",
    );
    accounts.semantics = Some(RelationSemantics {
        relationships: [(
            "manager".into(),
            relationship("manager", "accounts", "manager", "manager_id", "id"),
        )]
        .into(),
        ..Default::default()
    });
    [invoices, accounts]
}
fn path() -> RelationshipPath {
    RelationshipPath {
        version: RELATIONSHIP_PATH_VERSION,
        start_relation: "invoices".into(),
        start_occurrence: "invoice".into(),
        usage: PathUsage::LookupUnique,
        steps: vec![
            PathStep {
                from_occurrence: "invoice".into(),
                to_occurrence: "billing_account".into(),
                right_relation: "accounts".into(),
                relationship: "billing".into(),
                role: "billing".into(),
                as_of: Some(AsOfJoin {
                    fact_time: "as_of".into(),
                    valid_from: "valid_from".into(),
                    valid_to: "valid_to".into(),
                    timezone: None,
                    missing: PathMissing::Null,
                    same_query_uniqueness: true,
                }),
            },
            PathStep {
                from_occurrence: "billing_account".into(),
                to_occurrence: "manager_account".into(),
                right_relation: "accounts".into(),
                relationship: "manager".into(),
                role: "manager".into(),
                as_of: None,
            },
        ],
    }
}

#[test]
fn authored_roles_and_occurrences_allow_bounded_self_paths_with_runtime_obligations() {
    let catalog = Catalog::from_relations(relations()).unwrap();
    let snapshot = catalog.snapshot();
    let validated = path()
        .validate(
            &snapshot,
            Some(&["invoices".into(), "accounts".into()].into()),
            2,
        )
        .unwrap();
    assert_eq!(validated.definitions.len(), 2);
    assert_eq!(validated.required_relations.len(), 2);
    assert_eq!(validated.obligations.len(), 3);
    assert!(
        validated
            .obligations
            .iter()
            .any(|item| item.kind == PathObligationKind::UniqueAsOfInterval)
    );
    assert_eq!(
        path().validate(&snapshot, None, 1),
        Err(RelationshipPathError::Limit)
    );
    assert_eq!(
        path().validate(&snapshot, Some(&["invoices".into()].into()), 2),
        Err(RelationshipPathError::Scope)
    );
}

#[test]
fn billing_shipping_and_self_roles_cannot_collapse() {
    let catalog = Catalog::from_relations(relations()).unwrap();
    let snapshot = catalog.snapshot();
    let mut wrong = path();
    wrong.steps[0].role = "shipping".into();
    assert_eq!(
        wrong.validate(&snapshot, None, 2),
        Err(RelationshipPathError::Role)
    );
    let mut repeated = path();
    repeated.steps[1].to_occurrence = "billing_account".into();
    assert_eq!(
        repeated.validate(&snapshot, None, 2),
        Err(RelationshipPathError::Identity)
    );
    let mut broken = path();
    broken.steps[1].from_occurrence = "invoice".into();
    assert_eq!(
        broken.validate(&snapshot, None, 2),
        Err(RelationshipPathError::Identity)
    );
    let mut version = path();
    version.version += 1;
    assert_eq!(
        version.validate(&snapshot, None, 2),
        Err(RelationshipPathError::Version)
    );
}

#[test]
fn as_of_requires_exact_time_types_and_same_query_uniqueness() {
    let catalog = Catalog::from_relations(relations()).unwrap();
    let snapshot = catalog.snapshot();
    let mut missing_guard = path();
    missing_guard.steps[0]
        .as_of
        .as_mut()
        .unwrap()
        .same_query_uniqueness = false;
    assert_eq!(
        missing_guard.validate(&snapshot, None, 2),
        Err(RelationshipPathError::Uniqueness)
    );
    let mut wrong_time = path();
    wrong_time.steps[0].as_of.as_mut().unwrap().fact_time = "billing_id".into();
    assert_eq!(
        wrong_time.validate(&snapshot, None, 2),
        Err(RelationshipPathError::TimeType)
    );
    let mut wrong_zone = path();
    wrong_zone.steps[0].as_of.as_mut().unwrap().timezone = Some("Europe/Zurich".into());
    assert_eq!(
        wrong_zone.validate(&snapshot, None, 2),
        Err(RelationshipPathError::Timezone)
    );
    let mut invalid_interval = path();
    invalid_interval.steps[0].as_of.as_mut().unwrap().valid_to = "valid_from".into();
    assert_eq!(
        invalid_interval.validate(&snapshot, None, 2),
        Err(RelationshipPathError::Interval)
    );
}

#[test]
fn existence_path_allows_many_to_many_but_key_types_remain_checked() {
    let mut source = relations();
    source[0]
        .semantics
        .as_mut()
        .unwrap()
        .relationships
        .get_mut("billing")
        .unwrap()
        .key_pairs[0]
        .left_field = "as_of".into();
    let catalog = Catalog::from_relations(source).unwrap();
    assert_eq!(
        path().validate(&catalog.snapshot(), None, 2),
        Err(RelationshipPathError::KeyType)
    );
    let catalog = Catalog::from_relations(relations()).unwrap();
    let mut exists = path();
    exists.usage = PathUsage::Existence;
    exists.steps[0].as_of = None;
    let validated = exists.validate(&catalog.snapshot(), None, 2).unwrap();
    assert!(validated.obligations.is_empty());
}
