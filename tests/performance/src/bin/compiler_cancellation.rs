//! Opt-in concurrent cancellation probe with a deterministic gated provider.
//! No network, model service, or result-row execution is involved.

use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::TypedOutcome;
use semantic_engine::Engine;
use semantic_interpreter::{
    Interpreter,
    provider::{Message, ModelProvider, ProviderError},
    typed::InterpretOptions,
};
use serde_json::json;
use tokio::{sync::Semaphore, task::JoinSet};

struct GatedProvider {
    started: Arc<AtomicUsize>,
    gate: Arc<Semaphore>,
    proposal: String,
}

impl ModelProvider for GatedProvider {
    async fn complete(&self, _: &[Message]) -> Result<String, ProviderError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let _permit = self
            .gate
            .acquire()
            .await
            .map_err(|_| ProviderError::Transport)?;
        Ok(self.proposal.clone())
    }
}

fn engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("items", schema.clone(), "benchmark:items"),
            Arc::new(MemTable::try_new(schema, vec![vec![]]).expect("fixed table")),
        )
        .expect("fixed relation");
    engine
}

fn proposal() -> String {
    json!({
        "status": "query",
        "query": {
            "version": 1,
            "input": {"relation": "items", "instance": "i"},
            "requirements": [{
                "id": "id",
                "source_text": "IDs",
                "operation": {"kind": "project", "field": {"instance": "i", "field": "id"}, "alias": "id"}
            }],
            "unresolved": []
        }
    }).to_string()
}

fn argument(name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut values = args.windows(2).filter(|pair| pair[0] == name);
    let value = values.next().map_or(Ok(default), |pair| pair[1].parse())?;
    if values.next().is_some() || !(2..=64).contains(&value) || value % 2 != 0 {
        return Err(format!("{name} must occur at most once and be even in 2..=64").into());
    }
    Ok(value)
}

fn summary(mut samples: Vec<f64>) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    let at = |p: f64| samples[((samples.len() - 1) as f64 * p).ceil() as usize];
    json!({"count": samples.len(), "p50_ms": at(0.5), "p95_ms": at(0.95), "p99_ms": at(0.99), "samples_ms": samples})
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let clients = argument("--clients", 8)?;
    let engine = Arc::new(engine());
    let started = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Semaphore::new(0));
    let provider = GatedProvider {
        started: started.clone(),
        gate: gate.clone(),
        proposal: proposal(),
    };
    let compiler = Arc::new(Interpreter::new(provider));
    let mut tasks = JoinSet::new();
    let mut cancellations = Vec::new();
    for id in 0..clients {
        let mut options = InterpretOptions::default();
        options.timeout = Duration::from_secs(10);
        cancellations.push(options.cancellation.clone());
        let compiler = compiler.clone();
        let engine = engine.clone();
        tasks.spawn(async move {
            let start = Instant::now();
            let result = compiler.compile_typed(&engine, "List IDs", options).await;
            let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
            let kind = match result.outcome {
                TypedOutcome::Compiled { .. } => "compiled",
                TypedOutcome::Unresolved { ref diagnostic } if diagnostic.code == "cancelled" => {
                    "cancelled"
                }
                _ => "unexpected",
            };
            (
                id,
                kind,
                elapsed_ms,
                result.interpretation.work.model_calls,
                result.record.artifact_digest,
            )
        });
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while started.load(Ordering::SeqCst) < clients {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let cancel_start = Instant::now();
    for token in cancellations.iter().take(clients / 2) {
        token.cancel();
    }
    let mut cancelled = Vec::new();
    for _ in 0..clients / 2 {
        let (id, kind, elapsed, calls, digest) =
            tokio::time::timeout(Duration::from_secs(5), tasks.join_next())
                .await?
                .ok_or("missing cancelled task")??;
        if id >= clients / 2 || kind != "cancelled" || calls != 1 || digest.is_some() {
            return Err(format!("cancelled client {id} produced {kind}, calls={calls}").into());
        }
        cancelled.push(elapsed);
    }
    let cancel_completion_ms = cancel_start.elapsed().as_secs_f64() * 1_000.0;
    gate.add_permits(clients / 2);
    let mut completed = Vec::new();
    let mut accepted_digest = None;
    for _ in clients / 2..clients {
        let (id, kind, elapsed, calls, digest) =
            tokio::time::timeout(Duration::from_secs(5), tasks.join_next())
                .await?
                .ok_or("missing released task")??;
        if id < clients / 2 || kind != "compiled" || calls != 1 || digest.is_none() {
            return Err(format!("released client {id} produced {kind}, calls={calls}").into());
        }
        if let Some(expected) = &accepted_digest {
            if digest.as_ref() != Some(expected) {
                return Err("surviving clients produced different artifacts".into());
            }
        } else {
            accepted_digest = digest;
        }
        completed.push(elapsed);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "compiler-cancellation-load-v1",
            "clients": clients,
            "provider_calls_started": started.load(Ordering::SeqCst),
            "cancelled": clients / 2,
            "compiled": clients / 2,
            "cancel_completion_ms": cancel_completion_ms,
            "cancelled_latency": summary(cancelled),
            "survivor_latency": summary(completed),
            "claims": "deterministic concurrent provider-wait cancellation and survivor compilation only; no network/model service or result-row execution"
        }))?
    );
    Ok(())
}
