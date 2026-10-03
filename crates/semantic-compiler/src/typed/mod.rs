//! Opt-in typed row compiler. SQL compatibility compilation stays separate.
//! Records are compiler-owned and available without a tracing subscriber/exporter.
mod bind;
mod cache;
pub use cache::{CompilationCacheOptions, CompilationCacheStats, CompilationSession};
mod calendar_spine;
mod dependency_cache;
pub use dependency_cache::{
    AnalysisCache, AnalysisCacheError, AnalysisCacheIdentity, AnalysisCacheLimits,
    AnalysisCacheStats, CandidateKind, DependencySet, LookupDependency,
};
mod diagnostic;
pub use diagnostic::{
    DiagnosticDetails, DiagnosticKind, DiagnosticStage, DiagnosticTerminal, NextAction,
    Recoverability, diagnostic_details,
};
mod intent;
mod literal;
mod metrics;
mod observation;
mod prepared;
mod scalar;
pub use intent::compile_intent;
pub use metrics::{CompilerMetrics, CompilerMetricsSnapshot, LATENCY_BUCKETS_US};
pub use observation::{Observation, ObservationOutcome, ObservationQueue, ObservationQueueError};
pub use prepared::{ParameterDeclaration, PreparedReferenceContext, PreparedRows, PreparedType};
mod lower;
mod replay;
mod temporal;
pub use replay::{
    PIPELINE_REVISION, ReplayBundle, RuleDecision, StageArtifact, StageReplayOutcome,
};
pub use temporal::{Calendar, ContextOrigin, RequestContext, TemporalResolution};
mod graph;
pub use graph::{CompiledGraph, GraphReplayBundle, compile_graph, compile_graph_intent};

use std::{
    collections::BTreeSet,
    io::Write,
    sync::Arc,
    time::{Duration, Instant},
};

use datafusion::dataframe::DataFrame;
use semantic_catalog::CatalogSnapshot;
use semantic_engine::{Engine, MVP_EXECUTION_PROFILE_REVISION, QueryExecution, QueryOptions};
use semantic_plan::typed::{RowOperation, RowPredicate, RowQuery};
use serde::Serialize;
use tokio::sync::watch;
use tracing::Instrument;

pub use bind::BoundQuery;
pub use lower::{RelationalPlan, SqlArtifact};

