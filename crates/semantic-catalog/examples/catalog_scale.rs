//! Offline host-cost benchmark. No providers, model calls, sockets, or row reads.
//! cargo run --release -p semantic-catalog --example catalog_scale --offline -- 1000 100
use semantic_catalog::{
    AiContext, Catalog, CatalogMutation, DataType, Field, FieldSemantics, PublicationLimits,
    Relation, RelationSemantics, Schema, SearchOptions,
};
use std::{sync::Arc, time::Instant};
fn relation(index: usize, width: usize) -> Relation {
    let fields = (0..width)
        .map(|field| Field::new(format!("field_{field:05}"), DataType::Int64, true))
        .collect::<Vec<_>>();
    let mut relation = Relation::base(
        format!("relation_{index:05}"),
        Arc::new(Schema::new(fields)),
        "benchmark",
    );
    relation.description = Some(format!("Synthetic domain {index}"));
    if index == 0 {
        let mut semantics = RelationSemantics::default();
        semantics.fields.insert(
            "field_00000".into(),
            FieldSemantics {
                ai_context: Some(AiContext {
                    synonyms: vec!["rare needle".into()],
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        relation.semantics = Some(semantics);
    }
    relation
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let count: usize = args
        .first()
        .map_or("1000", String::as_str)
        .parse()
        .expect("relation count");
    let width: usize = args
        .get(1)
        .map_or("100", String::as_str)
        .parse()
        .expect("width");
    assert!(count > 0 && width > 0);
    let start = Instant::now();
    let mut catalog =
        Catalog::from_relations((0..count).map(|index| relation(index, width))).unwrap();
    let registration_us = start.elapsed().as_micros();
    let start = Instant::now();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let initial_validation_us = start.elapsed().as_micros();
    let snapshot = catalog.snapshot();
    let mut objects = 0;
    let mut bytes = 0;
    let start = Instant::now();
    let index = snapshot
        .search_index_budgeted(|n, b| {
            objects += n;
            bytes += b;
            Ok::<_, ()>(())
        })
        .unwrap();
    let index_us = start.elapsed().as_micros();
    let start = Instant::now();
    let report = index
        .search("rare needle", &SearchOptions::default(), || Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(report.hits[0].object.relation.as_ref(), "relation_00000");
    let search_us = start.elapsed().as_micros();
    let mut changed = relation(0, width);
    changed
        .semantics
        .as_mut()
        .unwrap()
        .fields
        .get_mut("field_00000")
        .unwrap()
        .ai_context
        .as_mut()
        .unwrap()
        .synonyms
        .push("new needle".into());
    let start = Instant::now();
    let publication = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(changed))],
            &PublicationLimits::default(),
        )
        .unwrap();
    let publication_us = start.elapsed().as_micros();
    let mut updated_objects = 0;
    let mut updated_bytes = 0;
    let start = Instant::now();
    let updated = catalog
        .snapshot()
        .search_index_budgeted(|n, b| {
            updated_objects += n;
            updated_bytes += b;
            Ok::<_, ()>(())
        })
        .unwrap();
    let update_index_us = start.elapsed().as_micros();
    assert!(
        !updated
            .search("new needle", &SearchOptions::default(), || Ok::<_, ()>(()))
            .unwrap()
            .hits
            .is_empty()
    );
    println!(
        "{}",
        serde_json::json!({"relations":count,"fields":count*width,"registration_us":registration_us,"initial_validation_us":initial_validation_us,"cold_index_us":index_us,"search_us":search_us,"publication_us":publication_us,"index_update_us":update_index_us,"index_objects":objects,"index_source_bytes":bytes,"update_objects":updated_objects,"update_bytes":updated_bytes,"publication_objects":publication.objects_validated,"publication_edges":publication.edges_visited,"search_postings":report.postings_visited})
    );
}
