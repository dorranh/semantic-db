use axum::{Json, Router, routing::post};
use semantic_db::{
    Engine,
    compiler::{
        Compiler,
        provider::{OpenAiConfig, OpenAiProvider},
    },
};
use semantic_server::http::{StateData, router};
use serde_json::{Value, json};
use std::sync::Arc;
async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", socket.local_addr().unwrap());
    (
        address,
        tokio::spawn(async move {
            axum::serve(socket, router).await.unwrap();
        }),
    )
}
async fn completion(Json(body): Json<Value>) -> Json<Value> {
    let input: Value =
        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    let response=match input["request"].as_str().unwrap() {
        "show items"=>json!({"status":"selected","selection":{"view":"items","phrase":"items","columns":["id"],"filters":[],"order_by":[]}}).to_string(),
        "stale items"=>json!({"status":"needs_clarification","phrases":["stale"],"question":"What age counts as stale?"}).to_string(),
        "delete items"=>json!({"status":"unsupported","reason":"Writes are unsupported"}).to_string(),
        _=>"invalid JSON".into(),
    };
    Json(json!({"choices":[{"message":{"content":response},"finish_reason":"stop"}]}))
}
#[tokio::test]
async fn compilation_outcomes_and_disabled_mode() {
    let mut engine = Engine::new();
    engine.create_view("items", "SELECT 1 AS id").await.unwrap();
    let engine = Arc::new(engine);
    let (model, model_task) =
        serve(Router::new().route("/chat/completions", post(completion))).await;
    let mut config = OpenAiConfig::new("fixture".into(), "fixture".into());
    config.base_url = model;
    let compiler = Compiler::new(OpenAiProvider::new(config).unwrap());
    let (url, server) = serve(router(StateData {
        engine: engine.clone(),
        compiler: Some(compiler),
    }))
    .await;
    let client = reqwest::Client::new();
    for (question, status) in [
        ("show items", "grounded"),
        ("stale items", "needs_clarification"),
        ("delete items", "unsupported"),
    ] {
        let response = client
            .post(format!("{url}/compile"))
            .json(&json!({"question":question}))
            .send()
            .await
            .unwrap();
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["outcome"]["status"], status, "{value}");
        if status == "grounded" {
            assert!(
                value["outcome"]["query"]["sql"]
                    .as_str()
                    .unwrap()
                    .contains("items")
            );
            assert!(
                !value["outcome"]["query"]["evidence"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(value.get("rows").is_none());
    }
    assert_eq!(
        client
            .post(format!("{url}/compile"))
            .json(&json!({"question":"bad output"}))
            .send()
            .await
            .unwrap()
            .status(),
        502
    );
    model_task.abort();
    assert_eq!(
        client
            .post(format!("{url}/compile"))
            .json(&json!({"question":"show items"}))
            .send()
            .await
            .unwrap()
            .status(),
        502
    );
    server.abort();
    let (url, server) = serve(router(StateData {
        engine,
        compiler: None,
    }))
    .await;
    assert_eq!(
        client
            .get(format!("{url}/health"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()["ask_enabled"],
        false
    );
    assert_eq!(
        client
            .post(format!("{url}/compile"))
            .json(&json!({"question":"show items"}))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    server.abort();
}