#[derive(Debug, Clone)]
pub struct Cancellation(Arc<watch::Sender<bool>>);
impl Default for Cancellation {
    fn default() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }
}
impl Cancellation {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    pub async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|value| *value).await;
    }
}

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub metrics: Option<Arc<CompilerMetrics>>,
    pub request_evidence: Option<semantic_plan::typed::RequestEvidence>,
    pub graph_request_evidence: Option<semantic_plan::graph::GraphRequestEvidence>,
    pub request_context: Option<RequestContext>,
    pub timeout: Duration,
    pub cancellation: Cancellation,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_input_bytes: usize,
    pub max_index_objects: usize,
    pub max_sql_bytes: usize,
    pub max_page_offset: u32,
    pub max_page_fetch: u32,
    pub allowed_relations: Option<std::collections::BTreeSet<String>>,
    deadline: Option<Instant>,
}
impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            request_context: None,
            request_evidence: None,
            graph_request_evidence: None,
            metrics: None,
            timeout: Duration::from_secs(30),
            cancellation: Cancellation::default(),
            max_nodes: 512,
            max_depth: 32,
            max_input_bytes: 64 * 1024,
            max_index_objects: 2_000_000,
            max_sql_bytes: 128 * 1024,
            max_page_offset: 1_000_000,
            max_page_fetch: 1_000_000,
            allowed_relations: None,
            deadline: None,
        }
    }
}
impl CompileOptions {
    fn start(mut self) -> Self {
        self.deadline = Instant::now().checked_add(self.timeout);
        // Keep recursion safe even when callers loosen their work budget.
        self.max_depth = self.max_depth.min(64);
        self.max_nodes = self.max_nodes.min(8192);
        self
    }
    pub fn check(&self) -> Result<(), CompileDiagnostic> {
        if self.cancellation.is_cancelled() {
            return Err(diagnostic("cancelled", "Compilation was cancelled"));
        }
        if self.deadline.is_some_and(|t| Instant::now() >= t) {
            return Err(diagnostic("deadline", "Compilation deadline exhausted"));
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct CompileDiagnostic {
    pub code: String,
    pub message: String,
    pub details: DiagnosticDetails,
}
impl std::fmt::Debug for CompileDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompileDiagnostic")
            .field("code", &self.code)
            .field("stage", &self.details.stage)
            .field("kind", &self.details.kind)
            .finish_non_exhaustive()
    }
}
pub fn diagnostic(code: &str, message: &str) -> CompileDiagnostic {
    CompileDiagnostic {
        code: code.into(),
        message: message.into(),
        details: diagnostic_details(code),
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Work {
    pub mapped_value_bytes: usize,
    pub relations_looked_up: usize,
    pub fields_looked_up: usize,
    pub nodes_visited: usize,
    /// Sum over every attempt, including repeated context and repair history.
    pub graph_nodes_visited: usize,
    pub graph_edges_visited: usize,
    pub graph_outputs_checked: usize,
}
#[derive(Debug, Serialize)]
pub struct StageRecord {
    pub stage: &'static str,
    pub code: String,
    pub elapsed_micros: u128,
}
#[derive(Serialize)]
pub struct CompilationRecord {
    pub compilation_id: String,
    pub compiler_build: &'static str,
    pub pipeline_revision: &'static str,
    pub execution_profile_revision: &'static str,
    pub bound_digest: Option<String>,
    pub request_digest: Option<String>,
    pub request_spans_validated: bool,
    pub relational_digest: Option<String>,
    pub requirement_dispositions: Vec<RequirementDisposition>,
    pub definition_refs: Vec<semantic_catalog::ObjectRef>,
    pub execution_obligations: Vec<ExecutionObligation>,
    pub cache_status: &'static str,
    pub version: u32,
    pub mode: &'static str,
    pub outcome: &'static str,
    pub snapshot_id: Option<String>,
    pub stages: Vec<StageRecord>,
    pub work: Work,
    pub artifact_digest: Option<String>,
    pub elapsed_micros: u128,
}
impl std::fmt::Debug for CompilationRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompilationRecord")
            .field("version", &self.version)
            .field("mode", &self.mode)
            .field("outcome", &self.outcome)
            .field("stage_count", &self.stages.len())
            .field("requirement_count", &self.requirement_dispositions.len())
            .field("definition_count", &self.definition_refs.len())
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Serialize)]
pub struct RequirementDisposition {
    pub requirement_id: String,
    pub rule: &'static str,
    pub result: &'static str,
}
#[derive(Debug, Serialize)]
pub struct ExecutionObligation {
    pub rule: &'static str,
    pub relationship: semantic_catalog::ObjectRef,
    pub relation: semantic_catalog::ObjectRef,
    /// Compilation never runs data checks. The guard executes on every query.
    pub status: &'static str,
}
impl CompilationRecord {
    pub fn new(mode: &'static str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let ordinal = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            compilation_id: format!("{epoch:x}-{ordinal:x}"),
            compiler_build: env!("CARGO_PKG_VERSION"),
            pipeline_revision: PIPELINE_REVISION,
            execution_profile_revision: MVP_EXECUTION_PROFILE_REVISION,
            bound_digest: None,
            request_digest: None,
            request_spans_validated: false,
            relational_digest: None,
            requirement_dispositions: Vec::new(),
            definition_refs: Vec::new(),
            execution_obligations: Vec::new(),
            cache_status: "disabled",
            version: 1,
            mode,
            outcome: "pending",
            snapshot_id: None,
            stages: Vec::new(),
            work: Work::default(),
            artifact_digest: None,
            elapsed_micros: 0,
        }
    }
    fn stage<T>(
        &mut self,
        stage: &'static str,
        start: Instant,
        result: &Result<T, CompileDiagnostic>,
    ) {
        let code = result
            .as_ref()
            .map(|_| "ok")
            .unwrap_or_else(|e| e.code.as_str());
        tracing::debug!(
            stage,
            code,
            elapsed_micros = start.elapsed().as_micros() as u64,
            "compiler stage"
        );
        self.stages.push(StageRecord {
            stage,
            code: code.into(),
            elapsed_micros: start.elapsed().as_micros(),
        });
    }
}

