use std::sync::{Arc, Mutex};

use semantic_compiler::{
    Compiler, CompilerError, GroundingOutcome, catalog_context,
    provider::{Message, ModelProvider, ProviderError},
};
use semantic_engine::{Engine, pretty_format_batches};
use serde_json::json;

type Calls = Arc<Mutex<Vec<Vec<Message>>>>;

struct ScriptedProvider {
    responses: Mutex<std::collections::VecDeque<String>>,
    calls: Calls,
}

impl ModelProvider for ScriptedProvider {
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.calls.lock().unwrap().push(messages.to_vec());
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected extra model call"))
    }
}

fn compiler(responses: Vec<String>) -> (Compiler<ScriptedProvider>, Calls) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        Compiler::new(ScriptedProvider {
            responses: Mutex::new(responses.into()),
            calls: calls.clone(),
        }),
        calls,
    )
}

async fn fixture() -> Engine {
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
}

fn grounded(sql: &str) -> String {
    json!({"status":"grounded","query":{"sql":sql,"evidence":[{
        "phrase":"wells", "catalog_reference":"wells", "interpretation":"Registered well records"
    }]}})
    .to_string()
}

#[tokio::test]
async fn compiles_explicit_request_and_executes_expected_rows() {
    let engine = fixture().await;
    let (compiler, calls) = compiler(vec![grounded(
        "SELECT well_id FROM wells WHERE status = 'active' AND basin = 'North Basin' AND total_depth_m >= 2500 ORDER BY well_id",
    )]);
    let result = compiler.compile(&engine, "List well_id for active wells in North Basin with total_depth_m >= 2500, ordered by well_id").await.unwrap();
    assert_eq!(result.attempts, 1);
    let GroundingOutcome::Grounded { query } = result.outcome else {
        panic!("expected grounded query")
    };
    let batches = engine
        .plan_generated_sql(&query.sql)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        pretty_format_batches(&batches).unwrap().to_string(),
        "+---------+\n| well_id |\n+---------+\n| W-001   |\n| W-004   |\n+---------+"
    );
    let calls = calls.lock().unwrap();
    let context: serde_json::Value = serde_json::from_str(&calls[0][1].content).unwrap();
    assert_eq!(context["catalog"][0]["name"], "wells");
    assert!(!calls[0][1].content.contains("wells.csv"));
    assert!(!calls[0][1].content.contains("Juniper-1"));
}

#[tokio::test]
async fn preserves_clarification_and_unsupported_without_repair() {
    let engine = fixture().await;
    for (request, outcome) in [
        (
            "Find deep wells",
            json!({"status":"needs_clarification","phrases":["deep"],"question":"What depth cutoff in metres defines deep?"}),
        ),
        (
            "Exclude wells with uncertain locations",
            json!({"status":"unsupported","reason":"The catalog has no location-quality field."}),
        ),
    ] {
        let (compiler, calls) = compiler(vec![outcome.to_string()]);
        let result = compiler.compile(&engine, request).await.unwrap();
        assert_eq!(serde_json::to_value(result.outcome).unwrap(), outcome);
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn repairs_bad_json_and_invalid_sql_with_original_context() {
    let engine = fixture().await;
    for invalid in [
        "not JSON".to_string(),
        grounded("SELECT missing_column FROM wells"),
    ] {
        let (compiler, calls) =
            compiler(vec![invalid.clone(), grounded("SELECT well_id FROM wells")]);
        let result = compiler.compile(&engine, "List well IDs").await.unwrap();
        assert_eq!(result.attempts, 2);
        let calls = calls.lock().unwrap();
        assert_eq!(calls[1].len(), 4);
        assert_eq!(calls[1][1].content, calls[0][1].content);
        assert_eq!(calls[1][2].content, invalid);
        assert!(calls[1][3].content.contains("validation_error"));
    }
}

#[tokio::test]
async fn refuses_invalid_evidence_mutations_and_malformed_outcomes() {
    let engine = fixture().await;
    let invalid = vec![
        grounded("DROP TABLE wells"),
        grounded("SELECT * FROM information_schema.tables"),
        grounded("SELECT * FROM unregistered"),
        grounded("SELECT * FROM wells; SELECT 1"),
        grounded("SELECT * FROM wells").replace(
            "\"catalog_reference\":\"wells\"",
            "\"catalog_reference\":\"wells.nonexistent\"",
        ),
        json!({"status":"grounded","query":{"sql":"SELECT 1","evidence":[]}}).to_string(),
        json!({"status":"unsupported","reason":""}).to_string(),
        json!({"status":"needs_clarification","phrases":[],"question":"Which?"}).to_string(),
        json!({"status":"unsupported","reason":"missing data","sql":"SELECT 1"}).to_string(),
    ];
    for output in invalid {
        let (compiler, calls) = compiler(vec![output]);
        assert!(matches!(
            compiler
                .with_max_repairs(0)
                .compile(&engine, "List wells")
                .await,
            Err(CompilerError::Validation { attempts: 1, .. })
        ));
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
    assert_eq!(engine.catalog().relations().count(), 1);
    assert!(engine.query("SELECT * FROM wells").await.is_ok());
}

#[tokio::test]
async fn bounds_repairs_and_skips_model_for_empty_input_or_catalog() {
    let engine = fixture().await;
    let (compiler, calls) = compiler(vec!["{}".into(), "{}".into()]);
    assert!(matches!(
        compiler.compile(&engine, "List wells").await,
        Err(CompilerError::Validation { attempts: 2, .. })
    ));
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert!(matches!(
        compiler.compile(&engine, "  ").await,
        Err(CompilerError::EmptyRequest)
    ));
    let result = compiler
        .compile(&Engine::new(), "List wells")
        .await
        .unwrap();
    assert_eq!(result.attempts, 0);
    assert!(matches!(
        result.outcome,
        GroundingOutcome::Unsupported { .. }
    ));
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn includes_view_definitions_and_supports_ctes_over_views() {
    let mut engine = fixture().await;
    engine
        .create_view(
            "deep_wells",
            "SELECT * FROM wells WHERE total_depth_m >= 2500",
        )
        .await
        .unwrap();
    let context = catalog_context(engine.catalog());
    assert_eq!(context[0]["name"], "deep_wells");
    assert!(context[0]["view_sql"].as_str().unwrap().contains("2500"));
    let (compiler, _) = compiler(vec![grounded(
        "WITH selected AS (SELECT well_id FROM deep_wells) SELECT * FROM selected",
    )]);
    assert!(
        compiler
            .compile(&engine, "List deep well IDs using deep_wells")
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn provider_errors_do_not_trigger_repairs() {
    struct FailingProvider;
    impl ModelProvider for FailingProvider {
        async fn complete(&self, _: &[Message]) -> Result<String, ProviderError> {
            Err(ProviderError::Http(401))
        }
    }
    let error = Compiler::new(FailingProvider)
        .compile(&fixture().await, "List wells")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CompilerError::Provider(ProviderError::Http(401))
    ));
}
