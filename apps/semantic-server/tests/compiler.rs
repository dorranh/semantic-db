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
    let catalog = input["catalog"].as_array().unwrap();
    assert!(
        catalog
            .iter()
            .any(|r| r["name"] == "wells" && r["view_sql"].is_null())
    );
    assert!(
        catalog
            .iter()
            .any(|r| r["name"] == "items" && r["view_sql"].is_string())
    );
    let grounded = |sql: &str, relation: &str| {
        json!({"status":"grounded","query":{"sql":sql,"evidence":[{
            "phrase":input["request"], "catalog_reference":relation,
            "interpretation":format!("Use {relation} for the requested well IDs or count")
        }]}})
        .to_string()
    };
    let response = match input["request"].as_str().unwrap() {
        "show items" => grounded("SELECT id FROM items ORDER BY id", "items"),
        "count items" => grounded("SELECT COUNT(*) AS count FROM items", "items"),
        "show suspended wells" => grounded("SELECT well_id FROM wells WHERE status = 'suspended'", "wells"),
        "stale items" => json!({"status":"needs_clarification","phrases":["stale"],"question":"What age counts as stale?"}).to_string(),
        "delete items" => json!({"status":"unsupported","reason":"Writes are unsupported"}).to_string(),
        "invalid SQL" => grounded("SELECT missing FROM items", "items"),
        "unsafe SQL" => grounded("DELETE FROM wells", "wells"),
        "invalid evidence" => grounded("SELECT id FROM items", "missing"),
        _ => "invalid JSON".into(),
    };
    Json(json!({"choices":[{"message":{"content":response},"finish_reason":"stop"}]}))
}
#[tokio::test]
async fn compilation_outcomes_and_disabled_mode() {
    let mut engine = Engine::new();
    engine
        .register_csv(
            "wells",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/geospatial/wells.csv"
            ),
        )
        .await
        .unwrap();
    engine
        .create_view(
            "items",
            "SELECT well_id AS id FROM wells WHERE status = 'active'",
        )
        .await
        .unwrap();
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
    for (question, status, expected_rows) in [
        ("show items", "grounded", vec!["W-001", "W-003", "W-004"]),
        ("count items", "grounded", vec!["3"]),
        ("show suspended wells", "grounded", vec!["W-002"]),
        ("stale items", "needs_clarification", vec![]),
        ("delete items", "unsupported", vec![]),
    ] {
        let response = client
            .post(format!("{url}/compile"))
            .json(&json!({"question":question}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["outcome"]["status"], status, "{value}");
        if status == "grounded" {
            let sql = value["outcome"]["query"]["sql"].as_str().unwrap();
            let batches = engine.query(sql).await.unwrap();
            let values: Vec<_> = batches
                .iter()
                .flat_map(|batch| {
                    (0..batch.num_rows()).map(|row| {
                        datafusion::arrow::util::display::array_value_to_string(
                            batch.column(0),
                            row,
                        )
                        .unwrap()
                    })
                })
                .collect();
            assert_eq!(values, expected_rows);
            assert!(
                !value["outcome"]["query"]["evidence"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(value.get("rows").is_none());
        assert!(value.get("view_selection").is_none());
    }
    for question in [
        "bad output",
        "invalid SQL",
        "unsafe SQL",
        "invalid evidence",
    ] {
        assert_eq!(
            client
                .post(format!("{url}/compile"))
                .json(&json!({"question":question}))
                .send()
                .await
                .unwrap()
                .status(),
            502
        );
    }
    for question in [String::new(), " ".into(), "x".repeat(8001)] {
        assert_eq!(
            client
                .post(format!("{url}/compile"))
                .json(&json!({"question":question}))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
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
