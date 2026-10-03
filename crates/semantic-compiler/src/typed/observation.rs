//! Optional local compilation observations. The payload has no free-form text,
//! identifiers, values, paths, provenance or digests. Channel pressure drops
//! observations without delaying or changing the compilation result.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use tokio::sync::mpsc::{self, Receiver, Sender};

use super::CompilationRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationOutcome {
    Compiled,
    NeedsClarification,
    Unsupported,
    Unresolved,
    Rejected,
    ProviderFailure,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub version: u32,
    pub outcome: ObservationOutcome,
    pub artifact_cache_hit: bool,
    pub context_cache_hit: bool,
    pub elapsed_micros: u64,
}

impl Observation {
    fn from_record(record: &CompilationRecord) -> Self {
        let outcome = match record.outcome {
            "compiled" => ObservationOutcome::Compiled,
            "needs_clarification" => ObservationOutcome::NeedsClarification,
            "unsupported" => ObservationOutcome::Unsupported,
            "unresolved" => ObservationOutcome::Unresolved,
            "rejected" => ObservationOutcome::Rejected,
            "provider_failure" => ObservationOutcome::ProviderFailure,
            _ => ObservationOutcome::Unknown,
        };
        Self {
            version: 1,
            outcome,
            artifact_cache_hit: record.cache_status == "hit_same_snapshot_and_scope",
            context_cache_hit: record.cache_status == "context_hit_same_snapshot",
            elapsed_micros: record.elapsed_micros.min(u128::from(u64::MAX)) as u64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationQueueError {
    InvalidCapacity,
}

pub struct ObservationQueue {
    sender: Sender<Observation>,
    dropped: AtomicU64,
    capacity: usize,
}

impl std::fmt::Debug for ObservationQueue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObservationQueue")
            .field("capacity", &self.capacity)
            .field("dropped", &self.dropped())
            .finish()
    }
}

impl ObservationQueue {
    /// The receiver is application-owned and may be consumed on another task.
    /// A nonempty finite bound prevents unobserved output from accumulating.
    pub fn new(
        capacity: usize,
    ) -> Result<(Arc<Self>, Receiver<Observation>), ObservationQueueError> {
        if capacity == 0 || capacity > 65_536 {
            return Err(ObservationQueueError::InvalidCapacity);
        }
        let (sender, receiver) = mpsc::channel(capacity);
        Ok((
            Arc::new(Self {
                sender,
                dropped: AtomicU64::new(0),
                capacity,
            }),
            receiver,
        ))
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub(super) fn observe(&self, record: &CompilationRecord) {
        if self
            .sender
            .try_send(Observation::from_record(record))
            .is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
