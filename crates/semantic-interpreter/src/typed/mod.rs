//! Model proposal, context selection, bounded repair, and interpretation history.
// Match the compiler API, which returns its structured diagnostic by value.
#![allow(clippy::result_large_err)]

mod capture;
mod context;
mod context_manifest;

pub use capture::{
    CaptureLimits, CaptureRecorder, ModelTranscript, RecordingProvider, TranscriptProvider,
};
pub(crate) use context::ContextCache;
pub use context::{ContextDependency, ContextManifest, ContextObject, SelectionMode};
pub use context_manifest::{
    ContextAudit, ContextFact, ContextGap, ContextGapKind, audit_context_manifest,
};

use crate::{
    Interpreter,
    provider::{CompletionMetadata, CompletionStatus, Message, ModelProvider, Role},
};
use semantic_compiler::typed::RequestContext;
use semantic_compiler::typed::{
    self as compiler, CompilationRecord, CompileDiagnostic, DiagnosticTerminal, PIPELINE_REVISION,
    TypedCompilation, TypedOutcome, diagnostic,
};
use semantic_engine::Engine;
use semantic_plan::typed::{ContextRequest, TypedProposal};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::Write,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
    time::Instant,
};
use tracing::Instrument;

/// Interpretation limits and host context. The embedded compiler options are
/// used for every structured proposal and remain independent of model transport.
#[derive(Debug, Clone)]
pub struct InterpretOptions {
    pub compiler: compiler::CompileOptions,
    pub metrics: Option<Arc<InterpreterMetrics>>,
    pub selection_mode: SelectionMode,
    pub max_context_bytes: usize,
    pub max_context_fields: usize,
    pub max_model_output_bytes: usize,
    pub max_context_relations: usize,
    pub max_context_edges: usize,
    pub initial_fields_per_relation: usize,
    pub small_relation_fields: usize,
    pub max_index_bytes: usize,
    pub max_search_postings: usize,
    pub max_search_candidates: usize,
    pub max_search_terms: usize,
    pub max_expansions: usize,
    pub max_context_requests: usize,
    pub max_model_calls: usize,
    pub max_model_call_bytes: usize,
    pub max_total_model_input_bytes: usize,
    pub max_total_model_output_bytes: usize,
    deadline: Option<Instant>,
}
impl Default for InterpretOptions {
    fn default() -> Self {
        Self {
            compiler: compiler::CompileOptions::default(),
            metrics: None,
            selection_mode: SelectionMode::Full,
            max_context_bytes: 256 * 1024,
            max_context_fields: 10_000,
            max_model_output_bytes: 64 * 1024,
            max_context_relations: 128,
            max_context_edges: 1024,
            initial_fields_per_relation: 4,
            small_relation_fields: 32,
            max_index_bytes: 64 * 1024 * 1024,
            max_search_postings: 20_000,
            max_search_candidates: 64,
            max_search_terms: 64,
            max_expansions: 2,
            max_context_requests: 8,
            max_model_calls: 8,
            max_model_call_bytes: 512 * 1024,
            max_total_model_input_bytes: 2 * 1024 * 1024,
            max_total_model_output_bytes: 256 * 1024,
            deadline: None,
        }
    }
}
impl Deref for InterpretOptions {
    type Target = compiler::CompileOptions;
    fn deref(&self) -> &Self::Target {
        &self.compiler
    }
}
impl DerefMut for InterpretOptions {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.compiler
    }
}
impl InterpretOptions {
    fn start(mut self) -> Self {
        self.deadline = Instant::now().checked_add(self.compiler.timeout);
        self
    }
    pub fn check(&self) -> Result<(), CompileDiagnostic> {
        self.compiler.check()?;
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(diagnostic("deadline", "Interpretation deadline exhausted"));
        }
        Ok(())
    }
    fn compile_options(&self) -> compiler::CompileOptions {
        let mut options = self.compiler.clone();
        if let Some(deadline) = self.deadline {
            options.timeout = deadline.saturating_duration_since(Instant::now());
        }
        options
    }
}
type CompileOptions = InterpretOptions;

