//! Bounded in-process aggregate accounting, independent of tracing subscribers
//! and sampling. Exporters read snapshots outside the compilation path.
use super::CompilationRecord;
use serde::Serialize;
use std::{collections::BTreeMap, sync::Mutex};

/// Upper bounds in microseconds. The final bucket includes all larger values.
pub const LATENCY_BUCKETS_US: [u128; 10] = [
    1_000,
    5_000,
    10_000,
    50_000,
    100_000,
    500_000,
    1_000_000,
    5_000_000,
    30_000_000,
    u128::MAX,
];
#[derive(Debug, Default, Clone, Serialize)]
pub struct CompilerMetricsSnapshot {
    pub completed: u128,
    pub outcomes: BTreeMap<&'static str, u128>,
    pub cache_hits: u128,
    pub model_calls: u128,
    pub model_input_bytes: u128,
    pub model_output_bytes: u128,
    pub reported_input_tokens: u128,
    pub reported_output_tokens: u128,
    pub reported_cached_input_tokens: u128,
    pub reported_reasoning_tokens: u128,
    pub calls_missing_input_usage: u128,
    pub calls_missing_output_usage: u128,
    pub context_expansions: u128,
    pub bound_nodes: u128,
    pub index_objects: u128,
    pub index_bytes: u128,
    pub search_postings: u128,
    pub admission_elapsed_micros: u128,
    pub elapsed_micros: u128,
    pub latency_buckets: [u128; 10],
}
#[derive(Debug, Default)]
pub struct CompilerMetrics {
    totals: Mutex<CompilerMetricsSnapshot>,
}
impl CompilerMetrics {
    pub fn snapshot(&self) -> CompilerMetricsSnapshot {
        self.totals
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    pub(super) fn observe(&self, record: &CompilationRecord) {
        let mut total = self
            .totals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        total.completed = total.completed.saturating_add(1);
        let outcome = total.outcomes.entry(record.outcome).or_default();
        *outcome = outcome.saturating_add(1);
        total.cache_hits = total.cache_hits.saturating_add(u128::from(
            record.cache_status == "hit_same_snapshot_and_scope",
        ));
        macro_rules! add {
            ($field:ident, $value:expr) => {
                total.$field = total.$field.saturating_add($value as u128);
            };
        }
        add!(model_calls, record.work.model_calls);
        add!(model_input_bytes, record.work.model_input_bytes);
        add!(model_output_bytes, record.work.model_output_bytes);
        add!(
            reported_input_tokens,
            record.token_accounting.reported_input_tokens
        );
        add!(
            reported_output_tokens,
            record.token_accounting.reported_output_tokens
        );
        add!(
            reported_cached_input_tokens,
            record.token_accounting.reported_cached_input_tokens
        );
        add!(
            reported_reasoning_tokens,
            record.token_accounting.reported_reasoning_tokens
        );
        add!(
            calls_missing_input_usage,
            record.token_accounting.calls_missing_input_usage
        );
        add!(
            calls_missing_output_usage,
            record.token_accounting.calls_missing_output_usage
        );
        add!(context_expansions, record.work.context_expansions);
        add!(bound_nodes, record.work.nodes_visited);
        add!(index_objects, record.work.index_objects_visited);
        add!(index_bytes, record.work.index_bytes_visited);
        add!(search_postings, record.work.search_postings_visited);
        add!(elapsed_micros, record.elapsed_micros);
        for stage in &record.stages {
            if stage.stage == "admission" {
                add!(admission_elapsed_micros, stage.elapsed_micros);
            }
        }
        let bucket = LATENCY_BUCKETS_US
            .iter()
            .position(|bound| record.elapsed_micros <= *bound)
            .expect("unbounded final bucket");
        total.latency_buckets[bucket] = total.latency_buckets[bucket].saturating_add(1);
    }
}
