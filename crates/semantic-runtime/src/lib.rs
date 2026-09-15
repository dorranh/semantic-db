//! Connector-independent execution state. Never place query state on a shared connection.
pub mod ipc;
pub mod staging;

use datafusion::{
    error::{DataFusionError, Result},
    execution::TaskContext,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::Notify, time::Instant};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub fn unique_id() -> String {
    format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

/// Hash length-prefixed components, avoiding ambiguous concatenations.
pub fn fingerprint(parts: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    format!("{:x}", hash.finalize())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueryOptions {
    pub timeout_seconds: u64,
    pub max_remote_bytes: usize,
    pub max_remote_requests: usize,
    pub max_decoded_bytes: usize,
    pub bypass_materialization: bool,
    pub max_cache_age_ms: Option<u64>,
    pub refresh_materializations: Vec<String>,
}
impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            timeout_seconds: 30,
            max_remote_bytes: 256 * 1024 * 1024,
            max_remote_requests: 256,
            max_decoded_bytes: 1024 * 1024 * 1024,
            bypass_materialization: false,
            max_cache_age_ms: None,
            refresh_materializations: vec![],
        }
    }
}
impl QueryOptions {
    pub fn validate(&self) -> Result<()> {
        if self.timeout_seconds == 0
            || self.timeout_seconds > 86400
            || self.max_remote_requests == 0
            || self.max_remote_bytes == 0
            || self.max_decoded_bytes == 0
        {
            return Err(failure("invalid query budgets"));
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct QueryMetrics {
    pub remote_bytes: usize,
    pub decoded_bytes: usize,
    pub remote_requests: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheObservation {
    pub relation: String,
    pub generation: String,
    pub published_at_ms: u64,
    pub age_ms: u64,
}

#[derive(Debug)]
pub struct QueryContext {
    pub id: String,
    pub options: QueryOptions,
    deadline: Instant,
    caches: std::sync::Mutex<Vec<CacheObservation>>,
    cancelled: AtomicBool,
    notification: Notify,
    remote_bytes: AtomicUsize,
    decoded_bytes: AtomicUsize,
    requests: AtomicUsize,
    hits: AtomicUsize,
    misses: AtomicUsize,
}
impl QueryContext {
    pub fn new(options: QueryOptions) -> Result<Arc<Self>> {
        options.validate()?;
        Ok(Arc::new(Self {
            id: unique_id(),
            caches: Default::default(),
            deadline: Instant::now() + Duration::from_secs(options.timeout_seconds),
            options,
            cancelled: AtomicBool::new(false),
            notification: Notify::new(),
            remote_bytes: AtomicUsize::new(0),
            decoded_bytes: AtomicUsize::new(0),
            requests: AtomicUsize::new(0),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }))
    }
    pub fn from_task(task: &TaskContext) -> Option<Arc<Self>> {
        task.session_config().get_extension::<Self>()
    }
    pub fn record_cache(&self, observation: CacheObservation) {
        self.caches.lock().unwrap().push(observation);
    }
    pub fn cache_observations(&self) -> Vec<CacheObservation> {
        self.caches.lock().unwrap().clone()
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notification.notify_waiters();
    }
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(failure("query cancelled"));
        }
        if Instant::now() >= self.deadline {
            return Err(failure("query timed out"));
        }
        Ok(())
    }
    pub async fn run<T>(&self, operation: impl Future<Output = Result<T>>) -> Result<T> {
        let notified = self.notification.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        self.check()?;
        tokio::select! {
            biased;
            _ = &mut notified => Err(failure("query cancelled")),
            result = tokio::time::timeout_at(self.deadline, operation) => result.map_err(|_| failure("query timed out"))?,
        }
    }
    pub fn charge_remote(&self, bytes: usize) -> Result<()> {
        self.check()?;
        charge(
            &self.remote_bytes,
            bytes,
            self.options.max_remote_bytes,
            "query remote byte budget exhausted",
        )
    }
    pub fn charge_decoded(&self, bytes: usize) -> Result<()> {
        self.check()?;
        charge(
            &self.decoded_bytes,
            bytes,
            self.options.max_decoded_bytes,
            "query decoded byte budget exhausted",
        )
    }
    pub fn request_started(&self) -> Result<()> {
        self.check()?;
        charge(
            &self.requests,
            1,
            self.options.max_remote_requests,
            "query request budget exhausted",
        )
    }
    pub fn cache_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }
    pub fn cache_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }
    pub fn metrics(&self) -> QueryMetrics {
        QueryMetrics {
            remote_bytes: self.remote_bytes.load(Ordering::Relaxed),
            decoded_bytes: self.decoded_bytes.load(Ordering::Relaxed),
            remote_requests: self.requests.load(Ordering::Relaxed),
            cache_hits: self.hits.load(Ordering::Relaxed),
            cache_misses: self.misses.load(Ordering::Relaxed),
        }
    }
}
fn charge(counter: &AtomicUsize, bytes: usize, limit: usize, message: &str) -> Result<()> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            n.checked_add(bytes).filter(|v| *v <= limit)
        })
        .map(|_| ())
        .map_err(|_| failure(message))
}
pub fn failure(message: &str) -> DataFusionError {
    DataFusionError::Execution(format!("Semantic runtime: {message}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceDescriptor {
    pub scope: String,
    pub schema_revision: String,
    /// An application/connector-issued opaque authorization boundary, never a credential.
    pub authorization_scope: String,
    pub revision: String,
}
impl SourceDescriptor {
    pub fn cache_key(&self) -> String {
        fingerprint(&[
            self.scope.as_bytes(),
            self.schema_revision.as_bytes(),
            self.authorization_scope.as_bytes(),
            self.revision.as_bytes(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancellation_and_shared_budgets() {
        let query = QueryContext::new(QueryOptions {
            max_remote_bytes: 10,
            ..Default::default()
        })
        .unwrap();
        query.charge_remote(6).unwrap();
        assert!(query.clone().charge_remote(5).is_err());
        assert_eq!(query.metrics().remote_bytes, 6);
        query.cancel();
        assert!(
            query
                .run(std::future::pending::<Result<()>>())
                .await
                .is_err()
        );
        let next = QueryContext::new(QueryOptions::default()).unwrap();
        next.check().unwrap();
        assert_ne!(query.id, next.id);
    }
    #[test]
    fn identities_are_unambiguous() {
        assert_ne!(fingerprint(&[b"ab", b"c"]), fingerprint(&[b"a", b"bc"]));
    }
}
