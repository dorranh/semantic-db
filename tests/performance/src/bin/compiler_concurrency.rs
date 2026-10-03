//! Opt-in deterministic compiler cache/admission concurrency probe.
//! cargo run --locked --offline -p semantic-performance --bin compiler_concurrency -- --clients 8 --rounds 7

use std::{error::Error, sync::Arc, time::Instant};

use datafusion::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{
    CompilationCacheOptions, CompilationSession, CompileOptions, TypedOutcome,
};
use semantic_engine::Engine;
use semantic_plan::typed::{FieldRef, RelationInput, Requirement, RowOperation, RowQuery};
use serde_json::json;
use tokio::{sync::Barrier, task::JoinSet};

fn argument(name: &str, default: usize, max: usize) -> Result<usize, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut values = args.windows(2).filter(|pair| pair[0] == name);
    let value = values
        .next()
        .map(|pair| pair[1].parse::<usize>())
        .transpose()?
        .unwrap_or(default);
    if values.next().is_some() || value == 0 || value > max {
        return Err(format!("{name} must occur at most once and be in 1..={max}").into());
    }
    Ok(value)
}

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1, 2]))])
        .expect("fixed batch");
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("items", schema.clone(), "benchmark:items"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).expect("fixed table")),
        )
        .expect("fixed relation");
    engine
}

fn query(label: &str) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "items".into(),
            instance: "i".into(),
        },
        requirements: vec![Requirement {
            id: "id".into(),
            source_text: label.into(),
            operation: RowOperation::Project {
                field: FieldRef {
                    instance: "i".into(),
                    field: "id".into(),
                },
                alias: "id".into(),
            },
        }],
        unresolved: vec![],
    }
}

async fn wave(
    session: Arc<CompilationSession<'static>>,
    proposal: RowQuery,
    clients: usize,
) -> Result<Vec<f64>, Box<dyn Error>> {
    let barrier = Arc::new(Barrier::new(clients));
    let mut tasks = JoinSet::new();
    for _ in 0..clients {
        let session = session.clone();
        let proposal = proposal.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            let start = Instant::now();
            let result = session.compile(proposal, CompileOptions::default()).await;
            let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
            let accepted = matches!(result.outcome, TypedOutcome::Compiled { .. });
            (elapsed, accepted, result.record.artifact_digest)
        });
    }
    let mut samples = Vec::with_capacity(clients);
    let mut digest = None;
    while let Some(result) = tasks.join_next().await {
        let (elapsed, accepted, current_digest) = result?;
        if !accepted || current_digest.is_none() {
            return Err("concurrent proposal did not produce an accepted artifact".into());
        }
        if let Some(expected) = &digest {
            if &current_digest != expected {
                return Err("concurrent compilation produced different artifacts".into());
            }
        } else {
            digest = Some(current_digest);
        }
        samples.push(elapsed);
    }
    Ok(samples)
}

fn summary(mut samples: Vec<f64>) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    let at = |p: f64| samples[((samples.len() - 1) as f64 * p).ceil() as usize];
    json!({
        "count": samples.len(),
        "p50_ms": at(0.50),
        "p95_ms": at(0.95),
        "p99_ms": at(0.99),
        "samples_ms": samples,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let clients = argument("--clients", 8, 128)?;
    let rounds = argument("--rounds", 7, 100)?;
    let session = Arc::new(CompilationSession::from_shared(
        Arc::new(fixture()),
        CompilationCacheOptions {
            max_entries: 4,
            max_bytes: 1024 * 1024,
            max_concurrent: clients,
        },
    )?);
    let cold = wave(session.clone(), query("IDs"), clients).await?;
    let cold_stats = session.stats();
    if cold_stats.misses != 1 || cold_stats.hits != (clients - 1) as u64 {
        return Err("same-key cold work was not coalesced".into());
    }
    let mut warm = Vec::with_capacity(clients * rounds);
    for _ in 0..rounds {
        warm.extend(wave(session.clone(), query("IDs"), clients).await?);
    }
    let warm_stats = session.stats();
    if warm_stats.misses != 1 || warm_stats.entries != 1 {
        return Err("warm cache retention was not stable".into());
    }
    // Distinct exact proposals pressure the same bounded LRU. These are still
    // fully checked compilations, with no model or result-row execution.
    for index in 0..8 {
        let result = session
            .compile(
                query(&format!("IDs variant {index}")),
                CompileOptions::default(),
            )
            .await;
        if !matches!(result.outcome, TypedOutcome::Compiled { .. }) {
            return Err("cache-pressure proposal did not compile".into());
        }
    }
    let pressure_stats = session.stats();
    if pressure_stats.entries > 4 || pressure_stats.evictions < 5 {
        return Err("cache pressure exceeded or failed to exercise retention".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "compiler-concurrency-v1",
            "clients": clients,
            "warm_rounds": rounds,
            "cold_latency": summary(cold),
            "cold_cache": cold_stats,
            "warm_latency": summary(warm),
            "warm_cache": warm_stats,
            "pressure_cache": pressure_stats,
            "claims": "deterministic same-key coalescing, bounded cache pressure and host latency; no model or row execution",
        }))?
    );
    Ok(())
}
