//! Opt-in typed row compiler. SQL compatibility compilation stays separate.
//! Records are compiler-owned and available without a tracing subscriber/exporter.
mod bind;
mod cache;
pub use cache::{CompilationCacheOptions, CompilationCacheStats, CompilationSession};
mod capture;
mod context;
mod intent;
pub use capture::{
    CaptureLimits, CaptureRecorder, ModelTranscript, RecordingProvider, TranscriptProvider,
};
mod literal;
mod metrics;
pub use intent::compile_intent;
pub use metrics::{CompilerMetrics, CompilerMetricsSnapshot, LATENCY_BUCKETS_US};
mod lower;
mod replay;
mod temporal;
pub use replay::{PIPELINE_REVISION, ReplayBundle};
pub use temporal::{Calendar, ContextOrigin, RequestContext, TemporalResolution};
mod graph;
pub use graph::{CompiledGraph, GraphReplayBundle, compile_graph};

use std::{
    io::Write,
    sync::Arc,
    time::{Duration, Instant},
};

use datafusion::dataframe::DataFrame;
use semantic_catalog::CatalogSnapshot;
use semantic_engine::{Engine, QueryExecution, QueryOptions};
use semantic_plan::typed::{ContextRequest, RowOperation, RowPredicate, RowQuery, TypedProposal};
use serde::Serialize;
use tokio::sync::watch;
use tracing::Instrument;

