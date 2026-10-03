//! Opt-in publication work for dependency chains, diamonds and dense hubs.
//! No source providers, model calls or network are involved.

use std::{convert::Infallible, error::Error, sync::Arc, time::Instant};

use datafusion::arrow::datatypes::{DataType, Field, Schema};
use semantic_catalog::{
    Catalog, CatalogMutation, PublicationLimits, Relation, RelationKind, SearchOptions,
};
use serde_json::json;

fn argument(name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut values = args.windows(2).filter(|pair| pair[0] == name);
    let value = values.next().map_or(Ok(default), |pair| pair[1].parse())?;
    if value == 0 || value > 64 || values.next().is_some() {
        return Err(format!("{name} must occur at most once and be between 1 and 64").into());
    }
    Ok(value)
}

fn shape() -> Result<String, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut values = args.windows(2).filter(|pair| pair[0] == "--shape");
    let value = values.next().map_or("chain", |pair| pair[1].as_str());
    if values.next().is_some() || !matches!(value, "chain" | "diamond" | "hub") {
        return Err("--shape must be chain, diamond or hub".into());
    }
    Ok(value.into())
}

fn view(name: String, dependencies: &[String], schema: &Arc<Schema>) -> Relation {
    let sql = dependencies
        .iter()
        .map(|dependency| format!("SELECT id FROM {dependency}"))
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    let mut relation = Relation::view(name, schema.clone(), sql);
    if let RelationKind::View {
        dependencies: stored,
        ..
    } = &mut relation.kind
    {
        *stored = dependencies.to_vec();
    }
    relation
}

fn definitions(shape: &str, size: usize) -> Vec<Relation> {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let mut relations = vec![Relation::base("root", schema.clone(), "benchmark:root")];
    match shape {
        "chain" => {
            let mut prior = "root".to_owned();
            for step in 0..size {
                let name = format!("chain_{step}");
                relations.push(view(name.clone(), &[prior], &schema));
                prior = name;
            }
        }
        "diamond" => {
            let mut prior = "root".to_owned();
            for step in 0..size {
                let left = format!("left_{step}");
                let right = format!("right_{step}");
                let merge = format!("merge_{step}");
                relations.push(view(left.clone(), &[prior.clone()], &schema));
                relations.push(view(right.clone(), &[prior], &schema));
                relations.push(view(merge.clone(), &[left, right], &schema));
                prior = merge;
            }
        }
        "hub" => {
            for step in 0..size {
                relations.push(view(format!("spoke_{step}"), &["root".into()], &schema));
            }
        }
        _ => unreachable!("validated shape"),
    }
    relations.push(Relation::base("unrelated", schema, "benchmark:unrelated"));
    relations
}

fn millis(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn main() -> Result<(), Box<dyn Error>> {
    let shape = shape()?;
    let size = argument("--size", 8)?;
    let build = Instant::now();
    let mut catalog = Catalog::from_relations(definitions(&shape, size))?;
    catalog.validate(&PublicationLimits::default())?;
    let build_ms = millis(build);
    let before = catalog.snapshot();
    let total_relations = before.relations().count();
    let cold_index_start = Instant::now();
    let mut cold_index_objects = 0usize;
    before.search_index_budgeted(|objects, _bytes| {
        cold_index_objects += objects;
        Ok::<_, Infallible>(())
    })?;
    let cold_index_ms = millis(cold_index_start);
    let root = catalog.relation("root").ok_or("missing root")?.clone();
    let mut changed = root;
    changed.description = Some("single root revision".into());
    let publication = Instant::now();
    let report = catalog.apply_changes(
        [CatalogMutation::Put(Box::new(changed))],
        &PublicationLimits::default(),
    )?;
    let publication_ms = millis(publication);
    let after = catalog.snapshot();
    if before.id() == after.id()
        || report.affected.len() != total_relations - 1
        || report.affected.contains("unrelated")
    {
        return Err("dependency publication reached the wrong relation set".into());
    }
    // Pinned readers remain on the old generation while the new generation is
    // indexed. This also checks a changed shared root never becomes 'absent'.
    if before.relation("root").unwrap().reference() == after.relation("root").unwrap().reference() {
        return Err("root revision did not change".into());
    }
    let index_start = Instant::now();
    let mut charged_objects = 0usize;
    let index = after.search_index_budgeted(|objects, _bytes| {
        charged_objects += objects;
        Ok::<_, Infallible>(())
    })?;
    let index_ms = millis(index_start);
    let search = index.search(
        "root",
        &SearchOptions::default(),
        || Ok::<_, Infallible>(()),
    )?;
    if search.hits.is_empty() {
        return Err("changed root was absent from the index".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "catalog-dependency-scale-v1",
            "shape": shape,
            "size": size,
            "relations": total_relations,
            "affected_relations": report.affected.len(),
            "unrelated_preserved": !report.affected.contains("unrelated"),
            "build_ms": build_ms,
            "cold_index_ms": cold_index_ms,
            "cold_index_objects_charged": cold_index_objects,
            "single_change_ms": publication_ms,
            "updated_index_ms": index_ms,
            "index_objects_charged": charged_objects,
            "root_search_hits": search.hits.len(),
            "claims": "deterministic dependency publication and index update only; no model or row execution",
        }))?
    );
    Ok(())
}
