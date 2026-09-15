//! Compilation only: successful SQL still executes through the PostgreSQL frontend.
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
};
use semantic_db::{
    Engine,
    compiler::{Compilation, Compiler, provider::OpenAiProvider},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct StateData {
    pub engine: Arc<Engine>,
    pub compiler: Option<Compiler<OpenAiProvider>>,
}
pub fn router(state: StateData) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/catalog", get(catalog))
        .route("/compile", post(compile))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(Arc::new(state))
}
async fn health(State(s): State<Arc<StateData>>) -> Json<Value> {
    Json(json!({"ready": true, "ask_enabled": s.compiler.is_some()}))
}
async fn catalog(State(s): State<Arc<StateData>>) -> Json<Value> {
    Json(semantic_db::compiler::catalog_context(s.engine.catalog()))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    question: String,
}
async fn compile(
    State(s): State<Arc<StateData>>,
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
    compiler.compile(&s.engine, &request.question).await.map(Json)
        .map_err(|_| (StatusCode::BAD_GATEWAY, Json(json!({"error": "Compilation failed; check model configuration or revise the question"}))))
}