use crate::{
    Compiler,
    provider::{Message, ModelProvider, Role},
};
pub use bind::BoundQuery;
pub use context::{ContextManifest, SelectionMode};
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
    async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|value| *value).await;
    }
}

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub metrics: Option<Arc<CompilerMetrics>>,
    pub request_evidence: Option<semantic_plan::typed::RequestEvidence>,
    pub request_context: Option<RequestContext>,
    pub timeout: Duration,
    pub cancellation: Cancellation,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_input_bytes: usize,
    pub max_context_bytes: usize,
    pub max_context_fields: usize,
    pub max_model_output_bytes: usize,
    pub max_sql_bytes: usize,
    pub selection_mode: SelectionMode,
    pub allowed_relations: Option<std::collections::BTreeSet<String>>,
    pub max_context_relations: usize,
    pub max_context_edges: usize,
    pub initial_fields_per_relation: usize,
    pub small_relation_fields: usize,
    pub max_index_objects: usize,
    pub max_index_bytes: usize,
    pub max_search_postings: usize,
    pub max_search_candidates: usize,
    pub max_search_terms: usize,
    pub max_expansions: usize,
    pub max_context_requests: usize,
    pub max_model_calls: usize,
    pub max_total_model_input_bytes: usize,
    pub max_total_model_output_bytes: usize,
    deadline: Option<Instant>,
}
impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            request_context: None,
            request_evidence: None,
            metrics: None,
            timeout: Duration::from_secs(30),
            cancellation: Cancellation::default(),
            max_nodes: 512,
            max_depth: 32,
            max_input_bytes: 64 * 1024,
            max_context_bytes: 256 * 1024,
            max_context_fields: 10_000,
            max_model_output_bytes: 64 * 1024,
            max_sql_bytes: 128 * 1024,
            selection_mode: SelectionMode::Full,
            allowed_relations: None,
            max_context_relations: 128,
            max_context_edges: 1024,
            initial_fields_per_relation: 4,
            small_relation_fields: 32,
            max_index_objects: 2_000_000,
            max_index_bytes: 64 * 1024 * 1024,
            max_search_postings: 20_000,
            max_search_candidates: 64,
            max_search_terms: 64,
            max_expansions: 2,
            max_context_requests: 8,
            max_model_calls: 8,
            max_total_model_input_bytes: 2 * 1024 * 1024,
            max_total_model_output_bytes: 256 * 1024,
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
    fn check(&self) -> Result<(), CompileDiagnostic> {
        if self.cancellation.is_cancelled() {
            return Err(diagnostic("cancelled", "Compilation was cancelled"));
        }
        if self.deadline.is_some_and(|t| Instant::now() >= t) {
            return Err(diagnostic("deadline", "Compilation deadline exhausted"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct CompileDiagnostic {
    pub code: String,
    pub message: String,
}
fn diagnostic(code: &str, message: &str) -> CompileDiagnostic {
    CompileDiagnostic {
        code: code.into(),
        message: message.into(),
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Work {
    pub mapped_value_bytes: usize,
    pub relations_looked_up: usize,
    pub fields_looked_up: usize,
    pub nodes_visited: usize,
    pub context_relations: usize,
    pub context_fields: usize,
    pub context_bytes: usize,
    pub model_calls: usize,
    /// Sum over every attempt, including repeated context and repair history.
    pub model_input_bytes: usize,
    pub model_output_bytes: usize,
    pub index_objects_visited: usize,
    pub index_bytes_visited: usize,
    pub search_postings_visited: usize,
    pub context_expansions: usize,
}
#[derive(Debug, Serialize)]
pub struct StageRecord {
    pub stage: &'static str,
    pub code: String,
    pub elapsed_micros: u128,
}
#[derive(Debug, Serialize)]
pub struct CompilationRecord {
    pub compilation_id: String,
    pub compiler_build: &'static str,
    pub pipeline_revision: &'static str,
    pub prompt_digest: Option<String>,
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
    pub contexts: Vec<ContextManifest>,
    pub model_attempts: Vec<ModelAttempt>,
    pub token_accounting: TokenAccounting,
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
#[derive(Debug, Serialize)]
pub struct ModelAttempt {
    pub attempt: usize,
    pub status: Option<crate::provider::CompletionStatus>,
    pub metadata: crate::provider::CompletionMetadata,
    pub failure_code: Option<&'static str>,
    pub input_bytes: usize,
    pub elapsed_micros: Option<u128>,
}
#[derive(Debug, Default, Serialize)]
pub struct TokenAccounting {
    pub reported_input_tokens: u128,
    pub reported_output_tokens: u128,
    pub reported_cached_input_tokens: u128,
    pub reported_reasoning_tokens: u128,
    pub calls_missing_input_usage: usize,
    pub calls_missing_output_usage: usize,
}
impl CompilationRecord {
    fn new(mode: &'static str) -> Self {
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
            prompt_digest: None,
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
            contexts: Vec::new(),
            model_attempts: Vec::new(),
            token_accounting: TokenAccounting::default(),
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

#[derive(Debug, Serialize)]
pub struct TypedCompilation {
    pub version: u32,
    pub outcome: TypedOutcome,
    pub record: CompilationRecord,
}
#[derive(Debug, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct CompiledQuery {
    request_evidence: Option<semantic_plan::typed::RequestEvidence>,
    request_context: Option<RequestContext>,
    intent: RowQuery,
    bound: BoundQuery,
    relational: RelationalPlan,
    sql: SqlArtifact,
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
    fn check_snapshot(&self, engine: &Engine) -> Result<(), CompileDiagnostic> {
        if engine.catalog().snapshot().id() != self.bound.snapshot_id() {
            return Err(diagnostic(
                "snapshot_mismatch",
                "Recompile against the current catalog before planning or executing",
            ));
        }
        Ok(())
    }
    /// Rebuild through deterministic DataFusion expressions, without reading rows.
    pub async fn plan_direct(&self, engine: &Engine) -> Result<DataFrame, CompileDiagnostic> {
        self.check_snapshot(engine)?;
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
        engine
            .execute_parameters(self.sql.statement(), self.sql.values(), options)
            .await
            .map_err(lower::backend_error)
    }
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
    let start = Instant::now();
    let mut record = CompilationRecord::new("structured_rows_v1");
    let result = {
        let work = async {
            options.check()?;
            let snapshot = engine.catalog().snapshot();
            record.snapshot_id = Some(snapshot.id().into());
            compile_bound(engine, &snapshot, query, &options, &mut record)
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
    finish(result, record, start, options.metrics.as_deref())
}

fn finish(
    result: Result<TypedOutcome, CompileDiagnostic>,
    mut record: CompilationRecord,
    start: Instant,
    metrics: Option<&CompilerMetrics>,
) -> TypedCompilation {
    for attempt in &record.model_attempts {
        let usage = &attempt.metadata.usage;
        if let Some(tokens) = usage.input_tokens {
            record.token_accounting.reported_input_tokens += u128::from(tokens);
        } else {
            record.token_accounting.calls_missing_input_usage += 1;
        }
        if let Some(tokens) = usage.output_tokens {
            record.token_accounting.reported_output_tokens += u128::from(tokens);
        } else {
            record.token_accounting.calls_missing_output_usage += 1;
        }
        record.token_accounting.reported_cached_input_tokens +=
            u128::from(usage.cached_input_tokens.unwrap_or(0));
        record.token_accounting.reported_reasoning_tokens +=
            u128::from(usage.reasoning_tokens.unwrap_or(0));
    }
    record.stage("complete", start, &result);
    record.elapsed_micros = start.elapsed().as_micros();
    let outcome = result.unwrap_or_else(|error| {
        if error.code.ends_with("limit")
            || matches!(
                error.code.as_str(),
                "deadline"
                    | "cancelled"
                    | "unresolved_terms"
                    | "unresolved_value"
                    | "unresolved_time_context"
                    | "unresolved_alternatives"
            )
        {
            TypedOutcome::Unresolved { diagnostic: error }
        } else if error.code == "provider_failure" {
            TypedOutcome::ProviderFailure { diagnostic: error }
        } else {
            TypedOutcome::Rejected { diagnostic: error }
        }
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
        model_calls = record.work.model_calls,
        "compilation completed"
    );
    if let Some(metrics) = metrics {
        metrics.observe(&record);
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
    let result = preflight(&query, options)
        .and_then(|_| bind::bind(snapshot, &query, options, &mut record.work));
    record.stage("bind", start, &result);
    let bound = result?;
    if let Some(evidence) = &options.request_evidence {
        record.request_digest = Some(semantic_catalog::canonical_digest(&serde_json::json!(
            evidence.original_request
        )));
        record.request_spans_validated = true;
    }
    record_bound(&bound, record);
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
    let artifact = CompiledQuery {
        request_evidence: options.request_evidence.clone(),
        request_context: options.request_context.clone(),
        intent: query,
        bound,
        relational,
        sql,
    };
    options.check()?;
    record.artifact_digest = Some(artifact_digest(&artifact));
    options.check()?;
    Ok(artifact)
}

fn preflight(query: &RowQuery, options: &CompileOptions) -> Result<(), CompileDiagnostic> {
    options.check()?;
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

impl<P: ModelProvider> Compiler<P> {
    /// Opt-in model interpretation into row IR. Never falls back to model SQL.
    /// Retrieved context is opt-in. Missing bindings require authoritative
    /// hydration and model reconsideration; budgets never imply catalog absence.
    #[tracing::instrument(
        name = "semantic.compile",
        skip_all,
        fields(mode = ?options.selection_mode)
    )]
    pub async fn compile_typed(
        &self,
        engine: &Engine,
        request: &str,
        options: CompileOptions,
    ) -> TypedCompilation {
        let options = options.start();
        let start = Instant::now();
        let mut record = CompilationRecord::new(match options.selection_mode {
            SelectionMode::Full => "model_rows_v1_full_context",
            SelectionMode::Retrieved => "model_rows_v1_retrieved",
            SelectionMode::Auto => "model_rows_v1_auto",
        });
        let result = {
            let work = self.interpret_rows(engine, request, &options, &mut record);
            tokio::select! {
                biased;
                _ = options.cancellation.cancelled() => Err(diagnostic("cancelled", "Compilation was cancelled")),
                result = tokio::time::timeout(options.timeout, work) => result.unwrap_or_else(|_| Err(diagnostic("deadline", "Compilation deadline exhausted"))),
            }
        };
        finish(result, record, start, options.metrics.as_deref())
    }

    async fn interpret_rows(
        &self,
        engine: &Engine,
        request: &str,
        options: &CompileOptions,
        record: &mut CompilationRecord,
    ) -> Result<TypedOutcome, CompileDiagnostic> {
        options.check()?;
        if let Some(context) = &options.request_context {
            context.validate()?;
        }
        if options
            .request_evidence
            .as_ref()
            .is_some_and(|evidence| evidence.original_request != request)
        {
            return Err(diagnostic(
                "request_evidence",
                "Host evidence must retain the exact original request",
            ));
        }
        if request.trim().is_empty() {
            return Err(diagnostic(
                "empty_request",
                "A nonempty request is required",
            ));
        }
        if request.len() > options.max_input_bytes {
            return Err(diagnostic("input_limit", "Request byte budget exhausted"));
        }
        let snapshot = engine.catalog().snapshot();
        record.snapshot_id = Some(snapshot.id().into());
        if snapshot.is_empty() {
            return Ok(TypedOutcome::Unsupported {
                reason: "No relations are registered".into(),
            });
        }
        let start = Instant::now();
        let initial = context::ContextState::initial(&snapshot, request, options, &mut record.work)
            .and_then(|state| {
                state
                    .render(&snapshot, request, options, &mut record.work)
                    .map(|bundle| (state, bundle))
            });
        record.stage("context", start, &initial);
        let (mut state, (payload, manifest)) = initial?;
        record.contexts.push(manifest);
        record.prompt_digest = Some(semantic_catalog::canonical_digest(&serde_json::json!(
            include_str!("prompt.txt")
        )));
        let mut messages = vec![
            Message {
                role: Role::System,
                content: include_str!("prompt.txt").into(),
            },
            Message {
                role: Role::User,
                content: payload,
            },
        ];
        let mut repairs = 0;
        loop {
            options.check()?;
            if record.work.model_calls >= options.max_model_calls {
                return Err(diagnostic(
                    "model_call_limit",
                    "Total model call budget exhausted",
                ));
            }
            let input_bytes = messages.iter().map(|m| m.content.len()).sum::<usize>();
            if input_bytes
                > options
                    .max_total_model_input_bytes
                    .saturating_sub(record.work.model_input_bytes)
            {
                return Err(diagnostic(
                    "model_input_limit",
                    "Total model input byte budget exhausted",
                ));
            }
            record.work.model_calls += 1;
            record.work.model_input_bytes += input_bytes;
            let start = Instant::now();
            record.model_attempts.push(ModelAttempt {
                attempt: record.work.model_calls,
                status: None,
                metadata: crate::provider::CompletionMetadata::default(),
                failure_code: Some("interrupted"),
                input_bytes,
                elapsed_micros: None,
            });
            let response = self
                .provider
                .complete_envelope(&messages)
                .instrument(tracing::debug_span!(
                    "semantic.interpret",
                    attempt = record.work.model_calls
                ))
                .await
                .map_err(|_| {
                    diagnostic(
                        "provider_failure",
                        "Model provider failed; no automatic transport retry",
                    )
                });
            record.stage("interpret", start, &response);
            *record
                .model_attempts
                .last_mut()
                .expect("recorded provider attempt") = ModelAttempt {
                attempt: record.work.model_calls,
                status: response.as_ref().ok().map(|r| r.status),
                metadata: response
                    .as_ref()
                    .ok()
                    .map(|r| r.metadata.clone())
                    .unwrap_or_default(),
                failure_code: response.as_ref().err().map(|_| "provider_failure"),
                input_bytes,
                elapsed_micros: Some(start.elapsed().as_micros()),
            };
            let response = response?;
            if response.status != crate::provider::CompletionStatus::Complete {
                record.work.model_output_bytes += response.text.as_ref().map_or(0, String::len);
            }
            let response = response.into_text().map_err(|_| diagnostic("provider_failure", "Model provider returned refusal, truncation, unsupported tool calls or empty output"))?;
            record.work.model_output_bytes += response.len();
            if response.len() > options.max_model_output_bytes
                || record.work.model_output_bytes > options.max_total_model_output_bytes
            {
                return Err(diagnostic(
                    "model_output_limit",
                    "Model output byte budget exhausted",
                ));
            }
            let start = Instant::now();
            let proposal = serde_json::from_str::<TypedProposal>(&response).map_err(|_| {
                diagnostic(
                    "invalid_proposal",
                    "Return exactly one documented typed proposal JSON shape",
                )
            });
            record.stage("decode", start, &proposal);
            let mut attempt_options = options.clone();
            let proposal = proposal.and_then(|proposal| match proposal {
                TypedProposal::Intent { query, evidence } => {
                    if evidence.original_request != request {
                        return Err(diagnostic(
                            "request_evidence",
                            "Intent must retain the exact original request",
                        ));
                    }
                    intent::validate(&query, &evidence, options)?;
                    attempt_options.request_evidence = Some(evidence);
                    Ok(TypedProposal::Query { query })
                }
                other => Ok(other),
            });
            let mut expansion = None;
            let result = match proposal {
                Ok(TypedProposal::NeedContext { requests }) => {
                    expansion = Some(requests);
                    Ok(None)
                }
                Ok(TypedProposal::Graph { query }) => {
                    match graph::preflight_graph(&query, &attempt_options) {
                        Err(error) => Err(error),
                        Ok(()) => {
                            let mut requests = Vec::new();
                            for node in &query.nodes {
                                if let semantic_plan::graph::GraphOperation::Rows { query } =
                                    &node.operation
                                {
                                    let missing = state.missing(query);
                                    if !state.contains_relation(&query.input.relation)
                                        || !missing.is_empty()
                                    {
                                        requests.push(ContextRequest::Hydrate {
                                            relation: query.input.relation.clone(),
                                            fields: missing.into_iter().collect(),
                                        });
                                    }
                                }
                                if let semantic_plan::graph::GraphOperation::Compose {
                                    relationship_relation,
                                    ..
                                } = &node.operation
                                    && !relationship_relation.is_empty()
                                    && !state.contains_relation(relationship_relation)
                                {
                                    requests.push(ContextRequest::Hydrate {
                                        relation: relationship_relation.clone(),
                                        fields: vec![],
                                    });
                                }
                            }
                            if requests.is_empty() {
                                graph::build(engine, query, &attempt_options, record)
                                    .await
                                    .map(|query| {
                                        Some(TypedOutcome::CompiledGraph {
                                            query: Box::new(query),
                                        })
                                    })
                            } else {
                                expansion = Some(requests);
                                Ok(None)
                            }
                        }
                    }
                }
                Ok(TypedProposal::Query { query }) => match preflight(&query, &attempt_options) {
                    Err(error) => Err(error),
                    Ok(()) if !query.unresolved.is_empty() => Err(diagnostic(
                        "unresolved_terms",
                        "Resolve all business choices before binding",
                    )),
                    Ok(()) => {
                        let missing = state.missing(&query);
                        if !state.contains_relation(&query.input.relation) || !missing.is_empty() {
                            expansion = Some(vec![ContextRequest::Hydrate {
                                relation: query.input.relation.clone(),
                                fields: missing.into_iter().collect(),
                            }]);
                            Ok(None)
                        } else {
                            compile_bound(engine, &snapshot, query, &attempt_options, record)
                                .await
                                .map(|query| {
                                    Some(TypedOutcome::Compiled {
                                        query: Box::new(query),
                                    })
                                })
                        }
                    }
                },
                Ok(TypedProposal::NeedsClarification { phrases, question })
                    if !phrases.is_empty()
                        && phrases.iter().all(|p| !p.trim().is_empty())
                        && !question.trim().is_empty() =>
                {
                    Ok(Some(TypedOutcome::NeedsClarification { phrases, question }))
                }
                Ok(TypedProposal::Unsupported { reason }) if !reason.trim().is_empty() => {
                    if record
                        .contexts
                        .last()
                        .is_some_and(|m| m.selection_mode == SelectionMode::Retrieved)
                    {
                        Ok(Some(TypedOutcome::Unresolved {
                            diagnostic: diagnostic(
                                "partial_catalog",
                                "A retrieved subset cannot establish global catalog unavailability",
                            ),
                        }))
                    } else {
                        Ok(Some(TypedOutcome::Unsupported { reason }))
                    }
                }
                Ok(TypedProposal::Unresolved { reason }) if !reason.trim().is_empty() => {
                    Ok(Some(TypedOutcome::Unresolved {
                        diagnostic: diagnostic("interpretation_unresolved", &reason),
                    }))
                }
                Ok(_) => Err(diagnostic(
                    "invalid_proposal",
                    "Outcome requires nonempty details",
                )),
                Err(error) => Err(error),
            };
            let result = if let Some(requests) = expansion {
                let start = Instant::now();
                let expanded =
                    state.expand(&requests, &snapshot, request, options, &mut record.work);
                record.stage("expand_context", start, &expanded);
                expanded.map(|(payload, manifest)| {
                    messages[1].content = payload;
                    record.contexts.push(manifest);
                    messages.push(Message { role: Role::Assistant, content: response.clone() });
                    messages.push(Message { role: Role::User, content: "Authoritative catalog context has expanded. Reconsider the entire original request, including governing definitions and alternatives, before proposing a query.".into() });
                    None
                })
            } else {
                result
            };
            match result {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => continue,
                Err(error)
                    if repairs < self.max_repairs
                        && !error.code.ends_with("limit")
                        && !matches!(
                            error.code.as_str(),
                            "cancelled" | "deadline" | "unresolved_terms" | "access_scope"
                        ) =>
                {
                    repairs += 1;
                    messages.push(Message {
                        role: Role::Assistant,
                        content: response,
                    });
                    messages.push(Message { role: Role::User, content: serde_json::json!({"validation_error": error, "instruction": "Repair the proposal while preserving every original requirement. Never substitute SQL."}).to_string() });
                }
                Err(error) => return Err(error),
            }
        }
    }
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

fn record_bound(bound: &BoundQuery, record: &mut CompilationRecord) {
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
            requirement_id: requirement.id.clone(),
            result: "bound",
            rule: match &requirement.operation {
                bind::BoundOperation::Lookup { .. } => "lookup.same_query_uniqueness_obligation.v1",
                bind::BoundOperation::CalendarFilter { .. } => "calendar.half_open_local_period.v1",
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
            },
        })
        .collect();
}
