//! Deterministic opt-in compiler host benchmark; no model or network calls.
//! Example: cargo run --locked --offline -p semantic-performance --release
//!   --bin compiler_scale -- --relations 1000 --fields 100 --repetitions 7

use std::{
    convert::Infallible,
    error::Error,
    sync::Arc,
    time::{Duration, Instant},
};

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::MemTable,
};
use semantic_catalog::{Relation, SearchOptions};
use semantic_compiler::typed::TypedOutcome;
use semantic_engine::{Engine, RelationBackend, TableProvider};
use semantic_interpreter::{
    Interpreter,
    provider::{Message, ModelProvider, ProviderError},
    typed::{InterpretOptions, SelectionMode},
};
use serde_json::{Value, json};

const REQUEST: &str = "measure_alpha measure_beta measure_gamma";
const NEEDLE_FIELDS: [&str; 3] = ["measure_alpha", "measure_beta", "measure_gamma"];

struct EmptyBackend;
impl RelationBackend for EmptyBackend {
    async fn resolve(
        &self,
        relation: &Relation,
    ) -> datafusion::error::Result<Arc<dyn TableProvider>> {
        Ok(Arc::new(MemTable::try_new(
            relation.schema.clone(),
            vec![vec![]],
        )?))
    }
}

struct StaticProposal(String);
impl ModelProvider for StaticProposal {
    async fn complete(&self, _messages: &[Message]) -> Result<String, ProviderError> {
        Ok(self.0.clone())
    }
}

fn argument(name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut values = args.windows(2).filter(|pair| pair[0] == name);
    let value = match values.next() {
        Some(pair) => pair[1].parse::<usize>()?,
        None => default,
    };
    if values.next().is_some() || value == 0 {
        return Err(format!("{name} must be supplied at most once and be positive").into());
    }
    Ok(value)
}

fn bounded_nonnegative_argument(
    name: &str,
    default: usize,
    max: usize,
) -> Result<usize, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut values = args.windows(2).filter(|pair| pair[0] == name);
    let value = values.next().map_or(Ok(default), |pair| pair[1].parse())?;
    if values.next().is_some() || value > max {
        return Err(format!("{name} must occur at most once and be in 0..={max}").into());
    }
    Ok(value)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let index = ((sorted.len() - 1) as f64 * p).ceil() as usize;
    sorted[index]
}

