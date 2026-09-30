use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use semantic_db::Engine;
use semantic_server::http::{StateData, router};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn app() -> axum::Router {
    let mut engine = Engine::new();
    engine
        .create_view(
            "items",
            "SELECT 1::bigint AS id, 10::bigint AS score UNION ALL SELECT 2::bigint, 20::bigint",
        )
        .await
        .unwrap();
    router(StateData {
        engine: Arc::new(engine),
        compiler: None,
    })
}

fn proposal() -> Value {
    json!({
        "version": 1,
        "input": {"relation":"items","instance":"i"},
        "requirements": [
            {"id":"minimum","source_text":"score at least minimum","operation":{
                "kind":"filter","predicate":{
                    "kind":"compare_parameter",
                    "field":{"instance":"i","field":"score"},
                    "operator":"gt_eq","parameter":"minimum"
                }
            }},
            {"id":"id","source_text":"item IDs","operation":{
                "kind":"project","field":{"instance":"i","field":"id"},"alias":"id"
            }}
        ],
        "unresolved": []
    })
}

fn prepared_request() -> Value {
    json!({
        "query": proposal(),
        "declarations": [{"name":"minimum","value_type":"int64"}],
        "values": {"minimum":{"type":"int64","value":20}}
    })
}

async fn post(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn public_prepared_route_binds_exact_value_without_a_model() {
    let app = app().await;
    let (status, response) = post(&app, "/v1/compile/prepared-row", prepared_request()).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["outcome"]["status"], "compiled", "{response}");
    assert_eq!(response["record"]["work"]["model_calls"], 0);
    let sql = response["outcome"]["query"]["sql"]["statement"]
        .as_str()
        .unwrap();
    assert!(sql.contains("$1"), "{sql}");
    assert!(!sql.contains("= 20"), "{sql}");
    assert_eq!(
        response["outcome"]["query"]["sql"]["parameters"][0]["value"],
        20
    );

    let (ordinary_status, ordinary) = post(&app, "/v1/compile/query", proposal()).await;
    assert_eq!(ordinary_status, StatusCode::OK);
    assert_eq!(ordinary["outcome"]["status"], "rejected");
    assert_eq!(
        ordinary["outcome"]["diagnostic"]["code"],
        "unbound_parameter"
    );
}

#[tokio::test]
async fn public_prepared_route_rejects_bad_values_and_never_echoes_them_in_errors() {
    let app = app().await;
    let mut missing = prepared_request();
    missing["values"] = json!({});
    let (status, body) = post(&app, "/v1/compile/prepared-row", missing).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "parameter_count");

    let mut wrong = prepared_request();
    wrong["values"]["minimum"] = json!({"type":"utf8","value":"private-secret"});
    let (status, body) = post(&app, "/v1/compile/prepared-row", wrong).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "parameter_type");
    assert!(!body.to_string().contains("private-secret"));

    let mut duplicate = prepared_request();
    duplicate["declarations"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"minimum","value_type":"int64"}));
    let (status, body) = post(&app, "/v1/compile/prepared-row", duplicate).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "parameter_contract");

    let mut malformed = prepared_request();
    malformed["values"]["minimum"] = json!({"type":"int64","value":"private-secret"});
    let (status, body) = post(&app, "/v1/compile/prepared-row", malformed).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body.to_string().contains("private-secret"));
}
