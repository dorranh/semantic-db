//! Compilation only: successful SQL still executes through the PostgreSQL frontend.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
};
use semantic_db::{
    Engine,
    interpreter::{Compilation, Interpreter, provider::OpenAiProvider},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub struct StateData {
    pub engine: Arc<Engine>,
    pub compiler: Option<Interpreter<OpenAiProvider>>,
}
struct HttpState {
    engine: Arc<Engine>,
    compiler: Option<Interpreter<OpenAiProvider>>,
    session: semantic_db::compiler::typed::CompilationSession<'static>,
    metrics: Arc<semantic_db::compiler::typed::CompilerMetrics>,
    interpreter_metrics: Arc<semantic_db::interpreter::typed::InterpreterMetrics>,
    admission: tokio::sync::Semaphore,
}
type HttpError = (StatusCode, Json<Value>);
impl HttpState {
    fn admit(&self) -> Result<tokio::sync::SemaphorePermit<'_>, HttpError> {
        self.admission.try_acquire().map_err(|_| {
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":"Compiler capacity is currently exhausted"})),
            )
        })
    }
    fn options(&self) -> semantic_db::compiler::typed::CompileOptions {
        let mut options = semantic_db::compiler::typed::CompileOptions::default();
        options.timeout =
            std::time::Duration::from_secs(self.engine.query_options().timeout_seconds);
        options.metrics = Some(self.metrics.clone());
        options
    }
}
pub fn router(state: StateData) -> Router {
    router_with_compilation_limit(state, 4).expect("valid default compiler admission limit")
}
/// Host configuration, never selected by a model or request body. Saturation
/// returns 429 immediately instead of retaining an unbounded request queue.
pub fn router_with_compilation_limit(
    state: StateData,
    max_concurrent: usize,
) -> Result<Router, &'static str> {
    if max_concurrent == 0 || max_concurrent > tokio::sync::Semaphore::MAX_PERMITS {
        return Err("compiler admission limit must be positive and bounded");
    }
    let session = semantic_db::compiler::typed::CompilationSession::from_shared(
        state.engine.clone(),
        semantic_db::compiler::typed::CompilationCacheOptions {
            max_concurrent,
            ..Default::default()
        },
    )
    .map_err(|_| "invalid compiler cache configuration")?;
    let state = HttpState {
        engine: state.engine,
        compiler: state.compiler,
        session,
        admission: tokio::sync::Semaphore::new(max_concurrent),
        metrics: Arc::new(Default::default()),
        interpreter_metrics: Arc::new(Default::default()),
    };
    Ok(Router::new()
        .route("/health", get(health))
        .route("/catalog", get(catalog))
        .route("/v1/compiler/metrics", get(compiler_metrics))
        .route("/compile", post(compile))
        .route("/v1/compile/semantic", post(compile_semantic))
        .route("/v1/compile/query", post(compile_query))
        .route("/v1/compile/prepared-row", post(compile_prepared_row))
        .route("/v1/compile/graph", post(compile_graph))
        .route("/v1/compile/graph-intent", post(compile_graph_intent))
        .route("/v1/compile/intent", post(compile_intent))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(Arc::new(state)))
}
async fn health(State(s): State<Arc<HttpState>>) -> Json<Value> {
    Json(json!({"ready": true, "ask_enabled": s.compiler.is_some()}))
}
async fn catalog(State(s): State<Arc<HttpState>>) -> Json<Value> {
    Json(semantic_db::interpreter::catalog_context(
        s.engine.catalog(),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    question: String,
}
async fn compile(
    State(s): State<Arc<HttpState>>,
    Json(request): Json<Request>,
) -> Result<Json<Compilation>, (StatusCode, Json<Value>)> {
    let Some(compiler) = &s.compiler else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "Ask is not configured"})),
        ));
    };
    if request.question.trim().is_empty() || request.question.len() > 8000 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Provide a question between 1 and 8000 bytes"})),
        ));
    }
    let _permit = s.admit()?;
    compiler.compile(&s.engine, &request.question).await.map(Json)
        .map_err(|_| (StatusCode::BAD_GATEWAY, Json(json!({"error": "Compilation failed; check model configuration or revise the question"}))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticRequest {
    question: String,
    #[serde(default)]
    context: semantic_db::interpreter::typed::SelectionMode,
    #[serde(default)]
    request_context: Option<semantic_db::compiler::typed::RequestContext>,
}
async fn compile_semantic(
    State(s): State<Arc<HttpState>>,
    Json(request): Json<SemanticRequest>,
) -> Result<Json<semantic_db::interpreter::typed::InterpretedCompilation>, (StatusCode, Json<Value>)>
{
    let Some(compiler) = &s.compiler else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"Ask is not configured"})),
        ));
    };
    if request.question.trim().is_empty() || request.question.len() > 8000 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Provide a question between 1 and 8000 bytes"})),
        ));
    }
    let _permit = s.admit()?;
    let mut options = semantic_db::interpreter::typed::InterpretOptions::default();
    options.compiler = s.options();
    options.metrics = Some(s.interpreter_metrics.clone());
    options.selection_mode = request.context;
    options.request_context = request.request_context;
    Ok(Json(
        compiler
            .compile_typed(&s.engine, &request.question, options)
            .await,
    ))
}
async fn compile_graph(
    State(s): State<Arc<HttpState>>,
    Json(query): Json<semantic_db::plan::graph::GraphQuery>,
) -> Result<Json<semantic_db::compiler::typed::TypedCompilation>, HttpError> {
    let _permit = s.admit()?;
    Ok(Json(
        semantic_db::compiler::typed::compile_graph(&s.engine, query, s.options()).await,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphIntentRequest {
    intent: semantic_db::plan::graph::GraphIntentQuery,
    #[serde(default)]
    request_context: Option<semantic_db::compiler::typed::RequestContext>,
}
async fn compile_graph_intent(
    State(s): State<Arc<HttpState>>,
    Json(request): Json<GraphIntentRequest>,
) -> Result<Json<semantic_db::compiler::typed::TypedCompilation>, HttpError> {
    let _permit = s.admit()?;
    let mut options = s.options();
    options.request_context = request.request_context;
    Ok(Json(
        semantic_db::compiler::typed::compile_graph_intent(&s.engine, request.intent, options)
            .await,
    ))
}
async fn compile_query(
    State(s): State<Arc<HttpState>>,
    Json(query): Json<semantic_db::plan::typed::SemanticQuery>,
) -> Result<Json<semantic_db::compiler::typed::TypedCompilation>, HttpError> {
    let _permit = s.admit()?;
    Ok(Json(s.session.compile(query, s.options()).await))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PreparedTypeWire {
    Boolean,
    Int64,
    Utf8,
    Date32,
    TimestampMicrosUtc,
}
impl From<PreparedTypeWire> for semantic_db::compiler::typed::PreparedType {
    fn from(value: PreparedTypeWire) -> Self {
        use semantic_db::compiler::typed::PreparedType;
        match value {
            PreparedTypeWire::Boolean => PreparedType::Boolean,
            PreparedTypeWire::Int64 => PreparedType::Int64,
            PreparedTypeWire::Utf8 => PreparedType::Utf8,
            PreparedTypeWire::Date32 => PreparedType::Date32,
            PreparedTypeWire::TimestampMicrosUtc => PreparedType::TimestampMicrosUtc,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedDeclarationWire {
    name: String,
    value_type: PreparedTypeWire,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedReferenceWire {
    instant_parameter: String,
    timezone: String,
    calendar: semantic_db::compiler::typed::Calendar,
    origin: semantic_db::compiler::typed::ContextOrigin,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedRowRequest {
    query: semantic_db::plan::typed::RowQuery,
    declarations: Vec<PreparedDeclarationWire>,
    values: BTreeMap<String, semantic_db::plan::typed::Literal>,
    #[serde(default)]
    reference: Option<PreparedReferenceWire>,
}
fn prepared_error(diagnostic: semantic_db::compiler::typed::CompileDiagnostic) -> HttpError {
    let status = match diagnostic.code.as_str() {
        "snapshot_mismatch" => StatusCode::CONFLICT,
        "access_scope" | "execution_scope" => StatusCode::FORBIDDEN,
        _ => StatusCode::BAD_REQUEST,
    };
    // Stable code only: request values and catalog detail never enter HTTP errors.
    (
        status,
        Json(json!({"error":"Prepared row request rejected","code":diagnostic.code})),
    )
}
async fn compile_prepared_row(
    State(s): State<Arc<HttpState>>,
    request: Result<Json<PreparedRowRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<semantic_db::compiler::typed::TypedCompilation>, HttpError> {
    let Json(request) = request.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Invalid prepared row JSON"})),
        )
    })?;
    let _permit = s.admit()?;
    let snapshot = s.engine.catalog().snapshot();
    let current_scope = snapshot
        .relations()
        .map(|relation| relation.definition().name.clone())
        .collect::<BTreeSet<_>>();
    let mut options = s.options();
    // The scope comes from the host's registered engine, never the request.
    // This service currently has no per-user authorization context.
    options.allowed_relations = Some(current_scope.clone());
    let declarations = request
        .declarations
        .into_iter()
        .map(
            |declaration| semantic_db::compiler::typed::ParameterDeclaration {
                name: declaration.name,
                value_type: declaration.value_type.into(),
            },
        )
        .collect();
    let reference =
        request.reference.map(
            |reference| semantic_db::compiler::typed::PreparedReferenceContext {
                instant_parameter: reference.instant_parameter,
                timezone: reference.timezone,
                calendar: reference.calendar,
                origin: reference.origin,
            },
        );
    let prepared = semantic_db::compiler::typed::PreparedRows::prepare(
        &s.engine,
        request.query,
        declarations,
        reference,
        options,
    )
    .map_err(prepared_error)?;
    let compilation = prepared
        .bind_values(&s.engine, request.values, Some(&current_scope))
        .await
        .map_err(prepared_error)?;
    Ok(Json(compilation))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentRequest {
    intent: semantic_db::plan::typed::IntentQuery,
    #[serde(default)]
    request_context: Option<semantic_db::compiler::typed::RequestContext>,
}
async fn compile_intent(
    State(s): State<Arc<HttpState>>,
    Json(request): Json<IntentRequest>,
) -> Result<Json<semantic_db::compiler::typed::TypedCompilation>, HttpError> {
    let _permit = s.admit()?;
    let mut options = s.options();
    options.request_context = request.request_context;
    options.request_evidence = Some(request.intent.evidence);
    Ok(Json(s.session.compile(request.intent.query, options).await))
}
async fn compiler_metrics(State(s): State<Arc<HttpState>>) -> Json<Value> {
    Json(
        json!({"compilations":s.metrics.snapshot(),"interpretations":s.interpreter_metrics.snapshot(),"cache":s.session.stats()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn saturated_admission_returns_429_without_compiling_and_recovers_on_release() {
        let engine = Arc::new(Engine::new());
        let state = Arc::new(HttpState {
            session: semantic_db::compiler::typed::CompilationSession::from_shared(
                engine.clone(),
                Default::default(),
            )
            .unwrap(),
            engine,
            compiler: None,
            metrics: Arc::new(Default::default()),
            interpreter_metrics: Arc::new(Default::default()),
            admission: tokio::sync::Semaphore::new(1),
        });
        let query = serde_json::from_value(json!({"version":1,"input":{"relation":"missing","instance":"r"},"requirements":[],"unresolved":[]})).unwrap();
        let held = state.admit().unwrap();
        let result = compile_query(State(state.clone()), Json(query))
            .await
            .unwrap_err();
        assert_eq!(result.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(state.metrics.snapshot().completed, 0);
        drop(held);
        assert!(state.admit().is_ok());
    }

    #[tokio::test]
    async fn prepared_row_route_uses_the_same_admission_limit() {
        let engine = Arc::new(Engine::new());
        let state = Arc::new(HttpState {
            session: semantic_db::compiler::typed::CompilationSession::from_shared(
                engine.clone(),
                Default::default(),
            )
            .unwrap(),
            engine,
            compiler: None,
            metrics: Arc::new(Default::default()),
            interpreter_metrics: Arc::new(Default::default()),
            admission: tokio::sync::Semaphore::new(1),
        });
        let request = serde_json::from_value(json!({
            "query":{"version":1,"input":{"relation":"missing","instance":"r"},"requirements":[],"unresolved":[]},
            "declarations":[{"name":"p","value_type":"int64"}],
            "values":{"p":{"type":"int64","value":1}}
        }))
        .unwrap();
        let held = state.admit().unwrap();
        let result = compile_prepared_row(State(state.clone()), Ok(Json(request)))
            .await
            .unwrap_err();
        assert_eq!(result.0, StatusCode::TOO_MANY_REQUESTS);
        drop(held);
        assert!(state.admit().is_ok());
    }
}
