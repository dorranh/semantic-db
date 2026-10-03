use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use semantic_catalog::{
    Catalog, CatalogMutation, ConceptDefinition, ConversionRounding, DataType, Field,
    PublicationLimits, Relation, RelationSemantics, Schema, UNIT_CONVERSION_VERSION,
    UnitConversion,
};
use semantic_compiler::typed::{
    AnalysisCache, AnalysisCacheError, AnalysisCacheIdentity, AnalysisCacheLimits, CandidateKind,
    DependencySet, LookupDependency,
};
use semantic_plan::typed::{Comparison, Literal, RowPredicate};

fn concept(name: &str, alias: &str, value: &str) -> ConceptDefinition {
    ConceptDefinition {
        id: format!("concepts/{name}"),
        description: name.into(),
        aliases: vec![alias.into()],
        alternatives: vec![],
        predicate: RowPredicate::Compare {
            field: "state".into(),
            operator: Comparison::Eq,
            value: Literal::Utf8(value.into()),
        },
        source_refs: vec![],
    }
}

fn relation(name: &str, concepts: Vec<(&str, &str, &str)>) -> Relation {
    let mut relation = Relation::base(
        name,
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("state", DataType::Utf8, false),
        ])),
        format!("source:{name}"),
    );
    relation.semantics = Some(RelationSemantics {
        concepts: concepts
            .into_iter()
            .map(|(name, alias, value)| (name.into(), concept(name, alias, value)))
            .collect(),
        ..Default::default()
    });
    relation
}

fn fixture() -> Catalog {
    Catalog::from_relations([
        relation("items", vec![("active", "live", "open")]),
        relation("unrelated", vec![]),
    ])
    .unwrap()
}

fn candidate(phrase: &str) -> LookupDependency {
    LookupDependency::Candidates {
        relation: "items".into(),
        kind: CandidateKind::Concept,
        phrase: phrase.into(),
    }
}

fn identity(input: &str) -> AnalysisCacheIdentity {
    AnalysisCacheIdentity {
        stage: "rendered_context".into(),
        input_digest: input.into(),
        access_scope_revision: "tenant-a".into(),
        parameter_digest: "no-parameters".into(),
        renderer_revision: "render-v1".into(),
        function_revision: "functions-v1".into(),
        acceptance_revision: "strict-v1".into(),
    }
}

#[test]
fn authored_conversion_candidate_tracks_negative_and_changed_lookup() {
    let mut catalog = fixture();
    let before = catalog.snapshot();
    let missing = DependencySet::capture(
        &before,
        [LookupDependency::Candidates {
            relation: "items".into(),
            kind: CandidateKind::Conversion,
            phrase: "to_kilograms".into(),
        }],
        1,
    )
    .unwrap();
    let mut changed = relation("items", vec![("active", "live", "open")]);
    changed.semantics.as_mut().unwrap().conversions.insert(
        "to_kilograms".into(),
        UnitConversion {
            version: UNIT_CONVERSION_VERSION,
            id: "items/conversions/to-kilograms".into(),
            field: "id".into(),
            from_unit: semantic_catalog::Unit::Named { id: "grams".into() },
            to_unit: semantic_catalog::Unit::Named {
                id: "kilograms".into(),
            },
            numerator: 1,
            denominator: 1000,
            rounding: ConversionRounding::Truncate,
            source_refs: vec![],
        },
    );
    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(changed))],
            &PublicationLimits::default(),
        )
        .unwrap();
    let after = catalog.snapshot();
    assert!(!missing.matches(&after));
    let present = DependencySet::capture(
        &after,
        [LookupDependency::Candidates {
            relation: "items".into(),
            kind: CandidateKind::Conversion,
            phrase: "items/conversions/to-kilograms".into(),
        }],
        1,
    )
    .unwrap();
    assert!(present.matches(&after));
}

#[test]
fn positive_negative_and_competing_lookups_track_publication_results() {
    let mut catalog = fixture();
    let first = catalog.snapshot();
    let positive = DependencySet::capture(&first, [candidate("live")], 8).unwrap();
    let negative = DependencySet::capture(&first, [candidate("pending")], 8).unwrap();
    let absent_relation = DependencySet::capture(
        &first,
        [LookupDependency::Relation {
            name: "later".into(),
        }],
        8,
    )
    .unwrap();
    let absent_definition = DependencySet::capture(
        &first,
        [LookupDependency::Definition {
            relation: "items".into(),
            kind: "concept".into(),
            name: "pending".into(),
        }],
        8,
    )
    .unwrap();

    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(relation(
                "unrelated",
                vec![("other", "else", "closed")],
            )))],
            &PublicationLimits::default(),
        )
        .unwrap();
    let unrelated = catalog.snapshot();
    assert_ne!(first.id(), unrelated.id());
    for dependency in [&positive, &negative, &absent_relation, &absent_definition] {
        assert!(dependency.matches(&unrelated));
    }

    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(relation(
                "items",
                vec![
                    ("active", "live", "open"),
                    ("second", "live", "closed"),
                    ("pending", "queued", "pending"),
                ],
            )))],
            &PublicationLimits::default(),
        )
        .unwrap();
    let changed = catalog.snapshot();
    assert!(
        !positive.matches(&changed),
        "new alias competitor must invalidate"
    );
    assert!(
        !negative.matches(&changed),
        "formerly missing name must invalidate"
    );
    assert!(!absent_definition.matches(&changed));
    assert!(absent_relation.matches(&changed));

    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(relation("later", vec![])))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert!(!absent_relation.matches(&catalog.snapshot()));
}

