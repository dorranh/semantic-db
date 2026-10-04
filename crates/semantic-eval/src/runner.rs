use crate::{
    Case, Context, Dataset, Expected, Result, TypedResult, compare, lifecycle::Lifecycle,
    result_from_batches,
};
use semantic_compiler::typed::TypedOutcome;
use semantic_engine::Engine;
use semantic_interpreter::{
    Interpreter,
    provider::{ModelProvider, OpenAiConfig, OpenAiProvider},
    typed::{InterpretOptions, SelectionMode},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, clap::ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Interface contract for artifact format version 1.
pub enum Interface {
    Sql,
    Ask,
    Both,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
/// ContextMode contract for artifact format version 1.
pub enum ContextMode {
    Auto,
    Full,
    Retrieved,
}
#[derive(Clone, Debug)]
/// Selection, budgets, model configuration, and lifecycle policy for one evaluation.
pub struct RunOptions {
    pub interface: Interface,
    pub context: ContextMode,
    pub cases: Vec<String>,
    pub repetitions: usize,
    pub release: bool,
    pub artifacts: PathBuf,
    pub attach: bool,
    pub keep_environment: bool,
    pub timeout_seconds: u64,
    /// Minimum start-to-start delay between harness model calls; zero disables pacing.
    pub model_request_interval_millis: u64,
    pub max_rows: usize,
    pub max_bytes: usize,
    pub model: Option<String>,
    pub env_file: Option<PathBuf>,
    pub cli_binary: Option<PathBuf>,
    pub public_interfaces: bool,
}
impl Default for RunOptions {
    fn default() -> Self {
        Self {
            interface: Interface::Both,
            context: ContextMode::Auto,
            cases: vec![],
            repetitions: 1,
            release: false,
            artifacts: ".semantic-eval".into(),
            attach: false,
            keep_environment: false,
            timeout_seconds: 60,
            model_request_interval_millis: 0,
            max_rows: 100_000,
            max_bytes: 32 * 1024 * 1024,
            model: None,
            env_file: None,
            cli_binary: None,
            public_interfaces: false,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
/// One independent interface attempt against a typed gold expectation.
pub struct CaseReport {
    pub id: String,
    pub interface: Interface,
    pub repetition: usize,
    pub passed: bool,
    pub incomplete: bool,
    pub outcome: String,
    pub diagnostic: Option<String>,
    /// Sanitized provider failures captured before interpreter classification.
    #[serde(default)]
    pub provider_errors: Vec<String>,
    pub differences: Vec<String>,
    pub actual: Option<TypedResult>,
    pub compilation: Option<serde_json::Value>,
    pub latency_millis: u128,
}
#[derive(Debug, Serialize, Deserialize)]
/// Complete run evidence; success requires all requested checks to pass.
pub struct RunReport {
    pub format_version: u32,
    pub dataset_id: String,
    pub dataset_version: String,
    pub artifact_digest: String,
    pub complete: bool,
    pub full_coverage: bool,
    pub release: bool,
    pub interface: Interface,
    pub context: ContextMode,
    pub configured_model: Option<String>,
    /// Harness pacing configuration; public subprocess model calls are unaffected.
    #[serde(default)]
    pub model_request_interval_millis: u64,
    pub report_path: PathBuf,
    pub attached: bool,
    pub compose_project: String,
    pub cleanup_command: Vec<String>,
    pub setup_error: Option<String>,
    pub cleanup_error: Option<String>,
    pub artifact_error: Option<String>,
    pub cases: Vec<CaseReport>,
    pub limitations: Vec<String>,
    pub public_checks: Vec<crate::PublicCheck>,
}
impl RunReport {
    pub fn success(&self) -> bool {
        self.complete
            && self.setup_error.is_none()
            && self.cleanup_error.is_none()
            && self.artifact_error.is_none()
            && self.cases.iter().all(|c| c.passed)
            && self.public_checks.iter().all(|c| c.passed)
    }
    pub fn write(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
pub fn select_cases<'a>(d: &'a Dataset, ids: &[String]) -> Result<Vec<&'a Case>> {
    let selection: BTreeSet<_> = ids.iter().collect();
    if selection.len() != ids.len() {
        return Err("repeated --case selection is invalid".into());
    }
    for id in ids {
        if !d.cases.iter().any(|c| &c.id == id) {
            return Err(format!("unknown case {id}").into());
        }
    }
    Ok(d.cases
        .iter()
        .filter(|c| ids.is_empty() || selection.contains(&c.id))
        .collect())
}
struct Actual {
    outcome: String,
    result: Option<TypedResult>,
    diagnostic: Option<String>,
    compilation: Option<serde_json::Value>,
    incomplete: bool,
}
async fn collect(
    mut execution: semantic_engine::QueryExecution,
    options: &RunOptions,
) -> Result<TypedResult> {
    use futures::StreamExt;
    let schema = execution.stream.schema();
    let mut batches = vec![];
    let mut rows = 0usize;
    let mut bytes = 0usize;
    while let Some(batch) = execution.stream.next().await {
        let batch = batch?;
        rows = rows
            .checked_add(batch.num_rows())
            .ok_or("row budget overflow")?;
        bytes = bytes
            .checked_add(batch.get_array_memory_size())
            .ok_or("byte budget overflow")?;
        if rows > options.max_rows || bytes > options.max_bytes {
            execution.cancel();
            return Err("output budget exceeded; result is incomplete".into());
        }
        batches.push(batch)
    }
    result_from_batches(schema.as_ref(), &batches)
}
async fn sql(engine: &Engine, statement: &str, o: &RunOptions) -> Result<Actual> {
    let result = collect(
        engine
            .execute(statement, engine.query_options().clone())
            .await?,
        o,
    )
    .await?;
    Ok(Actual {
        outcome: "result".into(),
        result: Some(result),
        diagnostic: None,
        compilation: None,
        incomplete: false,
    })
}
async fn ask<P: ModelProvider>(
    engine: &Engine,
    interpreter: &Interpreter<P>,
    c: &Case,
    ctx: &Context,
    o: &RunOptions,
) -> Result<Actual> {
    let mut options = InterpretOptions::default();
    options.selection_mode = match o.context {
        ContextMode::Auto => SelectionMode::Auto,
        ContextMode::Full => SelectionMode::Full,
        ContextMode::Retrieved => SelectionMode::Retrieved,
    };
    options.compiler.timeout = Duration::from_secs(o.timeout_seconds);
    options.compiler.allowed_relations = ctx.allowed_relations.clone();
    options.compiler.request_context = Some(semantic_compiler::typed::RequestContext {
        reference_unix_millis: chrono::DateTime::parse_from_rfc3339(&ctx.reference_time)?
            .timestamp_millis(),
        timezone: ctx.timezone.clone(),
        calendar: semantic_compiler::typed::Calendar::Gregorian,
        origin: semantic_compiler::typed::ContextOrigin::Caller,
    });
    let compiled = interpreter
        .compile_typed(engine, &c.question, options)
        .await;
    // Retain diagnostics/accounting only: model receives the question and imported catalog, never oracle/SQL/ledger.
    let record =
        serde_json::json!({"record":compiled.record,"interpretation":compiled.interpretation});
    let mut actual = Actual {
        outcome: String::new(),
        result: None,
        diagnostic: None,
        compilation: Some(record),
        incomplete: false,
    };
    match compiled.outcome {
        TypedOutcome::Compiled { query } => {
            let result = async {
                let execution = if let Some(scope) = &ctx.allowed_relations {
                    query
                        .execute_authorized(engine, scope, engine.query_options().clone())
                        .await?
                } else {
                    query
                        .execute(engine, engine.query_options().clone())
                        .await?
                };
                collect(execution, o).await
            }
            .await;
            record_execution(&mut actual, result);
        }
        TypedOutcome::CompiledGraph { query } => {
            let result = async {
                let execution = if let Some(scope) = &ctx.allowed_relations {
                    query
                        .execute_authorized(engine, scope, engine.query_options().clone())
                        .await?
                } else {
                    query
                        .execute(engine, engine.query_options().clone())
                        .await?
                };
                collect(execution, o).await
            }
            .await;
            record_execution(&mut actual, result);
        }
        TypedOutcome::NeedsClarification { question, .. } => {
            actual.outcome = "needs_clarification".into();
            actual.diagnostic = Some(question)
        }
        TypedOutcome::Unsupported { reason } => {
            actual.outcome = "unsupported".into();
            actual.diagnostic = Some(reason)
        }
        TypedOutcome::Rejected { diagnostic } => {
            actual.incomplete = matches!(diagnostic.code.as_str(), "deadline" | "cancelled")
                || diagnostic.code.ends_with("limit");
            actual.outcome = "rejected".into();
            actual.diagnostic = Some(diagnostic.to_string())
        }
        TypedOutcome::Unresolved { diagnostic } => {
            actual.incomplete = matches!(diagnostic.code.as_str(), "deadline" | "cancelled")
                || diagnostic.code.ends_with("limit");
            actual.outcome = "unresolved".into();
            actual.diagnostic = Some(diagnostic.to_string())
        }
        TypedOutcome::ProviderFailure { diagnostic } => {
            actual.outcome = "provider_failure".into();
            actual.diagnostic = Some(diagnostic.to_string());
            actual.incomplete = true
        }
    }
    Ok(actual)
}
fn record_execution(actual: &mut Actual, result: Result<TypedResult>) {
    match result {
        Ok(rows) => {
            actual.outcome = "result".into();
            actual.result = Some(rows)
        }
        Err(e) => {
            let diagnostic = e.to_string();
            actual.incomplete = diagnostic.contains("budget")
                || diagnostic.contains("deadline")
                || diagnostic.contains("timeout");
            actual.outcome = "execution_error".into();
            actual.diagnostic = Some(diagnostic)
        }
    }
}

fn assess(e: &Expected, a: &Actual, c: &Case) -> Result<Vec<String>> {
    let (expected, contains) = match e {
        Expected::Result { .. } => ("result", None),
        Expected::NeedsClarification {
            diagnostic_contains,
        } => ("needs_clarification", diagnostic_contains.as_ref()),
        Expected::Unsupported {
            diagnostic_contains,
        } => ("unsupported", diagnostic_contains.as_ref()),
        Expected::Rejected {
            diagnostic_contains,
        } => ("rejected", diagnostic_contains.as_ref()),
        Expected::ExecutionError {
            diagnostic_contains,
        } => ("execution_error", diagnostic_contains.as_ref()),
    };
    if a.outcome != expected {
        return Ok(vec![format!("expected {expected}, actual {}", a.outcome)]);
    }
    if let Some(needle) = contains
        && !a.diagnostic.as_deref().unwrap_or("").contains(needle)
    {
        return Ok(vec![format!("diagnostic lacks {needle:?}")]);
    }
    if let Some(result) = &a.result {
        compare(e, result, &c.comparison)
    } else {
        Ok(vec![])
    }
}
/// Execute against a configured live provider, preserving failure and completeness evidence.
pub async fn run(dataset: &Dataset, options: RunOptions) -> Result<RunReport> {
    run_inner::<OpenAiProvider>(dataset, options, None).await
}
/// Execute with an application-owned model provider. Scripted runs never certify release acceptance.
pub async fn run_with_provider<P: ModelProvider>(
    dataset: &Dataset,
    options: RunOptions,
    provider: P,
) -> Result<RunReport> {
    if options.release {
        return Err("injected provider runs cannot establish live release acceptance".into());
    }
    run_inner(dataset, options, Some(provider)).await
}
type ProviderDiagnostics = std::sync::Arc<std::sync::Mutex<Vec<String>>>;
type SharedModelRequestGate = std::sync::Arc<ModelRequestGate>;
struct ModelRequestGate {
    interval: Duration,
    last_start: tokio::sync::Mutex<Option<Instant>>,
}
impl ModelRequestGate {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_start: tokio::sync::Mutex::new(None),
        }
    }
    async fn enter(&self) {
        if self.interval.is_zero() {
            return;
        }
        // Keep the gate locked while waiting so concurrent calls cannot reserve the same start.
        let mut last_start = self.last_start.lock().await;
        if let Some(previous) = *last_start {
            let remaining = self.interval.saturating_sub(previous.elapsed());
            if !remaining.is_zero() {
                tokio::time::sleep(remaining).await;
            }
        }
        *last_start = Some(Instant::now());
    }
}

enum EvaluationProvider<P> {
    Live(OpenAiProvider, ProviderDiagnostics, SharedModelRequestGate),
    Injected(P, ProviderDiagnostics, SharedModelRequestGate),
}
impl<P> EvaluationProvider<P> {
    fn gate(&self) -> &SharedModelRequestGate {
        match self {
            Self::Live(_, _, gate) | Self::Injected(_, _, gate) => gate,
        }
    }
    fn capture<T>(
        &self,
        result: &std::result::Result<T, semantic_interpreter::provider::ProviderError>,
    ) {
        if let Err(error) = result {
            let diagnostics = match self {
                Self::Live(_, diagnostics, _) | Self::Injected(_, diagnostics, _) => diagnostics,
            };
            diagnostics
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(error.to_string());
        }
    }
}

impl<P: ModelProvider> ModelProvider for EvaluationProvider<P> {
    fn capabilities(&self) -> semantic_interpreter::provider::ProviderCapabilities {
        match self {
            Self::Live(p, _, _) => p.capabilities(),
            Self::Injected(p, _, _) => p.capabilities(),
        }
    }
    async fn complete(
        &self,
        messages: &[semantic_interpreter::provider::Message],
    ) -> std::result::Result<String, semantic_interpreter::provider::ProviderError> {
        self.gate().enter().await;
        let result = match self {
            Self::Live(p, _, _) => p.complete(messages).await,
            Self::Injected(p, _, _) => p.complete(messages).await,
        };
        self.capture(&result);
        result
    }
    async fn complete_envelope(
        &self,
        messages: &[semantic_interpreter::provider::Message],
    ) -> std::result::Result<
        semantic_interpreter::provider::ModelCompletion,
        semantic_interpreter::provider::ProviderError,
    > {
        self.gate().enter().await;
        let result = match self {
            Self::Live(p, _, _) => p.complete_envelope(messages).await,
            Self::Injected(p, _, _) => p.complete_envelope(messages).await,
        };
        self.capture(&result);
        result
    }
}
async fn run_inner<P: ModelProvider>(
    dataset: &Dataset,
    mut options: RunOptions,
    injected: Option<P>,
) -> Result<RunReport> {
    if options.model_request_interval_millis > 60_000 {
        return Err("model request interval must be at most 60000 milliseconds".into());
    }
    if options.repetitions == 0
        || options.repetitions > 100
        || options.timeout_seconds == 0
        || options.timeout_seconds > 86400
        || options.max_rows == 0
        || options.max_bytes == 0
    {
        return Err("run budgets and repetitions must be positive".into());
    }
    let selected = select_cases(dataset, &options.cases)?;
    if options.interface == Interface::Sql
        && (selected.iter().all(|c| c.sql.is_none())
            || (!options.cases.is_empty() && selected.iter().any(|c| c.sql.is_none())))
    {
        return Err(
            "selected SQL cohort contains a case without reference SQL, or has no executable cases"
                .into(),
        );
    }

    if options.release
        && (!options.cases.is_empty()
            || options.attach
            || options.keep_environment
            || options.interface != Interface::Both
            || options.repetitions < 3
            || !matches!(options.context, ContextMode::Auto))
    {
        return Err("release requires full SQL+Ask auto coverage, >=3 repetitions, fresh cleaned environment".into());
    }
    options.artifacts = output_directory(dataset, &options.artifacts)?;
    std::fs::create_dir_all(&options.artifacts)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    options.artifacts = options
        .artifacts
        .join(format!("run-{}-{nonce:x}", std::process::id()));
    std::fs::create_dir(&options.artifacts)?;
    let mut lifecycle = Lifecycle::new(
        dataset,
        &options.artifacts,
        options.keep_environment,
        options.attach,
    )?;
    let mut environment = std::collections::BTreeMap::new();
    if let Some(path) = &options.env_file {
        for item in dotenvy::from_path_iter(path).map_err(|_| "could not read env file")? {
            let (key, value) = item.map_err(|_| "invalid env-file syntax")?;
            environment.insert(key, value);
        }
    }
    let getenv = |name: &str| {
        std::env::var(name)
            .ok()
            .or_else(|| environment.get(name).cloned())
    };
    let model = options
        .model
        .clone()
        .or_else(|| getenv("SEMANTIC_EVAL_MODEL"))
        .or_else(|| getenv("OPENAI_MODEL"))
        .filter(|s| !s.trim().is_empty());
    let mut report = RunReport {
        format_version: 1,
        dataset_id: dataset.manifest.id.clone(),
        dataset_version: dataset.manifest.version.clone(),
        artifact_digest: dataset.digest.clone(),
        complete: true,
        full_coverage: options.cases.is_empty(),
        release: options.release,
        interface: options.interface,
        context: options.context,
        configured_model: model.clone(),
        model_request_interval_millis: options.model_request_interval_millis,
        report_path: options.artifacts.join("report.json"),
        attached: options.attach,
        compose_project: lifecycle.project.clone(),
        cleanup_command: lifecycle.cleanup_command(),
        setup_error: None,
        cleanup_error: None,
        artifact_error: None,
        cases: vec![],
        limitations: vec![],
        public_checks: vec![],
    };
    let setup_future = async {
        dataset.verify_digest()?;
        lifecycle.start(dataset).await?;
        let project =
            semantic_sources::Project::from_path(dataset.resolve(&dataset.manifest.project)?)?;
        let bindings = &lifecycle.bindings;
        let resolver = |name: &str| bindings.get(name).cloned().or_else(|| getenv(name));
        let mut engine = project
            .load(&semantic_sources::Registry::standard(), &resolver)
            .await?
            .engine;
        let mut budgets = engine.query_options().clone();
        budgets.timeout_seconds = options.timeout_seconds;
        budgets.max_decoded_bytes = options.max_bytes;
        engine.set_query_options(budgets)?;
        verify_canonical(dataset, &project, &engine, &options).await?;
        for f in &dataset.manifest.fixtures {
            let e: Expected =
                serde_json::from_slice(&std::fs::read(dataset.resolve(&f.expected)?)?)?;
            let a = sql(&engine, &f.sql, &options).await?;
            let diff = compare(
                &e,
                a.result.as_ref().ok_or("fixture missing result")?,
                &Default::default(),
            )?;
            if !diff.is_empty() {
                return Err(format!("fixture verification failed: {diff:?}").into());
            }
        }
        Ok::<Engine, crate::Error>(engine)
    };
    let mut cancelled = false;
    let setup = tokio::select! { result=async { match tokio::time::timeout(
        Duration::from_secs(
            options.timeout_seconds
                + dataset
                    .manifest
                    .environment
                    .as_ref()
                    .map(|e| e.startup_timeout_seconds + e.bootstrap_timeout_seconds + 30)
                    .unwrap_or(0),
        ),
        setup_future,
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err("dataset setup deadline exceeded".into()),
    } } => result, _=tokio::signal::ctrl_c()=>{cancelled=true;Err("run cancelled during setup".into())} };
    let engine = match setup {
        Ok(e) => Some(e),
        Err(e) => {
            report.setup_error = Some(e.to_string());
            report.complete = false;
            None
        }
    };
    let provider_diagnostics = ProviderDiagnostics::default();
    let model_request_gate = std::sync::Arc::new(ModelRequestGate::new(Duration::from_millis(
        options.model_request_interval_millis,
    )));
    let mut provider_error = None;
    let provider = if let Some(injected) = injected {
        report.limitations.push(
            "Application-owned provider: this run is diagnostic, not live release evidence".into(),
        );
        Some(Interpreter::new(EvaluationProvider::Injected(
            injected,
            provider_diagnostics.clone(),
            model_request_gate.clone(),
        )))
    } else if options.interface != Interface::Sql {
        match (getenv("OPENAI_API_KEY"), model) {
            (Some(key), Some(model)) => {
                let mut config = OpenAiConfig::new(key, model);
                config.timeout = Duration::from_secs(options.timeout_seconds);
                if let Some(base) =
                    getenv("SEMANTIC_EVAL_BASE_URL").or_else(|| getenv("OPENAI_BASE_URL"))
                {
                    config.base_url = base;
                }
                match OpenAiProvider::new(config) {
                    Ok(p) => Some(Interpreter::new(EvaluationProvider::Live(
                        p,
                        provider_diagnostics.clone(),
                        model_request_gate.clone(),
                    ))),
                    Err(e) => {
                        provider_error = Some(e.to_string());
                        None
                    }
                }
            }
            _ => None,
        }
    } else {
        None
    };
    for repetition in 0..options.repetitions {
        let mut ordered = selected.clone();
        shuffle(&mut ordered, repetition as u64 + 0x5eed);
        for c in ordered {
            for interface in [Interface::Sql, Interface::Ask] {
                if options.interface != Interface::Both && options.interface != interface {
                    continue;
                }
                if interface == Interface::Sql && c.sql.is_none() {
                    continue;
                }
                provider_diagnostics
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
                let start = Instant::now();
                let attempt = async {
                    let engine = engine
                        .as_ref()
                        .ok_or("dataset setup failed; see setup_error")?;
                    if interface == Interface::Sql {
                        if c.context
                            .as_ref()
                            .unwrap_or(&dataset.manifest.context)
                            .allowed_relations
                            .is_some()
                        {
                            return Ok(Actual {
                                outcome: "harness_unsupported".into(),
                                result: None,
                                diagnostic: Some(
                                    "reference SQL scope authorization adapter is unavailable"
                                        .into(),
                                ),
                                compilation: None,
                                incomplete: true,
                            });
                        }
                        sql(engine, c.sql.as_deref().ok_or("missing SQL")?, &options).await
                    } else {
                        let provider=provider.as_ref().ok_or_else(||provider_error.clone().unwrap_or_else(||"live model unavailable: OPENAI_API_KEY and explicit model required".into()))?;
                        ask(
                            engine,
                            provider,
                            c,
                            c.context.as_ref().unwrap_or(&dataset.manifest.context),
                            &options,
                        )
                        .await
                    }
                };
                let actual = if cancelled {
                    Actual {
                        outcome: "cancelled".into(),
                        result: None,
                        diagnostic: Some("run cancelled".into()),
                        compilation: None,
                        incomplete: true,
                    }
                } else {
                    tokio::select! { result=async { match tokio::time::timeout(
                        Duration::from_secs(options.timeout_seconds),
                        attempt,
                    )
                    .await
                    {
                        Ok(Ok(a)) => a,
                        Ok(Err(e)) => Actual {
                            outcome: if engine.is_none()
                                || (interface == Interface::Ask && provider.is_none())
                            {
                                "unavailable"
                            } else {
                                "execution_error"
                            }
                            .into(),
                            result: None,
                            diagnostic: Some(e.to_string()),
                            compilation: None,
                            incomplete: engine.is_none()
                                || (interface == Interface::Ask && provider.is_none())
                                || e.to_string().contains("budget"),
                        },
                        Err(_) => Actual {
                            outcome: "timeout".into(),
                            result: None,
                            diagnostic: Some("query deadline exceeded".into()),
                            compilation: None,
                            incomplete: true,
                        },
                    } } =>result, _=tokio::signal::ctrl_c()=>{cancelled=true;Actual{outcome:"cancelled".into(),result:None,diagnostic:Some("run cancelled".into()),compilation:None,incomplete:true}} }
                };
                let differences = assess(&dataset.expectations[&c.id], &actual, c)
                    .unwrap_or_else(|e| vec![format!("comparison failed: {e}")]);
                let passed = differences.is_empty() && !actual.incomplete;
                if actual.incomplete {
                    report.complete = false
                }
                report.cases.push(CaseReport {
                    id: c.id.clone(),
                    interface,
                    repetition,
                    passed,
                    incomplete: actual.incomplete,
                    outcome: actual.outcome,
                    diagnostic: actual.diagnostic,
                    provider_errors: std::mem::take(
                        &mut *provider_diagnostics
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()),
                    ),
                    differences,
                    actual: actual.result,
                    compilation: actual.compilation,
                    latency_millis: start.elapsed().as_millis(),
                });
                report.write(options.artifacts.join("report.json"))?;
            }
        }
    }
    if !cancelled && (options.release || options.public_interfaces) {
        if engine.is_some() {
            let mut child_env = environment.clone();
            for (k, v) in &lifecycle.bindings {
                child_env.insert(k.clone(), v.clone());
            }
            for key in ["OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_MODEL"] {
                if let Some(value) = getenv(key) {
                    child_env.insert(key.into(), value);
                }
            }
            if let Some(model) = &report.configured_model {
                child_env.insert("OPENAI_MODEL".into(), model.clone());
            }
            if let Some(base) = getenv("SEMANTIC_EVAL_BASE_URL") {
                child_env.insert("OPENAI_BASE_URL".into(), base);
            }
            report.public_checks = crate::public::checks(dataset, &options, &child_env).await;
            if report.public_checks.iter().any(|c| !c.passed) {
                report.complete = false;
            }
        } else {
            report.complete = false;
            report
                .limitations
                .push("Public interfaces unavailable because setup failed".into());
        }
    }
    if let Err(e) = lifecycle.finish().await {
        report.cleanup_error = Some(e.to_string());
        report.complete = false
    }
    if let Err(error) = dataset.verify_digest() {
        report.artifact_error = Some(error.to_string());
        report.complete = false;
    }
    report.write(options.artifacts.join("report.json"))?;
    Ok(report)
}
fn shuffle<T>(items: &mut [T], mut state: u64) {
    for i in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        items.swap(i, state as usize % (i + 1));
    }
}

#[derive(Deserialize)]
struct SchemaContract {
    columns: Vec<crate::Column>,
    #[serde(default)]
    primary_key: Vec<String>,
    row_count: usize,
    source: String,
}
async fn verify_canonical(
    dataset: &Dataset,
    project: &semantic_sources::Project,
    engine: &Engine,
    options: &RunOptions,
) -> Result<()> {
    let (Some(schema_path), Some(data_path)) =
        (&dataset.manifest.schemas, &dataset.manifest.canonical_data)
    else {
        return Ok(());
    };
    let schemas: std::collections::BTreeMap<String, SchemaContract> =
        serde_json::from_slice(&std::fs::read(dataset.resolve(schema_path)?)?)?;
    let canonical: std::collections::BTreeMap<
        String,
        Vec<serde_json::Map<String, serde_json::Value>>,
    > = serde_json::from_slice(&std::fs::read(dataset.resolve(data_path)?)?)?;
    let inspection = project.inspect_project(&semantic_sources::Registry::standard())?;
    let mut fixture_evidence = std::collections::BTreeMap::new();
    for (relation, contract) in schemas {
        let dataset_binding = inspection
            .model
            .datasets
            .iter()
            .find(|d| d.name == relation)
            .ok_or("fixture relation lacks an imported model source")?;
        if project.source_connector(&dataset_binding.source) != Some(contract.source.as_str()) {
            return Err(format!(
                "fixture {relation} must use authored connector {}",
                contract.source
            )
            .into());
        }
        if contract.source.trim().is_empty() {
            return Err("schema source must be named".into());
        }
        let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
        let records = canonical
            .get(&relation)
            .ok_or("schema has no canonical relation")?;
        if records.len() != contract.row_count {
            return Err(format!("canonical row count differs for {relation}").into());
        }
        let rows: Vec<Vec<serde_json::Value>> = records
            .iter()
            .map(|record| {
                contract
                    .columns
                    .iter()
                    .map(|column| {
                        let value = record.get(&column.name).ok_or("canonical field missing")?;
                        Ok(if column.kind.starts_with("int") && value.is_number() {
                            serde_json::Value::String(value.to_string())
                        } else {
                            value.clone()
                        })
                    })
                    .collect::<Result<_>>()
            })
            .collect::<Result<_>>()?;
        crate::compare::validate_result(&contract.columns, &rows)?;
        let mut keys = BTreeSet::new();
        for record in records {
            let key: Vec<_> = contract
                .primary_key
                .iter()
                .map(|field| record.get(field).ok_or("primary key field absent"))
                .collect::<std::result::Result<_, _>>()?;
            if !key.is_empty()
                && (key.iter().any(|v| v.is_null()) || !keys.insert(serde_json::to_string(&key)?))
            {
                return Err(format!("canonical key violation for {relation}").into());
            }
        }
        let statement = format!(
            "SELECT {} FROM {}",
            contract
                .columns
                .iter()
                .map(|c| quote(&c.name))
                .collect::<Vec<_>>()
                .join(","),
            quote(&relation)
        );
        let actual = sql(engine, &statement, options)
            .await?
            .result
            .ok_or("fixture missing result")?;
        fixture_evidence.insert(relation.clone(), serde_json::to_value(&actual)?);
        let differences = compare(
            &Expected::Result {
                columns: contract.columns,
                rows,
            },
            &actual,
            &crate::Comparison {
                assert_names: true,
                assert_physical_types: true,
                ordered: false,
            },
        )?;
        if !differences.is_empty() {
            return Err(
                format!("fixture schema/data mismatch for {relation}: {differences:?}").into(),
            );
        }
    }
    std::fs::write(
        options.artifacts.join("fixture-evidence.json"),
        serde_json::to_vec_pretty(&fixture_evidence)?,
    )?;
    Ok(())
}

fn output_directory(dataset: &Dataset, requested: &std::path::Path) -> Result<PathBuf> {
    let absolute = if requested.is_absolute() {
        requested.to_owned()
    } else {
        std::env::current_dir()?.join(requested)
    };
    let mut ancestor = absolute.clone();
    let mut missing = vec![];
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .ok_or("invalid artifact output path")?
                .to_owned(),
        );
        if !ancestor.pop() {
            return Err("invalid artifact output path".into());
        }
    }
    let mut resolved = std::fs::canonicalize(ancestor)?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    if resolved.starts_with(&dataset.root) {
        if requested == std::path::Path::new(".semantic-eval") {
            return Ok(dataset
                .root
                .parent()
                .ok_or("dataset root has no external artifact parent")?
                .join(".semantic-eval"));
        }
        return Err("artifact output must be outside the immutable dataset bundle".into());
    }
    Ok(resolved)
}
