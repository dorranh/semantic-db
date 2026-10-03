use semantic_catalog::{Catalog, DataType, Field, Relation, Schema};
use std::{collections::HashMap, sync::Arc};

fn relation(name: &str) -> Relation {
    Relation::base(
        name,
        Arc::new(Schema::new(vec![Field::new("ID", DataType::Int64, false)])),
        "private:source",
    )
}

#[test]
fn immutable_snapshots_are_cached_order_independent_and_binding_sensitive() {
    let mut catalog = Catalog::from_relations([relation("a")]).unwrap();
    let old = catalog.snapshot();
    assert!(Arc::ptr_eq(&old, &catalog.snapshot()));
    catalog.register(relation("b")).unwrap();
    let new = catalog.snapshot();
    assert_ne!(old.id(), new.id());
    assert!(old.relation("b").is_none());
    assert_eq!(
        new.id(),
        Catalog::from_relations([relation("b"), relation("a")])
            .unwrap()
            .snapshot()
            .id()
    );
    assert!(new.relation("a").unwrap().field("ID").is_some());
    assert!(new.relation("a").unwrap().field("id").is_none());
    let mut changed = relation("a");
    changed.description = Some("No historical status".into());
    assert_ne!(
        old.id(),
        Catalog::from_relations([changed]).unwrap().snapshot().id()
    );
    let changed = Relation::base("a", relation("a").schema, "different:source");
    assert_ne!(
        old.id(),
        Catalog::from_relations([changed]).unwrap().snapshot().id()
    );
    assert!(catalog.register(relation("a")).is_err());
    assert!(Arc::ptr_eq(&new, &catalog.snapshot()));
}

#[test]
fn metadata_hashing_is_canonical_and_duplicate_fields_are_not_resolved() {
    let snapshot = |pairs: Vec<(&str, &str)>| {
        let mut r = relation("a");
        let metadata: HashMap<_, _> = pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        r.schema = Arc::new(Schema::new_with_metadata(
            r.schema.fields().clone(),
            metadata,
        ));
        Catalog::from_relations([r]).unwrap().snapshot()
    };
    assert_eq!(
        snapshot(vec![("a", "1"), ("b", "2")]).id(),
        snapshot(vec![("b", "2"), ("a", "1")]).id()
    );
    let r = Relation::base(
        "a",
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("id", DataType::Utf8, true),
        ])),
        "source",
    );
    assert!(
        Catalog::from_relations([r])
            .unwrap()
            .snapshot()
            .relation("a")
            .unwrap()
            .field("id")
            .is_none()
    );
}

#[test]
fn source_binding_and_semantic_revisions_are_independent() {
    use semantic_catalog::{RelationSemantics, SourceRef};
    let mut original = relation("a");
    original.semantics = Some(RelationSemantics {
        source_refs: vec![SourceRef {
            artifact_revision: "source1".into(),
            path: "/a".into(),
            span: None,
        }],
        ..Default::default()
    });
    let a = Catalog::from_relations([original.clone()])
        .unwrap()
        .snapshot();
    let mut source_edit = original.clone();
    source_edit.semantics.as_mut().unwrap().source_refs[0].artifact_revision = "source2".into();
    let b = Catalog::from_relations([source_edit]).unwrap().snapshot();
    let mut binding_edit = original;
    binding_edit.kind = semantic_catalog::RelationKind::Base {
        source: "new-physical-source".into(),
    };
    let c = Catalog::from_relations([binding_edit]).unwrap().snapshot();
    assert_ne!(a.id(), b.id());
    assert_ne!(a.id(), c.id());
    assert_eq!(
        a.relation("a").unwrap().semantic_revision(),
        b.relation("a").unwrap().semantic_revision()
    );
    assert_eq!(
        a.relation("a").unwrap().binding_revision(),
        b.relation("a").unwrap().binding_revision()
    );
    assert_eq!(
        a.relation("a").unwrap().semantic_revision(),
        c.relation("a").unwrap().semantic_revision()
    );
    assert_ne!(
        a.relation("a").unwrap().binding_revision(),
        c.relation("a").unwrap().binding_revision()
    );
}