#[derive(Debug, Default, Clone, Serialize)]
pub struct InterpreterMetricsSnapshot {
    pub admitted: u128,
    pub in_flight: u128,
    pub peak_in_flight: u128,
    pub completed: u128,
    pub cancelled: u128,
    pub deadline_exceeded: u128,
    pub abandoned: u128,
    pub outcomes: BTreeMap<&'static str, u128>,
    pub model_calls: u128,
    pub model_input_bytes: u128,
    pub model_output_bytes: u128,
    pub reported_input_tokens: u128,
    pub reported_output_tokens: u128,
}
#[derive(Debug, Default)]
pub struct InterpreterMetrics {
    totals: Mutex<InterpreterMetricsSnapshot>,
}
impl InterpreterMetrics {
    pub fn snapshot(&self) -> InterpreterMetricsSnapshot {
        self.totals
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn begin(self: &Arc<Self>) -> InterpretationGuard {
        let mut total = self.totals.lock().unwrap_or_else(|e| e.into_inner());
        total.admitted += 1;
        total.in_flight += 1;
        total.peak_in_flight = total.peak_in_flight.max(total.in_flight);
        InterpretationGuard {
            metrics: self.clone(),
            complete: false,
        }
    }
    fn observe(&self, outcome: &TypedOutcome, record: &InterpretationRecord) {
        let mut total = self.totals.lock().unwrap_or_else(|e| e.into_inner());
        total.completed += 1;
        let (name, code) = match outcome {
            TypedOutcome::Compiled { .. } | TypedOutcome::CompiledGraph { .. } => {
                ("compiled", None)
            }
            TypedOutcome::NeedsClarification { .. } => ("needs_clarification", None),
            TypedOutcome::Unsupported { .. } => ("unsupported", None),
            TypedOutcome::Unresolved { diagnostic } => {
                ("unresolved", Some(diagnostic.code.as_str()))
            }
            TypedOutcome::Rejected { diagnostic } => ("rejected", Some(diagnostic.code.as_str())),
            TypedOutcome::ProviderFailure { diagnostic } => {
                ("provider_failure", Some(diagnostic.code.as_str()))
            }
        };
        *total.outcomes.entry(name).or_default() += 1;
        match code {
            Some("cancelled") => total.cancelled += 1,
            Some("deadline") => total.deadline_exceeded += 1,
            _ => {}
        }
        total.model_calls += record.work.model_calls as u128;
        total.model_input_bytes += record.work.model_input_bytes as u128;
        total.model_output_bytes += record.work.model_output_bytes as u128;
        total.reported_input_tokens += record.token_accounting.reported_input_tokens;
        total.reported_output_tokens += record.token_accounting.reported_output_tokens;
    }
}
struct InterpretationGuard {
    metrics: Arc<InterpreterMetrics>,
    complete: bool,
}
impl Drop for InterpretationGuard {
    fn drop(&mut self) {
        let mut total = self
            .metrics
            .totals
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        total.in_flight = total.in_flight.saturating_sub(1);
        if !self.complete {
            total.abandoned += 1;
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Work {
    pub context_relations: usize,
    pub context_fields: usize,
    pub context_bytes: usize,
    pub index_objects_visited: usize,
    pub index_bytes_visited: usize,
    pub search_postings_visited: usize,
    pub context_expansions: usize,
    pub model_calls: usize,
    pub model_input_bytes: usize,
    pub model_output_bytes: usize,
}
#[derive(Debug, Serialize)]
pub struct ModelAttempt {
    pub attempt: usize,
    pub status: Option<CompletionStatus>,
    pub metadata: CompletionMetadata,
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
#[derive(Debug, Serialize)]
pub struct StageRecord {
    pub stage: &'static str,
    pub code: String,
    pub elapsed_micros: u128,
}
#[derive(Serialize)]
pub struct InterpretationRecord {
    pub mode: &'static str,
    pub prompt_digest: Option<String>,
    pub snapshot_id: Option<String>,
    pub cache_status: &'static str,
    pub contexts: Vec<ContextManifest>,
    pub model_attempts: Vec<ModelAttempt>,
    pub token_accounting: TokenAccounting,
    pub work: Work,
    pub stages: Vec<StageRecord>,
    pub elapsed_micros: u128,
}
impl std::fmt::Debug for InterpretationRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterpretationRecord")
            .field("mode", &self.mode)
            .field("model_calls", &self.work.model_calls)
            .field("context_count", &self.contexts.len())
            .finish_non_exhaustive()
    }
}
impl InterpretationRecord {
    fn new(mode: &'static str) -> Self {
        Self {
            mode,
            prompt_digest: None,
            snapshot_id: None,
            cache_status: "disabled",
            contexts: Vec::new(),
            model_attempts: Vec::new(),
            token_accounting: TokenAccounting::default(),
            work: Work::default(),
            stages: Vec::new(),
            elapsed_micros: 0,
        }
    }
    fn stage<T>(
        &mut self,
        stage: &'static str,
        start: Instant,
        result: &Result<T, CompileDiagnostic>,
    ) {
        self.stages.push(StageRecord {
            stage,
            code: result
                .as_ref()
                .map(|_| "ok")
                .unwrap_or_else(|e| e.code.as_str())
                .into(),
            elapsed_micros: start.elapsed().as_micros(),
        });
    }
    fn finish(&mut self) {
        for attempt in &self.model_attempts {
            let usage = &attempt.metadata.usage;
            if let Some(tokens) = usage.input_tokens {
                self.token_accounting.reported_input_tokens += u128::from(tokens);
            } else {
                self.token_accounting.calls_missing_input_usage += 1;
            }
            if let Some(tokens) = usage.output_tokens {
                self.token_accounting.reported_output_tokens += u128::from(tokens);
            } else {
                self.token_accounting.calls_missing_output_usage += 1;
            }
            self.token_accounting.reported_cached_input_tokens +=
                u128::from(usage.cached_input_tokens.unwrap_or(0));
            self.token_accounting.reported_reasoning_tokens +=
                u128::from(usage.reasoning_tokens.unwrap_or(0));
        }
    }
}
#[derive(Serialize)]
pub struct InterpretedCompilation {
    pub version: u32,
    pub outcome: TypedOutcome,
    pub record: CompilationRecord,
    pub interpretation: InterpretationRecord,
}
impl std::fmt::Debug for InterpretedCompilation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterpretedCompilation")
            .field("outcome", &self.outcome)
            .field("record", &self.record)
            .field("interpretation", &self.interpretation)
            .finish()
    }
}

impl<P: ModelProvider> Interpreter<P> {
    /// Interpret a request, then compile each complete structured proposal with
    /// the independent deterministic compiler. No model-written SQL fallback.
    pub async fn compile_typed(
        &self,
        engine: &Engine,
        request: &str,
        options: InterpretOptions,
    ) -> InterpretedCompilation {
        let options = options.start();
        let mut guard = options.metrics.as_ref().map(InterpreterMetrics::begin);
        let start = Instant::now();
        let mut record = InterpretationRecord::new(match options.selection_mode {
            SelectionMode::Full => "model_rows_v1_full_context",
            SelectionMode::Retrieved => "model_rows_v1_retrieved",
            SelectionMode::Auto => "model_rows_v1_auto",
        });
        let result = tokio::select! {
            biased;
            _ = options.compiler.cancellation.cancelled() => Err(diagnostic("cancelled", "Interpretation was cancelled")),
            result = tokio::time::timeout(options.compiler.timeout, self.interpret_rows(engine, request, &options, &mut record)) =>
                result.unwrap_or_else(|_| Err(diagnostic("deadline", "Interpretation deadline exhausted"))),
        };
        record.elapsed_micros = start.elapsed().as_micros();
        record.finish();
        let mut compilation = result.unwrap_or_else(|diagnostic| {
            let outcome = match diagnostic.details.terminal {
                DiagnosticTerminal::Unresolved => TypedOutcome::Unresolved { diagnostic },
                DiagnosticTerminal::ProviderFailure => TypedOutcome::ProviderFailure { diagnostic },
                DiagnosticTerminal::Rejected => TypedOutcome::Rejected { diagnostic },
            };
            TypedCompilation {
                version: 1,
                outcome,
                record: CompilationRecord::new("model_proposal"),
            }
        });
        if compilation.record.outcome == "pending" {
            compilation.record.outcome = outcome_name(&compilation.outcome);
        }
        if let Some(metrics) = &options.metrics {
            metrics.observe(&compilation.outcome, &record);
        }
        if let Some(guard) = &mut guard {
            guard.complete = true;
        }
        InterpretedCompilation {
            version: compilation.version,
            outcome: compilation.outcome,
            record: compilation.record,
            interpretation: record,
        }
    }

