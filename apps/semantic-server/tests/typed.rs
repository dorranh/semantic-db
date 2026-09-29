use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use semantic_db::Engine;
use semantic_server::http::{StateData, router};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn structured_route_needs_no_model_and_never_executes_rows() {
    let mut engine = Engine::new();
    engine
        .create_view("items", "SELECT 42::bigint AS id")
        .await
        .unwrap();
    let app = router(StateData {
        engine: Arc::new(engine),
        compiler: None,
    });
    let body = json!({"version":1,"input":{"relation":"items","instance":"r"},"requirements":[{"id":"ids","source_text":"IDs","operation":{"kind":"project","field":{"instance":"r","field":"id"},"alias":"id"}}],"unresolved":[]});
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/compile/query")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let compiled: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(compiled["version"], 1);
    assert_eq!(compiled["outcome"]["status"], "compiled");
    assert_eq!(compiled["record"]["work"]["model_calls"], 0);
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/compile/query")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let cached: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(
        cached["record"]["cache_status"],
        "hit_same_snapshot_and_scope"
    );
    let response = app
        .clone()
        .oneshot(
            Request::get("/v1/compiler/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let metrics: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(metrics["compilations"]["completed"], 2);
    assert_eq!(metrics["cache"]["hits"], 1);
    for path in ["/compile", "/v1/compile/semantic"] {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"question":"IDs"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    let intent = json!({"intent":{"query":body.clone(),"evidence":{"version":1,"request_id":"r1","original_request":"IDs","requirement_spans":{"ids":[{"start":0,"end":3}]},"unresolved_alternatives":[]}}});
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/compile/intent")
                .header("content-type", "application/json")
                .body(Body::from(intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(result["outcome"]["status"], "compiled");
    assert_eq!(result["record"]["request_spans_validated"], true);
    let mut stale = body;
    stale["version"] = json!(99);
    let response = app
        .oneshot(
            Request::post("/v1/compile/query")
                .header("content-type", "application/json")
                .body(Body::from(stale.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let rejected: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(rejected["outcome"]["status"], "rejected");
    assert_eq!(
        rejected["outcome"]["diagnostic"]["code"],
        "unsupported_version"
    );
}

#[tokio::test]
async fn graph_route_compiles_sets_without_a_model() {
    let mut engine = Engine::new();
    engine
        .create_view("items", "SELECT 42::bigint AS id")
        .await
        .unwrap();
    let app = router(StateData {
        engine: Arc::new(engine),
        compiler: None,
    });
    let leaf = json!({"version":1,"input":{"relation":"items","instance":"r"},"requirements":[{"id":"ids","source_text":"IDs","operation":{"kind":"project","field":{"instance":"r","field":"id"},"alias":"id"}}],"unresolved":[]});
    let graph = json!({"version":1,"nodes":[
        {"id":"input","source_text":"IDs","operation":{"kind":"rows","query":leaf}},
        {"id":"twice","source_text":"duplicate IDs","operation":{"kind":"set","left":"input","right":"input","operator":"union","duplicates":"all","columns":[{"id":"id","left":"ids","right":"ids","alias":"id"}]}}
    ],"root":"twice","ordering":[],"limit":null,"unresolved":[]});
    let response = app
        .oneshot(
            Request::post("/v1/compile/graph")
                .header("content-type", "application/json")
                .body(Body::from(graph.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let compiled: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(compiled["outcome"]["status"], "compiled_graph");
    assert_eq!(compiled["record"]["outcome"], "compiled");
    assert_eq!(compiled["record"]["work"]["model_calls"], 0);
}

#[tokio::test]
async fn graph_intent_route_validates_final_order_and_limit_without_a_model() {
    let mut engine = Engine::new();
    engine
        .create_view("items", "SELECT 42::bigint AS id")
        .await
        .unwrap();
    let app = router(StateData {
        engine: Arc::new(engine),
        compiler: None,
    });
    let mut body = json!({"intent": {
        "query": {"version":1,"nodes":[{"id":"input","source_text":"IDs","operation":{"kind":"rows","query":{
            "version":1,"input":{"relation":"items","instance":"r"},"requirements":[
                {"id":"ids","source_text":"IDs","operation":{"kind":"project","field":{"instance":"r","field":"id"},"alias":"id"}}
            ],"unresolved":[]
        }}}],"root":"input","ordering":[{"slot":"ids","direction":"asc","nulls":"last"}],"limit":1,"unresolved":[]},
        "evidence":{"version":1,"request_id":"http-graph","original_request":"IDs ascending first one","requirements":[
            {"target":{"kind":"node","node":"input"},"source_spans":[{"start":0,"end":3}]},
            {"target":{"kind":"leaf","node":"input","requirement":"ids"},"source_spans":[{"start":0,"end":3}]},
            {"target":{"kind":"order","index":0},"source_spans":[{"start":4,"end":13}]},
            {"target":{"kind":"limit"},"source_spans":[{"start":14,"end":23}]}
        ],"unresolved_alternatives":[]}
    }});
    for expected in ["compiled_graph", "rejected"] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/v1/compile/graph-intent")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(value["outcome"]["status"], expected, "{value}");
        assert_eq!(value["record"]["work"]["model_calls"], 0);
        if expected == "compiled_graph" {
            assert_eq!(value["record"]["request_spans_validated"], true);
            assert_eq!(
                value["record"]["requirement_dispositions"]
                    .as_array()
                    .unwrap()
                    .len(),
                4
            );
        } else {
            assert_eq!(value["outcome"]["diagnostic"]["code"], "request_coverage");
        }
        body["intent"]["evidence"]["requirements"]
            .as_array_mut()
            .unwrap()
            .pop();
    }
}
