use semantic_catalog::{
    AiContext, Catalog, DataType, Field, FieldSemantics, Relation, RelationSemantics, Schema,
    SearchOptions,
};
use std::sync::Arc;
fn indexed_relation(name: &str) -> Relation {
    let mut relation = Relation::base(
        name,
        Arc::new(Schema::new(vec![
            Field::new("account_id", DataType::Int64, false),
            Field::new("amount", DataType::Int64, true),
        ])),
        "private",
    );
    let mut semantics = RelationSemantics {
        ai_context: Some(AiContext {
            synonyms: vec!["revenue".into()],
            ..Default::default()
        }),
        ..Default::default()
    };
    semantics.fields.insert(
        "account_id".into(),
        FieldSemantics {
            ai_context: Some(AiContext {
                synonyms: vec!["customer".into()],
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    relation.semantics = Some(semantics);
    relation
}
#[test]
fn exact_aliases_preserve_alternatives_and_global_fields_discover_parents() {
    let catalog =
        Catalog::from_relations([indexed_relation("orders"), indexed_relation("invoices")])
            .unwrap();
    let snapshot = catalog.snapshot();
    let index = snapshot.search_index(|| Ok::<_, ()>(())).unwrap();
    let report = index
        .search("revenue", &SearchOptions::default(), || Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(report.hits.len(), 2);
    assert!(
        report
            .hits
            .iter()
            .all(|h| h.exact && h.object.field.is_none())
    );
    let report = index
        .search("customer", &SearchOptions::default(), || Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(report.hits.len(), 2);
    assert!(
        report
            .hits
            .iter()
            .all(|h| h.object.field.as_deref() == Some("account_id"))
    );
    assert!(Arc::ptr_eq(
        &index,
        &snapshot
            .search_index(|| Err::<(), _>("should not rebuild"))
            .unwrap()
    ));
}
#[test]
fn cutoffs_and_negative_lookups_are_explicit_and_scope_filters_apply() {
    let mut catalog = Catalog::from_relations([indexed_relation("orders")]).unwrap();
    let snapshot = catalog.snapshot();
    let index = snapshot.search_index(|| Ok::<_, ()>(())).unwrap();
    let report = index
        .search("missing", &SearchOptions::default(), || Ok::<_, ()>(()))
        .unwrap();
    assert!(report.exhausted && report.hits.is_empty());
    assert!(!report.lookup_fingerprints.is_empty());
    catalog.register(indexed_relation("missing")).unwrap();
    let new = catalog.snapshot().search_index(|| Ok::<_, ()>(())).unwrap();
    assert_ne!(index.snapshot_id(), new.snapshot_id());
    assert!(
        !new.search("missing", &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap()
            .hits
            .is_empty()
    );
    let options = SearchOptions {
        max_postings: 0,
        ..Default::default()
    };
    assert!(
        !index
            .search("orders", &options, || Ok::<_, ()>(()))
            .unwrap()
            .exhausted
    );
    let options = SearchOptions {
        max_candidates: 0,
        ..Default::default()
    };
    assert!(
        index
            .search("orders", &options, || Ok::<_, ()>(()))
            .unwrap()
            .truncated_candidates
    );
    let options = SearchOptions {
        allowed_relations: Some(Default::default()),
        ..Default::default()
    };
    assert!(
        index
            .search("orders", &options, || Ok::<_, ()>(()))
            .unwrap()
            .hits
            .is_empty()
    );
}
#[test]
fn cancelled_index_build_is_not_published() {
    let snapshot = Catalog::from_relations([indexed_relation("orders")])
        .unwrap()
        .snapshot();
    assert!(snapshot.search_index(|| Err::<(), _>("cancelled")).is_err());
    assert_eq!(
        snapshot
            .search_index(|| Ok::<_, ()>(()))
            .unwrap()
            .fields_indexed,
        2
    );
}

#[test]
fn oversized_metadata_is_rejected_before_tokenization_and_failed_index_is_not_retained() {
    let mut relation = indexed_relation("orders");
    relation.description = Some("a".repeat(128 * 1024));
    let snapshot = Catalog::from_relations([relation]).unwrap().snapshot();
    let mut charged = 0usize;
    let result = snapshot.search_index_budgeted(|_, bytes| {
        charged = charged.saturating_add(bytes);
        if charged > 1024 { Err("bytes") } else { Ok(()) }
    });
    assert_eq!(result.unwrap_err(), "bytes");
    let mut objects = 0;
    let index = snapshot
        .search_index_budgeted(|n, _| {
            objects += n;
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(objects, 3);
    assert!(Arc::ptr_eq(
        &index,
        &snapshot
            .search_index_budgeted(|_, _| Err::<(), _>("warm"))
            .unwrap()
    ));
}

#[test]
fn publication_updates_postings_without_retokenizing_unrelated_text() {
    use semantic_catalog::{CatalogMutation, PublicationLimits};
    let mut catalog = Catalog::from_relations(
        (0..40)
            .map(|i| {
                let mut relation = indexed_relation(&format!("other_{i}"));
                relation.description = Some(format!("word{i}{}", "x".repeat(8 * 1024)));
                relation
            })
            .chain([indexed_relation("orders")]),
    )
    .unwrap();
    let old_snapshot = catalog.snapshot();
    let old = old_snapshot.search_index(|| Ok::<_, ()>(())).unwrap();
    assert!(
        old.search("bookings", &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap()
            .hits
            .is_empty()
    );
    let mut changed = indexed_relation("orders");
    changed
        .semantics
        .as_mut()
        .unwrap()
        .ai_context
        .as_mut()
        .unwrap()
        .synonyms = vec!["bookings".into()];
    catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(changed))],
            &PublicationLimits::default(),
        )
        .unwrap();
    let mut bytes = 0;
    let index = catalog
        .snapshot()
        .search_index_budgeted(|_, amount| {
            bytes += amount;
            if bytes > 32 * 1024 {
                Err("reindexed unrelated descriptions")
            } else {
                Ok(())
            }
        })
        .unwrap();
    assert_eq!(index.objects_indexed(), old.objects_indexed());
    let fresh = Catalog::from_relations(catalog.relations().cloned())
        .unwrap()
        .snapshot()
        .search_index(|| Ok::<_, ()>(()))
        .unwrap();
    for term in [
        "bookings", "orders", "customer", "revenue", "amount", "missing",
    ] {
        let updated = index
            .search(term, &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap();
        let cold = fresh
            .search(term, &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap();
        assert_eq!(
            serde_json::to_value(updated).unwrap(),
            serde_json::to_value(cold).unwrap(),
            "{term}"
        );
    }
    assert!(
        old.search("bookings", &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap()
            .hits
            .is_empty()
    );
    catalog
        .apply_changes(
            [CatalogMutation::Remove("orders".into())],
            &PublicationLimits::default(),
        )
        .unwrap();
    let removed = catalog.snapshot().search_index(|| Ok::<_, ()>(())).unwrap();
    assert!(
        removed
            .search("bookings", &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap()
            .hits
            .is_empty()
    );
    assert_eq!(removed.fields_indexed + 2, index.fields_indexed);
}