    async fn interpret_rows(
        &self,
        engine: &Engine,
        request: &str,
        options: &InterpretOptions,
        record: &mut InterpretationRecord,
    ) -> Result<TypedCompilation, CompileDiagnostic> {
        options.check()?;
        if let Some(context) = &options.request_context {
            context.validate()?;
        }
        if options
            .request_evidence
            .as_ref()
            .is_some_and(|e| e.original_request != request)
            || options
                .graph_request_evidence
                .as_ref()
                .is_some_and(|e| e.original_request != request)
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
            return Ok(special(TypedOutcome::Unsupported {
                reason: "No relations are registered".into(),
            }));
        }
        let started = Instant::now();
        let initial = self
            .context_cache
            .initial(&snapshot, request, options, &mut record.work)
            .await;
        record.stage("context", started, &initial);
        let (mut state, (payload, manifest), context_reuse) = initial?;
        record.cache_status = match context_reuse {
            context::ContextReuse::SameSnapshot => "context_hit_same_snapshot",
            context::ContextReuse::SelectionRerendered => "selection_hit_re_rendered",
            context::ContextReuse::Miss => "disabled",
        };
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
                    .max_model_call_bytes
                    .saturating_sub(options.max_model_output_bytes)
            {
                return Err(diagnostic(
                    "model_context_limit",
                    "Model-call messages and reserved output exceed the per-call byte envelope",
                ));
            }
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
            let started = Instant::now();
            record.model_attempts.push(ModelAttempt {
                attempt: record.work.model_calls,
                status: None,
                metadata: CompletionMetadata::default(),
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
            record.stage("interpret", started, &response);
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
                elapsed_micros: Some(started.elapsed().as_micros()),
            };
            let response = response?;
            if response.status != CompletionStatus::Complete {
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
            let started = Instant::now();
            let proposal = serde_json::from_str::<TypedProposal>(&response).map_err(|_| {
                diagnostic(
                    "invalid_proposal",
                    "Return exactly one documented typed proposal JSON shape",
                )
            });
            record.stage("decode", started, &proposal);
            let mut compiler_options = options.compile_options();
            let proposal = proposal.and_then(|proposal| match proposal {
                TypedProposal::GraphIntent { query, evidence } => {
                    if evidence.original_request != request {
                        return Err(diagnostic(
                            "request_evidence",
                            "Graph intent must retain the exact original request",
                        ));
                    }
                    if options
                        .graph_request_evidence
                        .as_ref()
                        .is_some_and(|host| host != &evidence)
                    {
                        return Err(diagnostic(
                            "request_evidence",
                            "Model evidence cannot replace the host graph requirement ledger",
                        ));
                    }
                    compiler_options.graph_request_evidence = Some(evidence);
                    Ok(TypedProposal::Graph { query })
                }
                TypedProposal::Intent { query, evidence } => {
                    if evidence.original_request != request {
                        return Err(diagnostic(
                            "request_evidence",
                            "Intent must retain the exact original request",
                        ));
                    }
                    if options
                        .request_evidence
                        .as_ref()
                        .is_some_and(|host| host != &evidence)
                    {
                        return Err(diagnostic(
                            "request_evidence",
                            "Model evidence cannot replace the host requirement ledger",
                        ));
                    }
                    compiler_options.request_evidence = Some(evidence);
                    Ok(TypedProposal::Query { query })
                }
                other => Ok(other),
            });
            let mut expansion = None;
            let result: Result<Option<TypedCompilation>, CompileDiagnostic> = match proposal {
                Ok(TypedProposal::NeedContext { requests }) => {
                    expansion = Some(requests);
                    Ok(None)
                }
                Ok(TypedProposal::Graph { query }) => {
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
                        compiler_result_at_snapshot(
                            engine,
                            snapshot.id(),
                            compiler::compile_graph(engine, query, compiler_options).await,
                        )
                        .map(Some)
                    } else {
                        expansion = Some(requests);
                        Ok(None)
                    }
                }
                Ok(TypedProposal::Query { query }) => {
                    if !query.unresolved.is_empty() {
                        Err(diagnostic(
                            "unresolved_terms",
                            "Resolve all business choices before binding",
                        ))
                    } else {
                        let missing = state.missing(&query);
                        if !state.contains_relation(&query.input.relation) || !missing.is_empty() {
                            expansion = Some(vec![ContextRequest::Hydrate {
                                relation: query.input.relation.clone(),
                                fields: missing.into_iter().collect(),
                            }]);
                            Ok(None)
                        } else {
                            compiler_result_at_snapshot(
                                engine,
                                snapshot.id(),
                                compiler::compile_rows(engine, query, compiler_options).await,
                            )
                            .map(Some)
                        }
                    }
                }
                Ok(TypedProposal::NeedsClarification { phrases, question })
                    if !phrases.is_empty()
                        && phrases.iter().all(|p| !p.trim().is_empty())
                        && !question.trim().is_empty() =>
                {
                    Ok(Some(special(TypedOutcome::NeedsClarification {
                        phrases,
                        question,
                    })))
                }
                Ok(TypedProposal::Unsupported { reason }) if !reason.trim().is_empty() => {
                    if record
                        .contexts
                        .last()
                        .is_some_and(|m| m.selection_mode == SelectionMode::Retrieved)
                    {
                        Ok(Some(special(TypedOutcome::Unresolved {
                            diagnostic: diagnostic(
                                "partial_catalog",
                                "A retrieved subset cannot establish global catalog unavailability",
                            ),
                        })))
                    } else {
                        Ok(Some(special(TypedOutcome::Unsupported { reason })))
                    }
                }
                Ok(TypedProposal::Unresolved { reason }) if !reason.trim().is_empty() => {
                    Ok(Some(special(TypedOutcome::Unresolved {
                        diagnostic: diagnostic("interpretation_unresolved", &reason),
                    })))
                }
                Ok(_) => Err(diagnostic(
                    "invalid_proposal",
                    "Outcome requires nonempty details",
                )),
                Err(error) => Err(error),
            };
            let result = if let Some(requests) = expansion {
                let started = Instant::now();
                let expanded =
                    state.expand(&requests, &snapshot, request, options, &mut record.work);
                record.stage("expand_context", started, &expanded);
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
                Ok(Some(compilation)) => return Ok(compilation),
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
fn outcome_name(outcome: &TypedOutcome) -> &'static str {
    match outcome {
        TypedOutcome::Compiled { .. } | TypedOutcome::CompiledGraph { .. } => "compiled",
        TypedOutcome::NeedsClarification { .. } => "needs_clarification",
        TypedOutcome::Unsupported { .. } => "unsupported",
        TypedOutcome::Unresolved { .. } => "unresolved",
        TypedOutcome::Rejected { .. } => "rejected",
        TypedOutcome::ProviderFailure { .. } => "provider_failure",
    }
}
fn special(outcome: TypedOutcome) -> TypedCompilation {
    TypedCompilation {
        version: 1,
        outcome,
        record: CompilationRecord::new("model_proposal"),
    }
}
fn compiler_result_at_snapshot(
    engine: &Engine,
    snapshot_id: &str,
    compilation: TypedCompilation,
) -> Result<TypedCompilation, CompileDiagnostic> {
    if compilation.record.snapshot_id.as_deref() != Some(snapshot_id)
        || engine.catalog().snapshot().id() != snapshot_id
    {
        return Err(diagnostic(
            "snapshot_mismatch",
            "Catalog changed during interpretation; retry against the current snapshot",
        ));
    }
    match compilation.outcome {
        TypedOutcome::Rejected { ref diagnostic }
        | TypedOutcome::Unresolved { ref diagnostic }
        | TypedOutcome::ProviderFailure { ref diagnostic } => Err(diagnostic.clone()),
        _ => Ok(compilation),
    }
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

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn changed_catalog_or_compiler_snapshot_rejects_a_model_proposal() {
        let engine = Engine::new();
        let current = engine.catalog().snapshot().id().to_owned();
        let mut compiled = special(TypedOutcome::Unsupported {
            reason: "fixture".into(),
        });
        compiled.record.snapshot_id = Some(current.clone());
        let error = compiler_result_at_snapshot(&engine, "earlier-snapshot", compiled)
            .expect_err("catalog drift must reject the proposal");
        assert_eq!(error.code, "snapshot_mismatch");

        let mut compiled = special(TypedOutcome::Unsupported {
            reason: "fixture".into(),
        });
        compiled.record.snapshot_id = Some("different-compiler-snapshot".into());
        let error = compiler_result_at_snapshot(&engine, &current, compiled)
            .expect_err("a compiler rebound to a different snapshot must be rejected");
        assert_eq!(error.code, "snapshot_mismatch");
    }
}