#[test]
fn field_and_policy_lookup_are_scoped_and_bounded() {
    let mut catalog = fixture();
    let first = catalog.snapshot();
    let lookups = [
        LookupDependency::Field {
            relation: "items".into(),
            name: "state".into(),
        },
        LookupDependency::PolicySet {
            relation: "items".into(),
        },
    ];
    let dependencies = DependencySet::capture(&first, lookups, 2).unwrap();
    assert_eq!(dependencies.lookups().count(), 2);
    assert!(DependencySet::capture(&first, [candidate("live"), candidate("missing")], 1).is_err());
    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(relation(
                "unrelated",
                vec![("other", "else", "closed")],
            )))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert!(dependencies.matches(&catalog.snapshot()));

    let mut changed = relation("items", vec![("active", "live", "open")]);
    changed.schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("state", DataType::Utf8, true),
    ]));
    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(changed))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert!(!dependencies.matches(&catalog.snapshot()));
}

#[tokio::test]
async fn concurrent_build_is_single_flight_and_stale_result_rebuilds() {
    let catalog = fixture();
    let snapshot = catalog.snapshot();
    let cache = Arc::new(
        AnalysisCache::<u32>::new(AnalysisCacheLimits {
            max_entries: 2,
            max_bytes: 64,
            max_lookups: 8,
            max_active_keys: 2,
        })
        .unwrap(),
    );
    let builds = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let snapshot = snapshot.clone();
        let cache = cache.clone();
        let builds = builds.clone();
        tasks.push(tokio::spawn(async move {
            cache
                .get_or_build(&snapshot, identity("same"), move || async move {
                    builds.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Ok::<_, ()>((7, vec![candidate("live")], 4))
                })
                .await
                .unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(*task.await.unwrap(), 7);
    }
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert_eq!(cache.stats().hits, 11);

    let mut changed = fixture();
    changed
        .apply_changes(
            [CatalogMutation::Put(Box::new(relation(
                "items",
                vec![("active", "live", "open"), ("second", "live", "closed")],
            )))],
            &PublicationLimits::default(),
        )
        .unwrap();
    let second = changed.snapshot();
    let builds_again = builds.clone();
    let value = cache
        .get_or_build(&second, identity("same"), move || async move {
            builds_again.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>((9, vec![candidate("live")], 4))
        })
        .await
        .unwrap();
    assert_eq!(*value, 9);
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    assert_eq!(cache.stats().evictions, 1);
}

#[tokio::test]
async fn distinct_build_admission_is_bounded_without_blocking_same_key_or_retained_hits() {
    let snapshot = fixture().snapshot();
    let cache = Arc::new(
        AnalysisCache::<u32>::new(AnalysisCacheLimits {
            max_entries: 3,
            max_bytes: 64,
            max_lookups: 8,
            max_active_keys: 2,
        })
        .unwrap(),
    );
    let permits = Arc::new(tokio::sync::Semaphore::new(0));
    let builds = Arc::new(AtomicUsize::new(0));
    let (started, mut entered) = tokio::sync::mpsc::unbounded_channel();
    let mut tasks = Vec::new();
    for (key, value) in [("first", 1), ("second", 2)] {
        let cache = cache.clone();
        let snapshot = snapshot.clone();
        let permits = permits.clone();
        let builds = builds.clone();
        let started = started.clone();
        tasks.push(tokio::spawn(async move {
            cache
                .get_or_build(&snapshot, identity(key), move || async move {
                    builds.fetch_add(1, Ordering::SeqCst);
                    started.send(()).unwrap();
                    let permit = permits.acquire().await.unwrap();
                    permit.forget();
                    Ok::<_, ()>((value, vec![candidate("live")], 4))
                })
                .await
                .unwrap()
        }));
    }
    entered.recv().await.unwrap();
    entered.recv().await.unwrap();
    assert_eq!(cache.stats().active_keys, 2);

    let rejected = cache
        .get_or_build(&snapshot, identity("third"), || async {
            panic!("new-key builder must not start at admission limit");
            #[allow(unreachable_code)]
            Ok::<_, ()>((3, vec![candidate("live")], 4))
        })
        .await;
    assert!(matches!(rejected, Err(AnalysisCacheError::Admission)));
    assert_eq!(cache.stats().admission_rejections, 1);

    let same_builds = builds.clone();
    let same_key = cache.get_or_build(&snapshot, identity("first"), move || async move {
        same_builds.fetch_add(1, Ordering::SeqCst);
        Ok::<_, ()>((99, vec![candidate("live")], 4))
    });
    tokio::pin!(same_key);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut same_key)
            .await
            .is_err()
    );
    permits.add_permits(2);
    assert_eq!(*same_key.await.unwrap(), 1);
    for task in tasks {
        assert!(*task.await.unwrap() <= 2);
    }
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    assert_eq!(cache.stats().active_keys, 0);

    let retained = cache
        .get_or_build(&snapshot, identity("first"), || async {
            panic!("retained hit must not rebuild");
            #[allow(unreachable_code)]
            Ok::<_, ()>((99, vec![candidate("live")], 4))
        })
        .await
        .unwrap();
    assert_eq!(*retained, 1);
    let newly_admitted = cache
        .get_or_build(&snapshot, identity("third"), || async {
            Ok::<_, ()>((3, vec![candidate("live")], 4))
        })
        .await
        .unwrap();
    assert_eq!(*newly_admitted, 3);
    assert!(cache.stats().active_keys <= 2);
}