#[derive(Serialize)]
pub struct TypedCompilation {
    pub version: u32,
    pub outcome: TypedOutcome,
    pub record: CompilationRecord,
}
impl std::fmt::Debug for TypedCompilation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TypedCompilation")
            .field("version", &self.version)
            .field("outcome", &self.outcome)
            .field("record", &self.record)
            .finish()
    }
}
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TypedOutcome {
    CompiledGraph {
        query: Box<CompiledGraph>,
    },
    Compiled {
        query: Box<CompiledQuery>,
    },
    NeedsClarification {
        phrases: Vec<String>,
        question: String,
    },
    Unsupported {
        reason: String,
    },
    Unresolved {
        diagnostic: CompileDiagnostic,
    },
    Rejected {
        diagnostic: CompileDiagnostic,
    },
    ProviderFailure {
        diagnostic: CompileDiagnostic,
    },
}
impl std::fmt::Debug for TypedOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CompiledGraph { .. } => formatter.write_str("CompiledGraph"),
            Self::Compiled { .. } => formatter.write_str("Compiled"),
            Self::NeedsClarification { .. } => formatter.write_str("NeedsClarification"),
            Self::Unsupported { .. } => formatter.write_str("Unsupported"),
            Self::Unresolved { diagnostic } => formatter
                .debug_tuple("Unresolved")
                .field(&diagnostic.code)
                .finish(),
            Self::Rejected { diagnostic } => formatter
                .debug_tuple("Rejected")
                .field(&diagnostic.code)
                .finish(),
            Self::ProviderFailure { diagnostic } => formatter
                .debug_tuple("ProviderFailure")
                .field(&diagnostic.code)
                .finish(),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct CompiledQuery {
    request_evidence: Option<semantic_plan::typed::RequestEvidence>,
    request_context: Option<RequestContext>,
    intent: RowQuery,
    bound: BoundQuery,
    relational: RelationalPlan,
    execution_profile_revision: &'static str,
    required_relations: BTreeSet<String>,
    restricted_scope: bool,
    sql: SqlArtifact,
}
impl std::fmt::Debug for CompiledQuery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompiledQuery")
            .field("has_request_evidence", &self.request_evidence.is_some())
            .field("relation_count", &self.required_relations.len())
            .field("output_count", &self.sql.expected_output().len())
            .finish_non_exhaustive()
    }
}
impl CompiledQuery {
    pub fn intent(&self) -> &RowQuery {
        &self.intent
    }
    pub fn bound(&self) -> &BoundQuery {
        &self.bound
    }
    pub fn relational(&self) -> &RelationalPlan {
        &self.relational
    }
    pub fn sql(&self) -> &SqlArtifact {
        &self.sql
    }
    pub fn execution_profile_revision(&self) -> &str {
        self.execution_profile_revision
    }
    fn check_snapshot(&self, engine: &Engine) -> Result<(), CompileDiagnostic> {
        if engine.catalog().snapshot().id() != self.bound.snapshot_id() {
            return Err(diagnostic(
                "snapshot_mismatch",
                "Recompile against the current catalog before planning or executing",
            ));
        }
        Ok(())
    }
    fn check_execution_scope(
        &self,
        allowed_relations: Option<&BTreeSet<String>>,
    ) -> Result<(), CompileDiagnostic> {
        if let Some(allowed) = allowed_relations {
            if self.required_relations.is_subset(allowed) {
                return Ok(());
            }
            return Err(diagnostic(
                "execution_scope",
                "Current execution scope does not authorize every compiled relation",
            ));
        }
        if self.restricted_scope {
            return Err(diagnostic(
                "execution_scope",
                "A scope-restricted artifact requires current execution authorization",
            ));
        }
        Ok(())
    }
    /// Rebuild through deterministic DataFusion expressions, without reading rows.
    pub async fn plan_direct(&self, engine: &Engine) -> Result<DataFrame, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(None)?;
        self.relational.plan_direct(engine).await
    }
    pub async fn plan_direct_authorized(
        &self,
        engine: &Engine,
        allowed_relations: &BTreeSet<String>,
    ) -> Result<DataFrame, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(Some(allowed_relations))?;
        self.relational.plan_direct(engine).await
    }
    /// Explicit execution through the engine's parameter and runtime-budget path.
    /// Execute with the engine's source read-consistency and cache policies.
    pub async fn execute_read(
        &self,
        engine: &Engine,
        options: semantic_engine::ReadOptions,
    ) -> Result<semantic_engine::ReadExecution, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(None)?;
        engine
            .execute_read(self.sql.statement(), self.sql.values(), options)
            .await
            .map_err(lower::backend_error)
    }
    pub async fn execute_read_authorized(
        &self,
        engine: &Engine,
        allowed_relations: &BTreeSet<String>,
        options: semantic_engine::ReadOptions,
    ) -> Result<semantic_engine::ReadExecution, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(Some(allowed_relations))?;
        engine
            .execute_read(self.sql.statement(), self.sql.values(), options)
            .await
            .map_err(lower::backend_error)
    }
    pub async fn execute(
        &self,
        engine: &Engine,
        options: QueryOptions,
    ) -> Result<QueryExecution, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(None)?;
        engine
            .execute_parameters(self.sql.statement(), self.sql.values(), options)
            .await
            .map_err(lower::backend_error)
    }
    pub async fn execute_authorized(
        &self,
        engine: &Engine,
        allowed_relations: &BTreeSet<String>,
        options: QueryOptions,
    ) -> Result<QueryExecution, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(Some(allowed_relations))?;
        engine
            .execute_parameters(self.sql.statement(), self.sql.values(), options)
            .await
            .map_err(lower::backend_error)
    }
}

