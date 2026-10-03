//! Bounded in-process aggregate accounting, independent of tracing subscribers
//! and sampling. Exporters read snapshots outside the compilation path.
use super::{CompilationRecord, ObservationQueue};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

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
    /// Public compile calls admitted into in-process accounting, including
    /// calls waiting for a session permit.
    pub admitted: u128,
    pub in_flight: u128,
    pub peak_in_flight: u128,
    pub completed: u128,
    pub cancelled: u128,
    pub deadline_exceeded: u128,
    /// Futures dropped before a terminal compilation record (abort or panic).
    pub abandoned: u128,
    pub outcomes: BTreeMap<&'static str, u128>,
    pub cache_hits: u128,
    pub bound_nodes: u128,
    pub admission_elapsed_micros: u128,
    pub elapsed_micros: u128,
    pub latency_buckets: [u128; 10],
    pub dropped_observations: u64,
}
#[derive(Debug, Default)]
pub struct CompilerMetrics {
    totals: Mutex<CompilerMetricsSnapshot>,
    observations: Option<Arc<ObservationQueue>>,
}

pub(super) struct RequestGuard {
    metrics: Arc<CompilerMetrics>,
    completed: bool,
}

impl RequestGuard {
    pub(super) fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        let mut total = self
            .metrics
            .totals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        total.in_flight = total.in_flight.saturating_sub(1);
        if !self.completed {
            total.abandoned = total.abandoned.saturating_add(1);
        }
    }
}

impl CompilerMetrics {
    pub(super) fn begin(self: &Arc<Self>) -> RequestGuard {
        let mut total = self
            .totals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        total.admitted = total.admitted.saturating_add(1);
        total.in_flight = total.in_flight.saturating_add(1);
        total.peak_in_flight = total.peak_in_flight.max(total.in_flight);
        drop(total);
        RequestGuard {
            metrics: self.clone(),
            completed: false,
        }
    }
    pub fn with_observation_queue(observations: Arc<ObservationQueue>) -> Self {
        Self {
            totals: Mutex::new(CompilerMetricsSnapshot::default()),
            observations: Some(observations),
        }
    }

    pub fn snapshot(&self) -> CompilerMetricsSnapshot {
        let mut snapshot = self
            .totals
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        snapshot.dropped_observations = self
            .observations
            .as_ref()
            .map_or(0, |queue| queue.dropped());
        snapshot
    }
    pub(super) fn observe(&self, record: &CompilationRecord, terminal_code: Option<&str>) {
        let mut total = self
            .totals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        total.completed = total.completed.saturating_add(1);
        match terminal_code {
            Some("cancelled") => total.cancelled = total.cancelled.saturating_add(1),
            Some("deadline") => total.deadline_exceeded = total.deadline_exceeded.saturating_add(1),
            _ => {}
        }
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
        add!(bound_nodes, record.work.nodes_visited);
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
        drop(total);
        if let Some(queue) = &self.observations {
            queue.observe(record);
        }
    }
}