fn summary(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    json!({
        "count": samples.len(),
        "p50_ms": percentile(&samples, 0.50),
        "p95_ms": percentile(&samples, 0.95),
        "p99_ms": percentile(&samples, 0.99),
        "samples_ms": samples,
    })
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn resident_bytes() -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", pid.as_str()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

fn definitions(relations: usize, fields: usize, description_bytes: usize) -> Vec<Relation> {
    (0..relations)
        .map(|relation| {
            let schema = Arc::new(Schema::new(
                (0..fields)
                    .map(|field| {
                        let name = if relation == 0 && field < NEEDLE_FIELDS.len() {
                            NEEDLE_FIELDS[field].to_owned()
                        } else {
                            format!("field_{field}")
                        };
                        Field::new(name, DataType::Int64, false)
                    })
                    .collect::<Vec<_>>(),
            ));
            let mut definition = Relation::base(
                format!("relation_{relation}"),
                schema,
                format!("benchmark:relation_{relation}"),
            );
            let mut description = format!("Synthetic domain {relation} with exact fields");
            if description_bytes > description.len() {
                description.push_str(&"x".repeat(description_bytes - description.len()));
            }
            definition.description = Some(description);
            definition
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let relations = argument("--relations", 100)?;
    let fields = argument("--fields", 16)?;
    let description_bytes = bounded_nonnegative_argument("--description-bytes", 0, 65_536)?;
    if fields < NEEDLE_FIELDS.len() {
        return Err("--fields must be at least three for the narrow-field probe".into());
    }
    let repetitions = argument("--repetitions", 7)?;
    let warmup = argument("--warmup", 2)?;
    let total_fields = relations
        .checked_mul(fields)
        .ok_or("field count overflow")?;
    if total_fields > 1_000_000 {
        return Err("benchmark profile caps total fields at 1,000,000".into());
    }
    if relations
        .checked_mul(description_bytes)
        .ok_or("description byte count overflow")?
        > 16 * 1024 * 1024
    {
        return Err("benchmark profile caps synthetic prose at 16 MiB".into());
    }
    let seed = 0x5345_4d41_4e54_4943_u64;
    let build_start = Instant::now();
    let engine = Engine::from_catalog(
        definitions(relations, fields, description_bytes),
        &EmptyBackend,
    )
    .await?;
    let engine_build_ms = millis(build_start.elapsed());
    let rss_after_engine_bytes = resident_bytes();
    let snapshot = engine.catalog().snapshot();

    let mut index_objects = 0usize;
    let mut index_bytes = 0usize;
    let index_start = Instant::now();
    let index = snapshot.search_index_budgeted(|objects, bytes| {
        index_objects += objects;
        index_bytes += bytes;
        Ok::<_, Infallible>(())
    })?;
    let index_build_ms = millis(index_start.elapsed());
    let rss_after_index_bytes = resident_bytes();
    let search = index.search(REQUEST, &SearchOptions::default(), || {
        Ok::<_, Infallible>(())
    })?;
    if search.hits.len() != NEEDLE_FIELDS.len() {
        return Err("narrow-field search retrieved unrelated fields".into());
    }
    let proposal = json!({
        "status": "query",
        "query": {
            "version": 1,
            "input": {"relation": "relation_0", "instance": "r"},
            "requirements": NEEDLE_FIELDS.iter().map(|field| json!({
                "id": field,
                "source_text": field,
                "operation": {
                    "kind": "project",
                    "field": {"instance": "r", "field": field},
                    "alias": field,
                },
            })).collect::<Vec<_>>(),
            "unresolved": [],
        },
    });
    let compiler = Interpreter::new(StaticProposal(proposal.to_string()));
    let mut options = InterpretOptions::default();
    options.selection_mode = SelectionMode::Retrieved;
    let mut samples = Vec::new();
    let mut work = Vec::new();
    let mut statuses = Vec::new();
    let mut cold_compile_ms = None;
    let mut cold_compile_work = None;
    let mut cold_compile_stages = None;
    for repetition in 0..(warmup + repetitions) {
        let start = Instant::now();
        let compilation = compiler
            .compile_typed(&engine, REQUEST, options.clone())
            .await;
        if compilation.record.outcome != "compiled" {
            return Err(format!(
                "deterministic three-field proposal did not compile: {}",
                compilation.record.outcome
            )
            .into());
        }
        let elapsed_ms = millis(start.elapsed());
        if repetition == 0 {
            cold_compile_ms = Some(elapsed_ms);
            cold_compile_work = Some(serde_json::to_value(&compilation.interpretation.work)?);
            cold_compile_stages = Some(serde_json::to_value(&compilation.interpretation.stages)?);
        }
        if repetition >= warmup {
            let plan_bytes = match &compilation.outcome {
                TypedOutcome::Compiled { query } => query.sql().statement().len(),
                _ => unreachable!("checked compiled outcome"),
            };
            samples.push(elapsed_ms);
            work.push(serde_json::to_value(&compilation.interpretation.work)?);
            statuses.push(json!({
                "outcome": compilation.record.outcome,
                "cache": compilation.interpretation.cache_status,
                "context_bytes": compilation.interpretation.work.context_bytes,
                "context_fields": compilation.interpretation.work.context_fields,
                "extra_context_fields": compilation.interpretation.work.context_fields.saturating_sub(NEEDLE_FIELDS.len()),
                "model_calls": compilation.interpretation.work.model_calls,
                "sql_plan_bytes": plan_bytes,
                "stages": compilation.interpretation.stages,
            }));
        }
    }
    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    let rss_after_compiles_bytes = resident_bytes();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "compiler-host-scale-v1",
            "seed": seed,
            "relations": relations,
            "fields_per_relation": fields,
            "minimum_description_bytes_per_relation": description_bytes,
            "total_fields": total_fields,
            "requested_fields": NEEDLE_FIELDS,
            "repetitions": repetitions,
            "warmup": warmup,
            "toolchain": rustc,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "engine_build_ms": engine_build_ms,
            "rss_after_engine_bytes": rss_after_engine_bytes,
            "index_build_ms": index_build_ms,
            "rss_after_index_bytes": rss_after_index_bytes,
            "index_objects_charged": index_objects,
            "index_bytes_charged": index_bytes,
            "search_postings_visited": search.postings_visited,
            "search_hits": search.hits.len(),
            "cold_compile_ms": cold_compile_ms,
            "cold_compile_work": cold_compile_work,
            "cold_compile_stages": cold_compile_stages,
            "compile_latency": summary(samples),
            "compile_work": work,
            "compile_statuses": statuses,
            "rss_after_compiles_bytes": rss_after_compiles_bytes,
            "claims": "deterministic successful proposal; host compile timings only; no live model or row execution",
        }))?
    );
    Ok(())
}
