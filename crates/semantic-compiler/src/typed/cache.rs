//! A cache belongs to one borrowed engine: backend validation cannot leak across
//! sessions with superficially identical catalog names but different bindings.
use super::*;
use semantic_engine::Engine;
use semantic_plan::typed::SemanticQuery;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
    time::Instant,
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

#[derive(Debug, Clone)]
pub struct CompilationCacheOptions {
    pub max_entries: usize,
    pub max_bytes: usize,
    pub max_concurrent: usize,
}
impl Default for CompilationCacheOptions {
    fn default() -> Self {
        Self {
            max_entries: 128,
            max_bytes: 16 * 1024 * 1024,
            max_concurrent: 4,
        }
    }
}
#[derive(Debug, Default, Clone, Serialize)]
pub struct CompilationCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
    /// Serialized artifact bytes; allocator/RSS overhead is not measured here.
    pub retained_bytes: usize,
}
struct Entry {
    query: CompiledQuery,
    bytes: usize,
    used: u64,
}
#[derive(Default)]
struct Cache {
    entries: BTreeMap<String, Entry>,
    active: BTreeMap<String, Weak<AsyncMutex<()>>>,
    clock: u64,
    stats: CompilationCacheStats,
}
enum EngineRef<'a> {
    Borrowed(&'a Engine),
    Shared(Arc<Engine>),
}
impl std::ops::Deref for EngineRef<'_> {
    type Target = Engine;
    fn deref(&self) -> &Engine {
        match self {
            Self::Borrowed(engine) => engine,
            Self::Shared(engine) => engine,
        }
    }
}
pub struct CompilationSession<'a> {
    engine: EngineRef<'a>,
    options: CompilationCacheOptions,
    cache: Mutex<Cache>,
    admission: Semaphore,
}
impl<'a> CompilationSession<'a> {
    pub fn new(
        engine: &'a Engine,
        options: CompilationCacheOptions,
    ) -> Result<Self, CompileDiagnostic> {
        Self::create(EngineRef::Borrowed(engine), options)
    }
    pub fn from_shared(
        engine: Arc<Engine>,
        options: CompilationCacheOptions,
    ) -> Result<CompilationSession<'static>, CompileDiagnostic> {
        CompilationSession::create(EngineRef::Shared(engine), options)
    }
    fn create(
        engine: EngineRef<'a>,
        options: CompilationCacheOptions,
    ) -> Result<Self, CompileDiagnostic> {
        if options.max_entries == 0
            || options.max_bytes == 0
            || options.max_concurrent == 0
            || options.max_concurrent > Semaphore::MAX_PERMITS
        {
            return Err(diagnostic(
                "cache_configuration",
                "Cache and concurrency limits must be positive and bounded",
            ));
        }
        Ok(Self {
            engine,
            admission: Semaphore::new(options.max_concurrent),
            options,
            cache: Mutex::new(Cache::default()),
        })
    }
    pub fn stats(&self) -> CompilationCacheStats {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stats
            .clone()
    }
    /// Admission and same-key coordination are inside the request deadline.
    /// Only accepted immutable artifacts are cached. Full snapshot and access
    /// scope keys conservatively invalidate all positive and negative lookups.
    #[tracing::instrument(name = "semantic.compile_cached", skip_all)]
    pub async fn compile(
        &self,
        proposal: SemanticQuery,
        options: CompileOptions,
    ) -> TypedCompilation {
        let options = options.start();
        let start = Instant::now();
        let mut record = CompilationRecord::new("structured_semantic_v1_cached");
        let result = {
            let work = async {
                preflight(&proposal, &options)?;
                let admission_start = Instant::now();
                let _permit = self.admission.acquire().await.map_err(|_| {
                    diagnostic("admission_closed", "Compilation admission is closed")
                })?;
                record.stage(
                    "admission",
                    admission_start,
                    &Ok::<_, CompileDiagnostic>(()),
                );
                options.check()?;
                let snapshot = self.engine.catalog().snapshot();
                record.snapshot_id = Some(snapshot.id().into());
                let key = semantic_catalog::canonical_digest(
                    &serde_json::json!({"pipeline":PIPELINE_REVISION,"snapshot":snapshot.id(),"scope":options.allowed_relations,"context":options.request_context,"evidence":options.request_evidence,"proposal":proposal}),
                );
                let lock = {
                    let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                    cache.active.retain(|_, value| value.strong_count() > 0);
                    match cache.active.get(&key).and_then(Weak::upgrade) {
                        Some(lock) => lock,
                        None => {
                            let lock = Arc::new(AsyncMutex::new(()));
                            cache.active.insert(key.clone(), Arc::downgrade(&lock));
                            lock
                        }
                    }
                };
                let _guard = lock.lock().await;
                options.check()?;
                let cached = {
                    let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                    cache.clock += 1;
                    let clock = cache.clock;
                    let cached = cache.entries.get_mut(&key).map(|entry| {
                        entry.used = clock;
                        entry.query.clone()
                    });
                    if cached.is_some() {
                        cache.stats.hits += 1;
                    } else {
                        cache.stats.misses += 1;
                    }
                    cached
                };
                if let Some(query) = cached {
                    if query.sql.statement().len() > options.max_sql_bytes {
                        return Err(diagnostic(
                            "sql_limit",
                            "Cached SQL exceeds the artifact byte budget",
                        ));
                    }
                    record.cache_status = "hit_same_snapshot_and_scope";
                    record_bound(&query.bound, &mut record, RequirementScope::Row);
                    if let Some(evidence) = &query.request_evidence {
                        record.request_digest = Some(semantic_catalog::canonical_digest(
                            &serde_json::json!(evidence.original_request),
                        ));
                        record.request_spans_validated = true;
                    }
                    for disposition in &mut record.requirement_dispositions {
                        disposition.result = "reused_validated";
                    }
                    record.relational_digest = Some(semantic_catalog::canonical_digest(
                        &serde_json::to_value(&query.relational)
                            .expect("relational plan serializes"),
                    ));
                    record.artifact_digest = Some(artifact_digest(&query));
                    options.check()?;
                    return Ok(TypedOutcome::Compiled {
                        query: Box::new(query),
                    });
                }
                record.cache_status = "miss";
                let query =
                    compile_bound(&self.engine, &snapshot, proposal, &options, &mut record).await?;
                if let Ok(serialized) = bounded_json(&query, self.options.max_bytes) {
                    let bytes = serialized.len();
                    let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                    while cache.entries.len() >= self.options.max_entries
                        || bytes
                            > self
                                .options
                                .max_bytes
                                .saturating_sub(cache.stats.retained_bytes)
                    {
                        let Some(oldest) = cache
                            .entries
                            .iter()
                            .min_by_key(|(_, entry)| entry.used)
                            .map(|(key, _)| key.clone())
                        else {
                            break;
                        };
                        let removed = cache.entries.remove(&oldest).expect("cache entry");
                        cache.stats.retained_bytes -= removed.bytes;
                        cache.stats.evictions += 1;
                    }
                    cache.clock += 1;
                    let used = cache.clock;
                    cache.entries.insert(
                        key,
                        Entry {
                            query: query.clone(),
                            bytes,
                            used,
                        },
                    );
                    cache.stats.retained_bytes += bytes;
                    cache.stats.entries = cache.entries.len();
                } else {
                    record.cache_status = "miss_artifact_exceeds_retention_budget";
                }
                options.check()?;
                Ok(TypedOutcome::Compiled {
                    query: Box::new(query),
                })
            };
            tokio::select! {
                biased;
                _ = options.cancellation.cancelled() => Err(diagnostic("cancelled","Compilation was cancelled")),
                result = tokio::time::timeout(options.timeout,work) => result.unwrap_or_else(|_|Err(diagnostic("deadline","Compilation deadline exhausted"))),
            }
        };
        finish(result, record, start, options.metrics.as_deref())
    }
}
