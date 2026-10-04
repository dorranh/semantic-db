//! Opt-in catalog publication and reader-coherence workload without providers.
//! Example: cargo run --locked --offline -p semantic-performance --release
//!   --bin catalog_scale -- --relations 10000 --fields 100

use std::{
    convert::Infallible,
    error::Error,
    sync::{Arc, Barrier},
    time::Instant,
};

use datafusion::arrow::datatypes::{DataType, Field, Schema};
use semantic_catalog::{Catalog, CatalogMutation, PublicationLimits, Relation, SearchOptions};
use serde_json::json;

const REQUEST: &str = "measure_alpha measure_beta measure_gamma";
const NEEDLES: [&str; 3] = ["measure_alpha", "measure_beta", "measure_gamma"];

fn argument(name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut values = args.windows(2).filter(|pair| pair[0] == name);
    let value = values
        .next()
        .map(|pair| pair[1].parse::<usize>())
        .transpose()?
        .unwrap_or(default);
    if values.next().is_some() || value == 0 {
        return Err(format!("{name} must occur at most once and be positive").into());
    }
    Ok(value)
}

fn definitions(relations: usize, fields: usize) -> Vec<Relation> {
    (0..relations)
        .map(|relation| {
            let schema = Arc::new(Schema::new(
                (0..fields)
                    .map(|field| {
                        let name = if relation == 0 && field < NEEDLES.len() {
                            NEEDLES[field].to_owned()
                        } else {
                            format!("field_{field}")
                        };
                        Field::new(name, DataType::Int64, false)
                    })
                    .collect::<Vec<_>>(),
            ));
            Relation::base(
                format!("relation_{relation}"),
                schema,
                format!("benchmark:relation_{relation}"),
            )
        })
        .collect()
}

fn millis(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn main() -> Result<(), Box<dyn Error>> {
    let relations = argument("--relations", 100)?;
    let fields = argument("--fields", 100)?;
    let readers = argument("--readers", 8)?;
    let reads_per_reader = argument("--reads-per-reader", 100)?;
    if fields < NEEDLES.len() || readers > 64 || reads_per_reader > 10_000 {
        return Err("invalid width or bounded reader profile".into());
    }
    let total_fields = relations
        .checked_mul(fields)
        .ok_or("field count overflow")?;
    if total_fields > 1_000_000 {
        return Err("catalog scale profile caps total fields at 1,000,000".into());
    }
    let seed = 0x4341_5441_4c4f_4753_u64;
    let build_start = Instant::now();
    let mut catalog = Catalog::from_relations(definitions(relations, fields))?;
    catalog.validate(&PublicationLimits::default())?;
    let publish_ms = millis(build_start);
    let before = catalog.snapshot();
    let index_start = Instant::now();
    let mut charged_objects = 0usize;
    let mut charged_bytes = 0usize;
    let index = before.search_index_budgeted(|objects, bytes| {
        charged_objects += objects;
        charged_bytes += bytes;
        Ok::<_, Infallible>(())
    })?;
    let index_ms = millis(index_start);
    let search = index.search(REQUEST, &SearchOptions::default(), || {
        Ok::<_, Infallible>(())
    })?;
    if search.hits.len() != NEEDLES.len() {
        return Err("narrow-field lookup found unrelated fields".into());
    }

    let barrier = Arc::new(Barrier::new(readers + 1));
    let mut tasks = Vec::with_capacity(readers);
    for _ in 0..readers {
        let pinned = before.clone();
        let barrier = barrier.clone();
        tasks.push(std::thread::spawn(move || {
            let id = pinned.id().to_owned();
            barrier.wait();
            for _ in 0..reads_per_reader {
                if pinned.id() != id
                    || pinned.relation("relation_0").is_none()
                    || pinned
                        .relation("relation_0")
                        .unwrap()
                        .field(NEEDLES[0])
                        .is_none()
                {
                    return false;
                }
            }
            true
        }));
    }
    let mut updated = catalog
        .relation("relation_0")
        .ok_or("missing first relation")?
        .clone();
    updated.description = Some("single changed description".into());
    barrier.wait();
    let update_start = Instant::now();
    let report = catalog.apply_changes(
        [CatalogMutation::Put(Box::new(updated))],
        &PublicationLimits::default(),
    )?;
    let update_ms = millis(update_start);
    let coherent_readers = tasks.into_iter().all(|task| task.join().unwrap_or(false));
    if !coherent_readers {
        return Err("a pinned catalog reader observed mixed generations".into());
    }
    let after = catalog.snapshot();
    if before.id() == after.id() || !report.changed.contains("relation_0") {
        return Err("single-relation publication failed to create a generation".into());
    }
    let reindex_start = Instant::now();
    let after_index = after.search_index(|| Ok::<_, Infallible>(()))?;
    let reindex_ms = millis(reindex_start);
    let after_search = after_index.search(REQUEST, &SearchOptions::default(), || {
        Ok::<_, Infallible>(())
    })?;
    if after_search.hits.len() != NEEDLES.len() {
        return Err("updated index lost narrow-field results".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "catalog-publication-scale-v1",
            "seed": seed,
            "relations": relations,
            "fields_per_relation": fields,
            "total_fields": total_fields,
            "reader_tasks": readers,
            "reads_per_reader": reads_per_reader,
            "coherent_readers": coherent_readers,
            "publish_ms": publish_ms,
            "index_ms": index_ms,
            "index_objects_charged": charged_objects,
            "index_bytes_charged": charged_bytes,
            "search_postings_visited": search.postings_visited,
            "search_hits": search.hits.len(),
            "single_change_ms": update_ms,
            "affected_relations": report.affected.len(),
            "updated_index_ms": reindex_ms,
            "claims": "deterministic catalog host workload; no model or row execution",
        }))?
    );
    Ok(())
}
