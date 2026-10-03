//! A cache belongs to one borrowed engine: backend validation cannot leak across
//! sessions with superficially identical catalog names but different bindings.
use super::*;
use semantic_engine::Engine;
use semantic_plan::graph::{GraphIntentQuery, GraphQuery};
use semantic_plan::typed::{RowOperation, RowQuery, SemanticQuery};
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
    artifact: CachedArtifact,
    bytes: usize,
    used: u64,
}
enum CachedArtifact {
    Row(CompiledQuery),
    Graph {
        query: graph::CompiledGraph,
        record: GraphRecord,
    },
}
#[derive(Clone)]
struct GraphRecord {
    bound_digest: Option<String>,
    request_digest: Option<String>,
    request_spans_validated: bool,
    relational_digest: Option<String>,
    artifact_digest: Option<String>,
    definition_refs: Vec<semantic_catalog::ObjectRef>,
    dispositions: Vec<(String, &'static str)>,
    obligations: Vec<(
        &'static str,
        semantic_catalog::ObjectRef,
        semantic_catalog::ObjectRef,
        &'static str,
    )>,
}
impl GraphRecord {
    fn capture(record: &CompilationRecord) -> Self {
        Self {
            bound_digest: record.bound_digest.clone(),
            request_digest: record.request_digest.clone(),
            request_spans_validated: record.request_spans_validated,
            relational_digest: record.relational_digest.clone(),
            artifact_digest: record.artifact_digest.clone(),
            definition_refs: record.definition_refs.clone(),
            dispositions: record
                .requirement_dispositions
                .iter()
                .map(|item| (item.requirement_id.clone(), item.rule))
                .collect(),
            obligations: record
                .execution_obligations
                .iter()
                .map(|item| {
                    (
                        item.rule,
                        item.relationship.clone(),
                        item.relation.clone(),
                        item.status,
                    )
                })
                .collect(),
        }
    }
    fn restore(&self, record: &mut CompilationRecord) {
        record.bound_digest = self.bound_digest.clone();
        record.request_digest = self.request_digest.clone();
        record.request_spans_validated = self.request_spans_validated;
        record.relational_digest = self.relational_digest.clone();
        record.artifact_digest = self.artifact_digest.clone();
        record.definition_refs = self.definition_refs.clone();
        record.requirement_dispositions = self
            .dispositions
            .iter()
            .map(|(requirement_id, rule)| RequirementDisposition {
                requirement_id: requirement_id.clone(),
                rule,
                result: "reused_validated",
            })
            .collect();
        record.execution_obligations = self
            .obligations
            .iter()
            .map(
                |(rule, relationship, relation, status)| ExecutionObligation {
                    rule,
                    relationship: relationship.clone(),
                    relation: relation.clone(),
                    status,
                },
            )
            .collect();
    }
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

fn binding_reuse_eligible(query: &RowQuery) -> bool {
    query.requirements.len() <= 64
        && query.requirements.iter().all(|requirement| {
            matches!(
                &requirement.operation,
                RowOperation::ConceptFilter { .. }
                    | RowOperation::FilterOutput { .. }
                    | RowOperation::Window { .. }
                    | RowOperation::Group { .. }
                    | RowOperation::Aggregate { .. }
                    | RowOperation::OrderOutput { .. }
                    | RowOperation::Project { .. }
                    | RowOperation::Filter { .. }
                    | RowOperation::Order { .. }
                    | RowOperation::Limit { .. }
            )
        })
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
    binding_cache: AnalysisCache<BoundQuery>,
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
            binding_cache: AnalysisCache::new(AnalysisCacheLimits {
                max_entries: options.max_entries,
                max_bytes: options.max_bytes,
                max_lookups: 66,
                max_active_keys: options.max_concurrent,
            })
            .expect("validated compilation-cache limits"),
            options,
            cache: Mutex::new(Cache::default()),
        })
    }

    /// Install a newly published engine. Backend-validated artifacts cannot
    /// cross this boundary; only dependency-checked bound analysis may survive.
    /// Exclusive access guarantees there is no compile in progress.
    pub fn replace_engine(&mut self, engine: Arc<Engine>) {
        self.engine = EngineRef::Shared(engine);
        self.cache = Mutex::new(Cache::default());
    }

    pub(super) async fn bind_analysis(
        &self,
        snapshot: &CatalogSnapshot,
        query: &RowQuery,
        options: &CompileOptions,
        work: &mut Work,
    ) -> Result<(BoundQuery, bool), CompileDiagnostic> {
        options.check()?;
        if !binding_reuse_eligible(query) {
            return bind::bind(snapshot, query, options, work).map(|bound| (bound, false));
        }
        let identity = AnalysisCacheIdentity {
            stage: "single_relation_bound_rows_v1".into(),
            input_digest: semantic_catalog::canonical_digest(&serde_json::json!({
                "query": query,
                "limits": {
                    "max_nodes": options.max_nodes,
                    "max_depth": options.max_depth,
                    "max_input_bytes": options.max_input_bytes,
                    "max_index_objects": options.max_index_objects,
                    "max_page_offset": options.max_page_offset,
                    "max_page_fetch": options.max_page_fetch,
                }
            })),
            access_scope_revision: semantic_catalog::canonical_digest(&serde_json::json!(
                options.allowed_relations
            )),
            parameter_digest: semantic_catalog::canonical_digest(&serde_json::json!({
                "context": options.request_context,
                "evidence": options.request_evidence,
            })),
            renderer_revision: "bound-row-v1".into(),
            function_revision: MVP_EXECUTION_PROFILE_REVISION.to_string(),
            acceptance_revision: PIPELINE_REVISION.to_string(),
        };
        let built = std::sync::atomic::AtomicBool::new(false);
        let built_for_build = &built;
        let cached = self
            .binding_cache
            .get_or_build(snapshot, identity, || async {
                built_for_build.store(true, std::sync::atomic::Ordering::Relaxed);
                let bound = bind::bind(snapshot, query, options, work)?;
                let bytes = serde_json::to_vec(&bound)
                    .expect("bound query serializes")
                    .len();
                let mut dependencies = vec![
                    LookupDependency::Relation {
                        name: query.input.relation.clone(),
                    },
                    LookupDependency::PolicySet {
                        relation: query.input.relation.clone(),
                    },
                ];
                for requirement in &query.requirements {
                    if let RowOperation::ConceptFilter { concept, .. } = &requirement.operation {
                        dependencies.push(LookupDependency::Candidates {
                            relation: query.input.relation.clone(),
                            kind: CandidateKind::Concept,
                            phrase: concept.clone(),
                        });
                    }
                }
                Ok::<_, CompileDiagnostic>((bound, dependencies, bytes))
            })
            .await
            .map_err(|error| match error {
                AnalysisCacheError::Build(error) => error,
                AnalysisCacheError::Dependencies(_) => diagnostic(
                    "cache_configuration",
                    "Bound analysis exceeded its dependency limit",
                ),
                AnalysisCacheError::Admission => diagnostic(
                    "admission_closed",
                    "Too many distinct bound analyses are already in progress",
                ),
            })?;
        options.check()?;
        let mut bound = (*cached).clone();
        bound.snapshot_id = snapshot.id().into();
        Ok((bound, !built.load(std::sync::atomic::Ordering::Relaxed)))
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
        let mut request_guard = options.metrics.as_ref().map(|metrics| metrics.begin());
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
                    &serde_json::json!({"kind":"row","acceptance":"strict/v1","pipeline":PIPELINE_REVISION,"execution_profile":MVP_EXECUTION_PROFILE_REVISION,"snapshot":snapshot.id(),"scope":options.allowed_relations,"context":options.request_context,"evidence":options.request_evidence,"budgets":{"max_nodes":options.max_nodes,"max_depth":options.max_depth,"max_input_bytes":options.max_input_bytes,"max_index_objects":options.max_index_objects,"max_page_offset":options.max_page_offset,"max_page_fetch":options.max_page_fetch},"proposal":proposal}),
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
                        match &entry.artifact {
                            CachedArtifact::Row(query) => query.clone(),
                            CachedArtifact::Graph { .. } => unreachable!("row cache key is typed"),
                        }
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
                let query = compile_bound(
                    &self.engine,
                    &snapshot,
                    proposal,
                    &options,
                    &mut record,
                    Some(self),
                )
                .await?;
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
                            artifact: CachedArtifact::Row(query.clone()),
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
        let completed = finish(result, record, start, options.metrics.as_deref());
        if let Some(guard) = &mut request_guard {
            guard.complete();
        }
        completed
    }

    /// Cache an accepted graph using the same engine-owned limits and admission
    /// as row compilations. An intent supplies evidence as part of the key.
    pub async fn compile_graph_intent(
        &self,
        intent: GraphIntentQuery,
        mut options: CompileOptions,
    ) -> TypedCompilation {
        options.graph_request_evidence = Some(intent.evidence);
        self.compile_graph(intent.query, options).await
    }

    #[tracing::instrument(name = "semantic.compile_graph_cached", skip_all)]
    pub async fn compile_graph(
        &self,
        proposal: GraphQuery,
        options: CompileOptions,
    ) -> TypedCompilation {
        let options = options.start();
        let mut request_guard = options.metrics.as_ref().map(|metrics| metrics.begin());
        let start = Instant::now();
        let mut record = CompilationRecord::new("structured_graph_v1_cached");
        let result = {
            let work = async {
                graph::preflight_graph(&proposal, &options)?;
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
                let key = semantic_catalog::canonical_digest(&serde_json::json!({
                    "kind":"graph","acceptance":"structured_graph_v1",
                    "pipeline":PIPELINE_REVISION,"execution_profile":MVP_EXECUTION_PROFILE_REVISION,
                    "snapshot":snapshot.id(),"scope":options.allowed_relations,
                    "context":options.request_context,"evidence":options.graph_request_evidence,
                    "budgets":{"max_nodes":options.max_nodes,"max_depth":options.max_depth,
                        "max_input_bytes":options.max_input_bytes,"max_index_objects":options.max_index_objects,
                        "max_page_offset":options.max_page_offset,"max_page_fetch":options.max_page_fetch},
                    "proposal":proposal,
                }));
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
                        match &entry.artifact {
                            CachedArtifact::Graph { query, record } => {
                                (query.clone(), record.clone())
                            }
                            CachedArtifact::Row(_) => unreachable!("graph cache key is typed"),
                        }
                    });
                    if cached.is_some() {
                        cache.stats.hits += 1;
                    } else {
                        cache.stats.misses += 1;
                    }
                    cached
                };
                if let Some((query, cached_record)) = cached {
                    if query.sql().statement().len() > options.max_sql_bytes {
                        return Err(diagnostic(
                            "sql_limit",
                            "Cached graph SQL exceeds the artifact byte budget",
                        ));
                    }
                    cached_record.restore(&mut record);
                    record.cache_status = "hit_same_snapshot_and_scope";
                    options.check()?;
                    return Ok(TypedOutcome::CompiledGraph {
                        query: Box::new(query),
                    });
                }
                record.cache_status = "miss";
                let query = graph::build(&self.engine, proposal, &options, &mut record).await?;
                if let Ok(serialized) = bounded_json(&query, self.options.max_bytes) {
                    let bytes = serialized.len();
                    let metadata = GraphRecord::capture(&record);
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
                            artifact: CachedArtifact::Graph {
                                query: query.clone(),
                                record: metadata,
                            },
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
                Ok(TypedOutcome::CompiledGraph {
                    query: Box::new(query),
                })
            };
            tokio::select! {
                biased;
                _ = options.cancellation.cancelled() => Err(diagnostic("cancelled", "Compilation was cancelled")),
                result = tokio::time::timeout(options.timeout, work) => result.unwrap_or_else(|_| Err(diagnostic("deadline", "Compilation deadline exhausted"))),
            }
        };
        let completed = finish(result, record, start, options.metrics.as_deref());
        if let Some(guard) = &mut request_guard {
            guard.complete();
        }
        completed
    }
}
