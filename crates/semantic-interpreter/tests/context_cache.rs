use std::sync::Arc;

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::TypedOutcome;
use semantic_engine::Engine;
use semantic_interpreter::{
    Interpreter,
    provider::{Message, ModelProvider, ProviderError},
    typed::{InterpretOptions, SelectionMode},
};

struct Unsupported;
impl ModelProvider for Unsupported {
    async fn complete(&self, _messages: &[Message]) -> Result<String, ProviderError> {
        Ok(r#"{"status":"unsupported","reason":"No requested calculation"}"#.into())
    }
}

fn register(engine: &mut Engine, name: &str) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("state", DataType::Utf8, false),
    ]));
    engine
        .register_table(
            Relation::base(name, schema.clone(), format!("source:{name}")),
            Arc::new(MemTable::try_new(schema, vec![vec![]]).unwrap()),
        )
        .unwrap();
}

fn options() -> InterpretOptions {
    let mut options = InterpretOptions::default();
    options.selection_mode = SelectionMode::Retrieved;
    options
}

#[tokio::test]
async fn repeated_context_reuses_rendering_and_respects_request_and_snapshot() {
    let mut engine = Engine::new();
    register(&mut engine, "items");
    let compiler = Interpreter::new(Unsupported);
    let first = compiler.compile_typed(&engine, "items", options()).await;
    assert!(
        matches!(first.outcome, TypedOutcome::Unresolved { ref diagnostic } if diagnostic.code == "partial_catalog")
    );
    assert!(first.interpretation.work.index_objects_visited > 0);
    let second = compiler.compile_typed(&engine, "items", options()).await;
    assert!(
        matches!(second.outcome, TypedOutcome::Unresolved { ref diagnostic } if diagnostic.code == "partial_catalog")
    );
    assert_eq!(
        second.interpretation.cache_status,
        "context_hit_same_snapshot"
    );
    assert_eq!(second.interpretation.work.index_objects_visited, 0);
    assert_eq!(
        second.interpretation.work.context_bytes,
        first.interpretation.work.context_bytes
    );

    let changed_request = compiler
        .compile_typed(&engine, "items current", options())
        .await;
    assert_ne!(
        changed_request.interpretation.cache_status,
        "context_hit_same_snapshot"
    );

    register(&mut engine, "later");
    let changed_snapshot = compiler.compile_typed(&engine, "items", options()).await;
    assert_ne!(
        changed_snapshot.interpretation.cache_status,
        "context_hit_same_snapshot"
    );
    assert_ne!(
        first.interpretation.snapshot_id,
        changed_snapshot.interpretation.snapshot_id
    );
}

#[tokio::test]
async fn concurrent_compiles_share_one_initial_context_build() {
    let mut engine = Engine::new();
    register(&mut engine, "items");
    let engine = Arc::new(engine);
    let compiler = Arc::new(Interpreter::new(Unsupported));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let compiler = compiler.clone();
        let engine = engine.clone();
        tasks.push(tokio::spawn(async move {
            compiler.compile_typed(&engine, "items", options()).await
        }));
    }
    let mut hits = 0;
    let mut builds = 0;
    for task in tasks {
        let compilation = task.await.unwrap();
        assert!(
            matches!(compilation.outcome, TypedOutcome::Unresolved { ref diagnostic } if diagnostic.code == "partial_catalog")
        );
        if compilation.interpretation.cache_status == "context_hit_same_snapshot" {
            hits += 1;
        } else {
            builds += 1;
        }
    }
    assert_eq!((builds, hits), (1, 7));
}
