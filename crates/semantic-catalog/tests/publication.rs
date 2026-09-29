use semantic_catalog::{
    Catalog, CatalogMutation, CatalogStore, DataType, Field, PublicationError, PublicationLimits,
    Relation, RelationKind, Schema,
};
use std::sync::Arc;
fn base(name: &str) -> Relation {
    Relation::base(
        name,
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
        format!("source:{name}"),
    )
}
fn view(name: &str, dependencies: &[&str]) -> Relation {
    let mut relation = Relation::view(name, base(name).schema, "SELECT id FROM a");
    if let RelationKind::View {
        dependencies: stored,
        ..
    } = &mut relation.kind
    {
        *stored = dependencies.iter().map(|s| s.to_string()).collect();
    }
    relation
}
#[test]
fn root_digest_is_canonical_and_updates_share_unchanged_records() {
    let names = (0..300).map(|i| format!("r{i}")).collect::<Vec<_>>();
    let mut catalog = Catalog::from_relations(names.iter().map(|n| base(n))).unwrap();
    let reversed = Catalog::from_relations(names.iter().rev().map(|n| base(n))).unwrap();
    assert_eq!(catalog.snapshot().id(), reversed.snapshot().id());
    let original = catalog.snapshot();
    let changed = base("r150").with_description("changed");
    let report = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(changed))],
            &PublicationLimits::default(),
        )
        .unwrap();
    let current = catalog.snapshot();
    assert_eq!(report.changed, ["r150".into()].into_iter().collect());
    assert!(!std::ptr::eq(
        original.relation("r150").unwrap(),
        current.relation("r150").unwrap()
    ));
    assert!(std::ptr::eq(
        original.relation("r151").unwrap(),
        current.relation("r151").unwrap()
    ));
    assert!(
        original
            .relation("r150")
            .unwrap()
            .definition()
            .description
            .is_none()
    );
    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(base("r150")))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert_eq!(catalog.snapshot().id(), original.id());
    catalog
        .apply_changes(
            [CatalogMutation::Remove("r149".into())],
            &PublicationLimits::default(),
        )
        .unwrap();
    let rebuilt = Catalog::from_relations(
        names
            .iter()
            .filter(|n| n.as_str() != "r149")
            .map(|n| base(n)),
    )
    .unwrap();
    assert_eq!(catalog.snapshot().id(), rebuilt.snapshot().id());
}
#[test]
fn batches_are_atomic_and_report_transitive_dependency_invalidation() {
    let mut catalog = Catalog::from_relations([
        base("a"),
        base("unrelated"),
        view("b", &["a"]),
        view("c", &["a", "b"]),
    ])
    .unwrap();
    let old = catalog.snapshot();
    let report = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(
                base("a").with_description("new"),
            ))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert_eq!(
        report.affected,
        ["a".into(), "b".into(), "c".into()].into_iter().collect()
    );
    let current = catalog.snapshot();
    assert!(matches!(
        catalog.apply_changes(
            [CatalogMutation::Remove("a".into())],
            &PublicationLimits::default()
        ),
        Err(PublicationError::MissingDependency { .. })
    ));
    assert_eq!(catalog.snapshot().id(), current.id());
    assert!(
        catalog
            .apply_changes(
                [CatalogMutation::Put(Box::new(view("a", &["c"])))],
                &PublicationLimits::default()
            )
            .is_err()
    );
    assert_eq!(catalog.snapshot().id(), current.id());
    assert_ne!(old.id(), current.id());
}
#[test]
fn optimistic_publication_preserves_pinned_readers_and_rejects_stale_writers() {
    let store = CatalogStore::new(Catalog::from_relations([base("a")]).unwrap()).unwrap();
    let pinned = store.snapshot();
    store
        .publish(
            pinned.id(),
            [CatalogMutation::Put(Box::new(base("b")))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert!(pinned.relation("b").is_none());
    assert!(store.snapshot().relation("b").is_some());
    assert!(matches!(
        store.publish(
            pinned.id(),
            [CatalogMutation::Put(Box::new(base("c")))],
            &PublicationLimits::default()
        ),
        Err(PublicationError::Conflict)
    ));
    assert!(store.snapshot().relation("c").is_none());
}

#[test]
fn relationship_edges_invalidate_without_becoming_view_expansion_cycles() {
    use semantic_catalog::{
        FactResolution, RelationSemantics, RelationshipDefinition, RelationshipKey,
    };
    let mut a = base("a");
    a.semantics = Some(RelationSemantics {
        relationships: [(
            "self".into(),
            RelationshipDefinition {
                ai_context: None,
                id: "roles/self".into(),
                right_relation: "a".into(),
                role: "related".into(),
                key_pairs: vec![RelationshipKey {
                    left_field: "id".into(),
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
    let mut b = base("b");
    b.semantics = a.semantics.clone();
    let mut catalog = Catalog::from_relations([a.clone(), b]).unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let report = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(
                a.with_description("updated"),
            ))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert_eq!(report.affected, ["a".into(), "b".into()].into());
    let pinned = catalog.snapshot();
    assert!(matches!(
        catalog.apply_changes(
            [CatalogMutation::Remove("a".into())],
            &PublicationLimits::default()
        ),
        Err(PublicationError::MissingDependency { .. })
    ));
    assert_eq!(pinned.id(), catalog.snapshot().id());
}

#[test]
fn warm_publication_revalidates_only_dependents_and_preserves_cycle_and_removal_checks() {
    let mut catalog =
        Catalog::from_relations((0..1000).map(|i| base(&format!("unrelated_{i}"))).chain([
            base("a"),
            view("b", &["a"]),
            view("c", &["a", "b"]),
        ]))
        .unwrap();
    let baseline = catalog.validate(&PublicationLimits::default()).unwrap();
    assert_eq!(baseline.objects_validated, 1003);
    let pinned = catalog.snapshot();
    let limits = PublicationLimits {
        max_objects: 4,
        max_edges: 30,
    };
    let report = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(
                base("a").with_description("new annotation"),
            ))],
            &limits,
        )
        .unwrap();
    assert_eq!(report.objects_validated, 3);
    assert_eq!(report.affected, ["a".into(), "b".into(), "c".into()].into());
    assert!(report.edges_visited < 30);
    assert!(std::ptr::eq(
        pinned.relation("unrelated_0").unwrap(),
        catalog.snapshot().relation("unrelated_0").unwrap()
    ));
    let before = catalog.snapshot().id().to_owned();
    assert!(matches!(
        catalog.apply_changes([CatalogMutation::Put(Box::new(view("a", &["c"])))], &limits),
        Err(PublicationError::Cycle(_))
    ));
    assert!(matches!(
        catalog.apply_changes([CatalogMutation::Remove("a".into())], &limits),
        Err(PublicationError::MissingDependency { .. })
    ));
    assert_eq!(before, catalog.snapshot().id());
    // Removing a whole dependent closure is valid and leaves no stale edges.
    catalog
        .apply_changes(
            [
                CatalogMutation::Remove("c".into()),
                CatalogMutation::Remove("b".into()),
                CatalogMutation::Remove("a".into()),
            ],
            &limits,
        )
        .unwrap();
    let report = catalog
        .apply_changes([CatalogMutation::Put(Box::new(base("a")))], &limits)
        .unwrap();
    assert_eq!(report.affected, ["a".into()].into());
}

#[test]
fn warm_relationship_updates_recheck_endpoint_types_and_account_for_old_edges() {
    use semantic_catalog::{
        FactResolution, RelationSemantics, RelationshipDefinition, RelationshipKey,
    };
    let mut left = base("left");
    left.semantics = Some(RelationSemantics {
        relationships: [(
            "r".into(),
            RelationshipDefinition {
                ai_context: None,
                id: "r".into(),
                right_relation: "right".into(),
                role: "owner".into(),
                key_pairs: vec![RelationshipKey {
                    left_field: "id".into(),
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
    let mut catalog =
        Catalog::from_relations([left.clone(), base("right"), base("other")]).unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let mut changed = base("right");
    changed.schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
    assert!(matches!(
        catalog.apply_changes(
            [CatalogMutation::Put(Box::new(changed))],
            &PublicationLimits::default()
        ),
        Err(PublicationError::InvalidDefinition {
            code: "relationship_key_type",
            ..
        })
    ));
    left.semantics
        .as_mut()
        .unwrap()
        .relationships
        .get_mut("r")
        .unwrap()
        .right_relation = "other".into();
    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(left))],
            &PublicationLimits::default(),
        )
        .unwrap();
    let report = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(
                base("right").with_description("unreferenced"),
            ))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert_eq!(report.affected, ["right".into()].into());
    let report = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(
                base("other").with_description("referenced"),
            ))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert_eq!(report.affected, ["other".into(), "left".into()].into());
}

#[test]
fn value_dictionary_edits_publish_atomically_and_invalidate_search_and_semantics() {
    use semantic_catalog::{RelationSemantics, SearchOptions, ValueMapping};
    let mut relation = base("countries");
    relation.schema = Arc::new(Schema::new(vec![Field::new("code", DataType::Utf8, false)]));
    relation.semantics = Some(RelationSemantics {
        value_mappings: [(
            "country".into(),
            ValueMapping {
                id: "values/country".into(),
                field: "code".into(),
                description: "Country codes".into(),
                codes: [("UK".into(), "GB".into())].into(),
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    let mut catalog = Catalog::from_relations([relation.clone()]).unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let old = catalog.snapshot();
    let old_index = old.search_index(|| Ok::<_, ()>(())).unwrap();
    assert_eq!(
        old_index
            .search("UK", &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap()
            .hits[0]
            .object
            .field
            .as_deref(),
        Some("code")
    );
    let mapping = relation
        .semantics
        .as_mut()
        .unwrap()
        .value_mappings
        .get_mut("country")
        .unwrap();
    mapping.codes.clear();
    mapping.codes.insert("Britain".into(), "GB".into());
    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(relation.clone()))],
            &PublicationLimits::default(),
        )
        .unwrap();
    let current = catalog.snapshot();
    assert_ne!(
        old.relation("countries").unwrap().reference(),
        current.relation("countries").unwrap().reference()
    );
    assert!(
        current
            .search_index(|| Ok::<_, ()>(()))
            .unwrap()
            .search("UK", &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap()
            .hits
            .is_empty()
    );
    assert!(
        !old_index
            .search("UK", &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap()
            .hits
            .is_empty()
    );
    relation.schema = Arc::new(Schema::new(vec![Field::new(
        "code",
        DataType::Int64,
        false,
    )]));
    assert!(
        catalog
            .apply_changes(
                [CatalogMutation::Put(Box::new(relation))],
                &PublicationLimits::default()
            )
            .is_err()
    );
    assert_eq!(catalog.snapshot().id(), current.id());
}