fn bound_required_relations(bound: &BoundQuery) -> BTreeSet<String> {
    let mut relations = BTreeSet::from([bound.input.id.clone()]);
    for requirement in &bound.requirements {
        match &requirement.operation {
            bind::BoundOperation::Related { relationship } => {
                relations.insert(relationship.right.id.clone());
            }
            bind::BoundOperation::Lookup { lookup, .. } => {
                relations.insert(lookup.relationship.right.id.clone());
            }
            bind::BoundOperation::PathLookup { lookups, .. } => {
                relations.extend(
                    lookups
                        .iter()
                        .map(|lookup| lookup.relationship.right.id.clone()),
                );
            }
            bind::BoundOperation::Allocate { allocation, .. } => {
                relations.insert(allocation.bridge.id.clone());
            }
            bind::BoundOperation::CurrencyConvert { rate, .. } => {
                relations.insert(rate.rate_relation.id.clone());
            }
            bind::BoundOperation::BusinessCalendar { calendar, .. } => {
                relations.insert(calendar.calendar_relation.id.clone());
            }
            _ => {}
        }
    }
    relations
}

/// Deterministic entry point; no model provider is required.
#[tracing::instrument(
    name = "semantic.compile",
    skip_all,
    fields(mode = "structured_rows_v1")
)]
pub async fn compile_rows(
    engine: &Engine,
    query: RowQuery,
    options: CompileOptions,
) -> TypedCompilation {
    let options = options.start();
    let mut request_guard = options.metrics.as_ref().map(|metrics| metrics.begin());
    let start = Instant::now();
    let mut record = CompilationRecord::new("structured_rows_v1");
    let result = {
        let work = async {
            options.check()?;
            let snapshot = engine.catalog().snapshot();
            record.snapshot_id = Some(snapshot.id().into());
            compile_bound(engine, &snapshot, query, &options, &mut record, None)
                .await
                .map(|query| TypedOutcome::Compiled {
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

fn finish(
    result: Result<TypedOutcome, CompileDiagnostic>,
    mut record: CompilationRecord,
    start: Instant,
    metrics: Option<&CompilerMetrics>,
) -> TypedCompilation {
    record.stage("complete", start, &result);
    record.elapsed_micros = start.elapsed().as_micros();
    let outcome = result.unwrap_or_else(|error| match error.details.terminal {
        DiagnosticTerminal::Unresolved => TypedOutcome::Unresolved { diagnostic: error },
        DiagnosticTerminal::ProviderFailure => TypedOutcome::ProviderFailure { diagnostic: error },
        DiagnosticTerminal::Rejected => TypedOutcome::Rejected { diagnostic: error },
    });
    record.outcome = match &outcome {
        TypedOutcome::Compiled { .. } | TypedOutcome::CompiledGraph { .. } => "compiled",
        TypedOutcome::NeedsClarification { .. } => "needs_clarification",
        TypedOutcome::Unsupported { .. } => "unsupported",
        TypedOutcome::Unresolved { .. } => "unresolved",
        TypedOutcome::Rejected { .. } => "rejected",
        TypedOutcome::ProviderFailure { .. } => "provider_failure",
    };
    tracing::info!(
        compilation_id = record.compilation_id,
        outcome = record.outcome,
        "compilation completed"
    );
    if let Some(metrics) = metrics {
        let terminal_code = match &outcome {
            TypedOutcome::Unresolved { diagnostic }
            | TypedOutcome::Rejected { diagnostic }
            | TypedOutcome::ProviderFailure { diagnostic } => Some(diagnostic.code.as_str()),
            _ => None,
        };
        metrics.observe(&record, terminal_code);
    }
    TypedCompilation {
        version: 1,
        outcome,
        record,
    }
}

async fn compile_bound(
    engine: &Engine,
    snapshot: &CatalogSnapshot,
    query: RowQuery,
    options: &CompileOptions,
    record: &mut CompilationRecord,
    session: Option<&CompilationSession<'_>>,
) -> Result<CompiledQuery, CompileDiagnostic> {
    // Repairs start a fresh proof record; earlier failed artifacts are not the
    // current proposal's evidence. Stage/call accounting remains cumulative.
    record.bound_digest = None;
    record.request_digest = None;
    record.request_spans_validated = false;
    record.relational_digest = None;
    record.artifact_digest = None;
    record.definition_refs.clear();
    record.execution_obligations.clear();
    record.requirement_dispositions.clear();
    let start = Instant::now();
    let result = async {
        preflight(&query, options)?;
        if let Some(session) = session {
            session
                .bind_analysis(snapshot, &query, options, &mut record.work)
                .await
        } else {
            bind::bind(snapshot, &query, options, &mut record.work).map(|bound| (bound, false))
        }
    }
    .await;
    record.stage("bind", start, &result);
    let (bound, binding_reused) = result?;
    if binding_reused {
        record.cache_status = "binding_hit_revalidated";
    }
    if let Some(evidence) = &options.request_evidence {
        record.request_digest = Some(semantic_catalog::canonical_digest(&serde_json::json!(
            evidence.original_request
        )));
        record.request_spans_validated = true;
    }
    record_bound(&bound, record, RequirementScope::Row);
    let start = Instant::now();
    options.check()?;
    let result = lower::lower(&bound);
    record.stage("lower", start, &result);
    let relational = result?;
    record.relational_digest = Some(semantic_catalog::canonical_digest(
        &serde_json::to_value(&relational).expect("relational plan serializes"),
    ));
    for disposition in &mut record.requirement_dispositions {
        disposition.result = "lowered_and_verified";
    }
    let start = Instant::now();
    options.check()?;
    let sql = relational.emit(snapshot.id());
    if sql.statement().len() > options.max_sql_bytes {
        return Err(diagnostic(
            "sql_limit",
            "SQL artifact byte budget exhausted",
        ));
    }
    let result = async {
        let direct = relational.plan_direct(engine).await?;
        let emitted = engine
            .plan_generated_sql(sql.statement())
            .await
            .map_err(lower::backend_error)?;
        // DFSchema also carries backend-local qualifiers and functional
        // dependencies. The portable output contract is the Arrow row schema.
        if direct.schema().as_arrow() != emitted.schema().as_arrow() {
            return Err(diagnostic(
                "output_contract",
                "Direct and SQL output contracts differ",
            ));
        }
        options.check()?;
        Ok(())
    }
    .instrument(tracing::debug_span!("semantic.backend"))
    .await;
    record.stage("backend", start, &result);
    result?;
    let required_relations = bound_required_relations(&bound);
    let artifact = CompiledQuery {
        request_evidence: options.request_evidence.clone(),
        request_context: options.request_context.clone(),
        intent: query,
        bound,
        relational,
        execution_profile_revision: MVP_EXECUTION_PROFILE_REVISION,
        required_relations,
        restricted_scope: options.allowed_relations.is_some(),
        sql,
    };
    options.check()?;
    record.artifact_digest = Some(artifact_digest(&artifact));
    options.check()?;
    Ok(artifact)
}

fn preflight(query: &RowQuery, options: &CompileOptions) -> Result<(), CompileDiagnostic> {
    options.check()?;
    if options.graph_request_evidence.is_some() {
        return Err(diagnostic(
            "graph_evidence",
            "Graph request evidence cannot cover a standalone row query",
        ));
    }
    if let Some(context) = &options.request_context {
        context.validate()?;
    }
    if query.requirements.len() > options.max_nodes {
        return Err(diagnostic("work_limit", "Requirement budget exhausted"));
    }
    let mut count = query.requirements.len();
    for requirement in &query.requirements {
        if let RowOperation::Filter { predicate }
        | RowOperation::Related {
            predicate: Some(predicate),
            ..
        } = &requirement.operation
        {
            predicate_preflight(predicate, options, &mut count)?;
        }
        if let RowOperation::FilterOutput { predicate, .. } = &requirement.operation {
            predicate_preflight(predicate, options, &mut count)?;
        }
    }
    if let Some(evidence) = &options.request_evidence {
        intent::validate(query, evidence, options)?;
    }
    bounded_json(query, options.max_input_bytes)
        .map(|_| ())
        .map_err(|_| diagnostic("input_limit", "Typed input byte budget exhausted"))
}

fn predicate_preflight<F>(
    predicate: &RowPredicate<F>,
    options: &CompileOptions,
    count: &mut usize,
) -> Result<(), CompileDiagnostic> {
    let mut stack = vec![(predicate, 1)];
    while let Some((predicate, depth)) = stack.pop() {
        options.check()?;
        *count += 1;
        if *count > options.max_nodes || depth > options.max_depth {
            return Err(diagnostic(
                "work_limit",
                "Expression depth or node budget exhausted",
            ));
        }
        match predicate {
            RowPredicate::All { predicates } | RowPredicate::Any { predicates } => {
                if predicates.len() > options.max_nodes {
                    return Err(diagnostic("work_limit", "Predicate budget exhausted"));
                }
                stack.extend(predicates.iter().map(|p| (p, depth + 1)));
            }
            RowPredicate::Not { predicate } => stack.push((predicate, depth + 1)),
            _ => {}
        }
    }
    Ok(())
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("byte budget exhausted"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded_json(value: &impl Serialize, limit: usize) -> Result<String, serde_json::Error> {
    let mut writer = LimitedWriter {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value)?;
    Ok(String::from_utf8(writer.bytes).expect("JSON is UTF-8"))
}

/// Structured semantic entry point (rows, aggregates and supported relationships).
pub async fn compile_semantic(
    engine: &Engine,
    query: semantic_plan::typed::SemanticQuery,
    options: CompileOptions,
) -> TypedCompilation {
    compile_rows(engine, query, options).await
}

fn artifact_digest(query: &CompiledQuery) -> String {
    semantic_catalog::canonical_digest(
        &serde_json::json!({"pipeline": PIPELINE_REVISION, "artifact": query}),
    )
}

enum RequirementScope<'a> {
    Row,
    GraphLeaf(&'a str),
}

fn safe_requirement_id(value: &impl Serialize) -> String {
    format!(
        "requirement/{}",
        semantic_catalog::canonical_digest(
            &serde_json::to_value(value).expect("requirement identity serializes")
        )
    )
}

fn record_bound(bound: &BoundQuery, record: &mut CompilationRecord, scope: RequirementScope<'_>) {
    record.execution_obligations.clear();
    record.bound_digest = Some(semantic_catalog::canonical_digest(
        &serde_json::to_value(bound).expect("bound query serializes"),
    ));
    record.definition_refs = vec![bound.input.clone()];
    record
        .definition_refs
        .extend(bound.definitions.iter().cloned());
    for requirement in &bound.requirements {
        if let bind::BoundOperation::Related { relationship } = &requirement.operation {
            record.definition_refs.push(relationship.right.clone());
        } else if let bind::BoundOperation::Lookup { lookup, .. } = &requirement.operation {
            record
                .definition_refs
                .push(lookup.relationship.right.clone());
            record.execution_obligations.push(ExecutionObligation {
                rule: lookup.obligation,
                relationship: lookup.relationship.definition.clone(),
                relation: lookup.relationship.right.clone(),
                status: "pending_each_execution",
            });
        } else if let bind::BoundOperation::PathLookup { lookups, .. } = &requirement.operation {
            for lookup in lookups {
                record
                    .definition_refs
                    .push(lookup.relationship.right.clone());
                record.execution_obligations.push(ExecutionObligation {
                    rule: lookup.obligation,
                    relationship: lookup.relationship.definition.clone(),
                    relation: lookup.relationship.right.clone(),
                    status: "pending_each_execution",
                });
            }
        } else if let bind::BoundOperation::Allocate { allocation, .. } = &requirement.operation {
            record.definition_refs.push(allocation.bridge.clone());
            record.execution_obligations.push(ExecutionObligation {
                rule: allocation.obligation,
                relationship: allocation.definition.clone(),
                relation: allocation.bridge.clone(),
                status: "pending_each_execution",
            });
        } else if let bind::BoundOperation::CurrencyConvert { rate, .. } = &requirement.operation {
            record.definition_refs.push(rate.rate_relation.clone());
            record.execution_obligations.push(ExecutionObligation {
                rule: rate.obligation,
                relationship: rate.definition.clone(),
                relation: rate.rate_relation.clone(),
                status: "pending_each_execution",
            });
        } else if let bind::BoundOperation::BusinessCalendar { calendar, .. } =
            &requirement.operation
        {
            record
                .definition_refs
                .push(calendar.calendar_relation.clone());
            record.execution_obligations.push(ExecutionObligation {
                rule: calendar.obligation,
                relationship: calendar.definition.clone(),
                relation: calendar.calendar_relation.clone(),
                status: "pending_each_execution",
            });
        }
    }
    record
        .definition_refs
        .sort_by(|a, b| (&a.id, &a.revision).cmp(&(&b.id, &b.revision)));
    record.definition_refs.dedup();
    record.requirement_dispositions = bound
        .requirements
        .iter()
        .map(|requirement| RequirementDisposition {
            requirement_id: safe_requirement_id(&match scope {
                RequirementScope::Row => serde_json::json!({
                    "kind": "row",
                    "requirement": requirement.id,
                }),
                RequirementScope::GraphLeaf(node) => serde_json::json!({
                    "kind": "graph_leaf",
                    "node": node,
                    "requirement": requirement.id,
                }),
            }),
            result: "bound",
            rule: match &requirement.operation {
                bind::BoundOperation::Lookup { .. } => "lookup.same_query_uniqueness_obligation.v1",
                bind::BoundOperation::PathLookup { lookups, .. }
                    if lookups.iter().any(|lookup| lookup.as_of.is_some()) =>
                {
                    "path_lookup.as_of_half_open_same_query_unique.v1"
                }
                bind::BoundOperation::PathLookup { .. } => {
                    "path_lookup.two_hop_same_query_unique.v1"
                }
                bind::BoundOperation::Allocate { .. } => {
                    "allocation.same_query_population_conservation.v1"
                }
                bind::BoundOperation::CurrencyConvert { .. } => {
                    "currency_rate.same_query_exactly_one.v1"
                }
                bind::BoundOperation::BusinessCalendar { .. } => {
                    "business_calendar.same_query_exactly_one.v1"
                }
                bind::BoundOperation::Convert { .. } => "conversion.authored_exact_rational.v1",
                bind::BoundOperation::CalendarFilter { .. } => "calendar.half_open_local_period.v1",
                bind::BoundOperation::CalendarGroup { .. } => "calendar.observed_utc_month.v1",
                bind::BoundOperation::CalendarFill { .. } => {
                    "calendar.explicit_utc_month_count_zero.v1"
                }
                bind::BoundOperation::Window { .. } => "window.explicit_peer_frame.v1",
                bind::BoundOperation::Ratio { .. } => "ratio.integer_decimal18_truncate.v1",
                bind::BoundOperation::Project { .. } => "project.exact_field.v1",
                bind::BoundOperation::Filter { .. } => "filter.sql_null_logic.v1",
                bind::BoundOperation::FilterOutput { .. } => "filter.explicit_output_stage.v1",
                bind::BoundOperation::Group { .. } => "group.explicit_dimension.v1",
                bind::BoundOperation::Aggregate { .. } => "aggregate.pinned_function.v1",
                bind::BoundOperation::Related { .. } => {
                    "relationship.semi_anti_preserve_multiplicity.v1"
                }
                bind::BoundOperation::Order { .. } => "sort.explicit_null_order.v1",
                bind::BoundOperation::Limit { .. } => "fetch.after_sort.v1",
                bind::BoundOperation::Page { .. } => "page.explicit_offset_fetch.v1",
            },
        })
        .collect();
}

fn allowed(name: &str, options: &CompileOptions) -> bool {
    options
        .allowed_relations
        .as_ref()
        .is_none_or(|scope| scope.contains(name))
}
