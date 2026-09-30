//! Opt-in deterministic observation overhead probe. No model, database read or network.

use std::{error::Error, sync::Arc, time::Instant};

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{
    CompileOptions, CompilerMetrics, ObservationQueue, TypedOutcome, compile_rows,
};
use semantic_engine::Engine;
use semantic_plan::typed::{FieldRef, RelationInput, Requirement, RowOperation, RowQuery};
use serde_json::{Value, json};

fn argument(name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut values = args.windows(2).filter(|pair| pair[0] == name);
    let value = values.next().map_or(Ok(default), |pair| pair[1].parse())?;
    if value == 0 || value > 1_000 || values.next().is_some() {
        return Err(format!("{name} must occur at most once and be between 1 and 1000").into());
    }
    Ok(value)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).ceil() as usize]
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

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let provider = Arc::new(MemTable::try_new(schema.clone(), vec![vec![]]).unwrap());
    let mut engine = Engine::new();
    engine
        .register_table(Relation::base("items", schema, "benchmark:items"), provider)
        .unwrap();
    engine
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "items".into(),
            instance: "r".into(),
        },
        requirements: vec![Requirement {
            id: "value".into(),
            source_text: "value".into(),
            operation: RowOperation::Project {
                field: FieldRef {
                    instance: "r".into(),
                    field: "value".into(),
                },
                alias: "value".into(),
            },
        }],
        unresolved: vec![],
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let repetitions = argument("--repetitions", 100)?;
    let warmup = argument("--warmup", 10)?;
    let engine = fixture();
    let (queue, _receiver) = ObservationQueue::new((repetitions + warmup).min(65_536))
        .map_err(|error| format!("invalid observation queue capacity: {error:?}"))?;
    let metrics = Arc::new(CompilerMetrics::with_observation_queue(queue.clone()));
    let mut disabled = Vec::new();
    let mut normal = Vec::new();
    let mut debug = Vec::new();
    let mut expected_digest = None;

    for round in 0..(repetitions + warmup) {
        for mode in ["disabled", "normal", "debug"] {
            let mut options = CompileOptions::default();
            if mode != "disabled" {
                options.metrics = Some(metrics.clone());
            }
            let start = Instant::now();
            let result = compile_rows(&engine, query(), options).await;
            let TypedOutcome::Compiled { query: artifact } = result.outcome else {
                return Err(format!("{mode} did not compile: {}", result.record.outcome).into());
            };
            if mode == "debug" {
                artifact
                    .capture_stage_replay(1024 * 1024, true)
                    .map_err(|error| format!("stage capture failed: {}", error.code))?;
            }
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            let digest = result
                .record
                .artifact_digest
                .ok_or("missing artifact digest")?;
            if let Some(expected) = &expected_digest {
                if &digest != expected {
                    return Err(format!("{mode} changed semantic artifact digest").into());
                }
            } else {
                expected_digest = Some(digest);
            }
            if round >= warmup {
                match mode {
                    "disabled" => disabled.push(elapsed_ms),
                    "normal" => normal.push(elapsed_ms),
                    "debug" => debug.push(elapsed_ms),
                    _ => unreachable!(),
                }
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": "compiler-observation-v1",
            "repetitions": repetitions,
            "warmup": warmup,
            "disabled": summary(disabled),
            "normal": summary(normal),
            "debug_with_ir_capture": summary(debug),
            "queue_dropped": queue.dropped(),
            "semantic_digest_identical": true,
            "claims": "single-host structured-compile and optional capture overhead only; no model or row execution"
        }))?
    );
    Ok(())
}
