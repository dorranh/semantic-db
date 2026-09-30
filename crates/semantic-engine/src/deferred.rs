//! Opt-in demand-driven provider resolution against recorded schema contracts.
//! Publication/planning can use schemas without contacting every source.
use crate::{RelationBackend, TableProvider};
use async_trait::async_trait;
use datafusion::{
    catalog::Session,
    error::Result,
    logical_expr::{Expr, TableType},
    physical_plan::ExecutionPlan,
};
use semantic_catalog::{Catalog, Relation, SchemaRef};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

#[derive(Debug, Clone)]
pub struct DeferredOptions {
    pub max_cached_providers: usize,
    pub max_concurrent_resolutions: usize,
    /// Maximum distinct cold table resolutions admitted to either wait for a
    /// backend permit or run. Same-table followers coalesce before admission.
    pub max_active_cold_keys: usize,
}
impl Default for DeferredOptions {
    fn default() -> Self {
        Self {
            max_cached_providers: 256,
            max_concurrent_resolutions: 8,
            max_active_cold_keys: 256,
        }
    }
}
#[derive(Debug, Default)]
pub struct DeferredProviderStats {
    resolutions: AtomicU64,
    cache_hits: AtomicU64,
    evictions: AtomicU64,
    admission_rejections: AtomicU64,
    active_cold_keys: AtomicUsize,
    peak_active_cold_keys: AtomicUsize,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeferredProviderReport {
    pub resolutions: u64,
    pub cache_hits: u64,
    pub evictions: u64,
    pub cached_providers: usize,
    pub admission_rejections: u64,
    pub active_cold_keys: usize,
    pub peak_active_cold_keys: usize,
}
struct Cached {
    provider: Arc<dyn TableProvider>,
    last_use: u64,
}
struct Shared<B> {
    backend: Arc<B>,
    options: DeferredOptions,
    cache: Mutex<BTreeMap<String, Cached>>,
    clock: AtomicU64,
    semaphore: Semaphore,
    stats: DeferredProviderStats,
}
struct ColdAdmission<'a> {
    active: &'a AtomicUsize,
}
impl Drop for ColdAdmission<'_> {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}
impl<B> Shared<B> {
    fn admit_cold(&self) -> Result<ColdAdmission<'_>> {
        let active = self
            .stats
            .active_cold_keys
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < self.options.max_active_cold_keys).then_some(count + 1)
            })
            .map_err(|_| {
                self.stats
                    .admission_rejections
                    .fetch_add(1, Ordering::Relaxed);
                semantic_runtime::failure(
                    "deferred provider admission exhausted: too many active cold tables",
                )
            })?
            + 1;
        self.stats
            .peak_active_cold_keys
            .fetch_max(active, Ordering::Relaxed);
        Ok(ColdAdmission {
            active: &self.stats.active_cold_keys,
        })
    }
}
/// Wrap a backend before passing it to `Engine::from_catalog`. The resulting
/// engine validates view schemas using recorded base schemas. Live base
/// providers resolve on the first selected physical scan and use a bounded LRU.
/// Generic wrappers conservatively disable planner pushdown; connector scans
/// still receive projection/limit contracts and the normal execution context.
pub struct DeferredBackend<B> {
    shared: Arc<Shared<B>>,
}
impl<B: RelationBackend + 'static> DeferredBackend<B> {
    pub fn new(backend: Arc<B>, options: DeferredOptions) -> Result<Self> {
        if options.max_concurrent_resolutions == 0
            || options.max_cached_providers == 0
            || options.max_active_cold_keys == 0
        {
            return Err(semantic_runtime::failure(
                "deferred provider limits must be positive",
            ));
        }
        let concurrency = options.max_concurrent_resolutions;
        Ok(Self {
            shared: Arc::new(Shared {
                backend,
                options,
                cache: Mutex::new(BTreeMap::new()),
                clock: AtomicU64::new(0),
                semaphore: Semaphore::new(concurrency),
                stats: DeferredProviderStats::default(),
            }),
        })
    }
    pub fn report(&self) -> DeferredProviderReport {
        DeferredProviderReport {
            resolutions: self.shared.stats.resolutions.load(Ordering::Relaxed),
            cache_hits: self.shared.stats.cache_hits.load(Ordering::Relaxed),
            evictions: self.shared.stats.evictions.load(Ordering::Relaxed),
            cached_providers: self
                .shared
                .cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len(),
            admission_rejections: self
                .shared
                .stats
                .admission_rejections
                .load(Ordering::Relaxed),
            active_cold_keys: self.shared.stats.active_cold_keys.load(Ordering::Acquire),
            peak_active_cold_keys: self
                .shared
                .stats
                .peak_active_cold_keys
                .load(Ordering::Relaxed),
        }
    }
}
impl<B: RelationBackend + 'static> RelationBackend for DeferredBackend<B> {
    async fn resolve(&self, relation: &Relation) -> Result<Arc<dyn TableProvider>> {
        let key = Catalog::from_relations([relation.clone()])
            .expect("single relation")
            .snapshot()
            .id()
            .to_owned();
        Ok(Arc::new(DeferredTable {
            relation: relation.clone(),
            key,
            shared: self.shared.clone(),
            resolution: AsyncMutex::new(()),
        }))
    }
}
struct DeferredTable<B> {
    relation: Relation,
    key: String,
    shared: Arc<Shared<B>>,
    resolution: AsyncMutex<()>,
}
impl<B> fmt::Debug for DeferredTable<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeferredTable")
            .field("name", &self.relation.name)
            .finish_non_exhaustive()
    }
}
impl<B: RelationBackend + 'static> DeferredTable<B> {
    fn cached(&self) -> Option<Arc<dyn TableProvider>> {
        let mut cache = self.shared.cache.lock().unwrap_or_else(|e| e.into_inner());
        let entry = cache.get_mut(&self.key)?;
        entry.last_use = self.shared.clock.fetch_add(1, Ordering::Relaxed);
        self.shared.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
        Some(entry.provider.clone())
    }
    async fn provider(&self) -> Result<Arc<dyn TableProvider>> {
        if let Some(provider) = self.cached() {
            return Ok(provider);
        }
        let _relation_guard = self.resolution.lock().await;
        if let Some(provider) = self.cached() {
            return Ok(provider);
        }
        let _admission = self.shared.admit_cold()?;
        let _permit = self
            .shared
            .semaphore
            .acquire()
            .await
            .map_err(|_| semantic_runtime::failure("provider resolution pool closed"))?;
        self.shared
            .stats
            .resolutions
            .fetch_add(1, Ordering::Relaxed);
        let provider = self.shared.backend.resolve(&self.relation).await?;
        if provider.schema() != self.relation.schema {
            return Err(semantic_runtime::failure(
                "deferred provider schema drift: rebuild the catalog binding",
            ));
        }
        let mut cache = self.shared.cache.lock().unwrap_or_else(|e| e.into_inner());
        if !cache.contains_key(&self.key) && cache.len() >= self.shared.options.max_cached_providers
        {
            let oldest = cache
                .iter()
                .min_by_key(|(_, entry)| entry.last_use)
                .map(|(key, _)| key.clone())
                .expect("positive bounded cache");
            cache.remove(&oldest);
            self.shared.stats.evictions.fetch_add(1, Ordering::Relaxed);
        }
        cache.insert(
            self.key.clone(),
            Cached {
                provider: provider.clone(),
                last_use: self.shared.clock.fetch_add(1, Ordering::Relaxed),
            },
        );
        Ok(provider)
    }
}
#[async_trait]
impl<B: RelationBackend + 'static> TableProvider for DeferredTable<B> {
    fn schema(&self) -> SchemaRef {
        self.relation.schema.clone()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        self.provider()
            .await?
            .scan(state, projection, filters, limit)
            .await
    }
}
