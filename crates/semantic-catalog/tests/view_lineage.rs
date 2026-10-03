use std::{collections::BTreeMap, sync::Arc};

use semantic_catalog::{
    Catalog, DataType, Field, ObjectRef, PublicationError, PublicationLimits, Relation,
    RelationKind, RelationSemantics, Schema, ViewOutputLineage,
};

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("state", DataType::Utf8, true),
    ]))
}

fn base() -> Relation {
    Relation::base("tickets", schema(), "fixture:tickets")
}

fn source_reference(source: &Relation) -> ObjectRef {
    Catalog::from_relations([source.clone()])
        .unwrap()
        .snapshot()
        .relation(&source.name)
        .unwrap()
        .reference()
        .clone()
}

fn direct_view(source: ObjectRef) -> Relation {
    let mut view = Relation::view(
        "projected_tickets",
        schema(),
        "SELECT id AS id, state AS state FROM tickets",
    );
    let RelationKind::View { dependencies, .. } = &mut view.kind else {
        unreachable!()
    };
    *dependencies = vec!["tickets".into()];
    view.semantics = Some(RelationSemantics {
        view_lineage: Some(ViewOutputLineage {
            source,
            columns: BTreeMap::from([("id".into(), "id".into()), ("state".into(), "state".into())]),
            source_refs: vec![],
        }),
        ..Default::default()
    });
    view
}

fn assert_invalid(base: Relation, view: Relation) {
    assert!(matches!(
        Catalog::from_relations([base, view])
            .unwrap()
            .validate(&PublicationLimits::default()),
        Err(PublicationError::InvalidDefinition {
            code: "invalid_view_lineage",
            ..
        })
    ));
}

#[test]
fn direct_view_lineage_pins_exact_source_and_definition_revision() {
    let source = base();
    let view = direct_view(source_reference(&source));
    let catalog = Catalog::from_relations([source.clone(), view.clone()]).unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let reference = catalog
        .snapshot()
        .relation("projected_tickets")
        .unwrap()
        .definition_reference("view_lineage", "projected_tickets")
        .unwrap()
        .clone();
    assert_eq!(reference.id, "view_lineage/projected_tickets");

    let mut changed = source.clone();
    changed.description = Some("new authored source description".into());
    assert_invalid(changed, view.clone());

    let mut altered = view.clone();
    altered
        .semantics
        .as_mut()
        .unwrap()
        .view_lineage
        .as_mut()
        .unwrap()
        .columns
        .insert("state".into(), "id".into());
    assert_invalid(source.clone(), altered);

    let mut altered = view.clone();
    altered
        .semantics
        .as_mut()
        .unwrap()
        .view_lineage
        .as_mut()
        .unwrap()
        .columns
        .remove("state");
    assert_invalid(source.clone(), altered);

    let mut altered = view.clone();
    let RelationKind::View { sql, .. } = &mut altered.kind else {
        unreachable!()
    };
    *sql = "SELECT id + 1 AS id, state AS state FROM tickets".into();
    assert_invalid(source.clone(), altered);

    let mut altered = view.clone();
    let RelationKind::View { dependencies, .. } = &mut altered.kind else {
        unreachable!()
    };
    dependencies.clear();
    // The authored source must also be a declared view dependency.
    assert_invalid(source, altered);
}
