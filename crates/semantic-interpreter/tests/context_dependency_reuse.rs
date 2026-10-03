use std::{collections::BTreeSet, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_engine::Engine;
use semantic_interpreter::{
    Interpreter,
    provider::{Message, ModelProvider, ProviderError},
    typed::{InterpretOptions, SelectionMode},
};

struct Unsupported;
impl ModelProvider for Unsupported {
    async fn complete(&self, _messages: &[Message]) -> Result<String, ProviderError> {
        Ok(r#"{"status":"unsupported","reason":"No matching calculation"}"#.into())
    }
}

fn register(engine: &mut Engine, name: &str, extra_field: bool) {
    let mut fields = vec![Field::new("id", DataType::Int64, false)];
    if extra_field {
        fields.push(Field::new("changed", DataType::Utf8, true));
    }
    let schema = Arc::new(Schema::new(fields));
    engine
        .register_table(
            Relation::base(name, schema.clone(), format!("source:{name}")),
            Arc::new(MemTable::try_new(schema, vec![vec![]]).unwrap()),
        )
        .unwrap();
}

fn options(mode: SelectionMode, name: &str) -> InterpretOptions {
    let mut options = InterpretOptions::default();
    options.selection_mode = mode;
    options.allowed_relations = Some(BTreeSet::from([name.into()]));
    options
}

#[tokio::test]
async fn unrelated_publication_reuses_selection_but_renders_current_snapshot() {
    let compiler = Interpreter::new(Unsupported);
    let mut engine = Engine::new();
    register(&mut engine, "items", false);
    let first = compiler
        .compile_typed(&engine, "items", options(SelectionMode::Full, "items"))
        .await;
    let first_manifest = &first.interpretation.contexts[0];
    assert_eq!(first_manifest.included[0].fields, vec!["id"]);

    register(&mut engine, "unrelated", false);
    let second = compiler
        .compile_typed(&engine, "items", options(SelectionMode::Full, "items"))
        .await;
    let second_manifest = &second.interpretation.contexts[0];
    assert_eq!(
        second.interpretation.cache_status,
        "selection_hit_re_rendered"
    );
    assert_ne!(first_manifest.snapshot_id, second_manifest.snapshot_id);
    assert_eq!(second_manifest.included[0].fields, vec!["id"]);
    assert_eq!(
        second_manifest.included[0].reference,
        first_manifest.included[0].reference
    );
    assert_eq!(
        second.interpretation.work.context_bytes,
        first.interpretation.work.context_bytes
    );
}

#[tokio::test]
async fn changed_or_newly_present_selected_name_invalidates_selection() {
    let compiler = Interpreter::new(Unsupported);
    let mut first_engine = Engine::new();
    register(&mut first_engine, "items", false);
    compiler
        .compile_typed(
            &first_engine,
            "items",
            options(SelectionMode::Full, "items"),
        )
        .await;

    let mut changed_engine = Engine::new();
    register(&mut changed_engine, "items", true);
    let changed = compiler
        .compile_typed(
            &changed_engine,
            "items",
            options(SelectionMode::Full, "items"),
        )
        .await;
    assert_ne!(
        changed.interpretation.cache_status,
        "selection_hit_re_rendered"
    );
    assert_eq!(
        changed.interpretation.contexts[0].included[0].fields,
        vec!["changed", "id"]
    );

    let mut missing_engine = Engine::new();
    register(&mut missing_engine, "unrelated", false);
    let missing = compiler
        .compile_typed(
            &missing_engine,
            "future",
            options(SelectionMode::Full, "future"),
        )
        .await;
    assert!(missing.interpretation.contexts[0].included.is_empty());
    register(&mut missing_engine, "future", false);
    let present = compiler
        .compile_typed(
            &missing_engine,
            "future",
            options(SelectionMode::Full, "future"),
        )
        .await;
    assert_ne!(
        present.interpretation.cache_status,
        "selection_hit_re_rendered"
    );
    assert_eq!(present.interpretation.contexts[0].included.len(), 1);
}

#[tokio::test]
async fn ranked_retrieval_still_rebuilds_after_unrelated_publication() {
    let compiler = Interpreter::new(Unsupported);
    let mut engine = Engine::new();
    register(&mut engine, "items", false);
    compiler
        .compile_typed(&engine, "items", options(SelectionMode::Retrieved, "items"))
        .await;
    register(&mut engine, "unrelated", false);
    let changed = compiler
        .compile_typed(&engine, "items", options(SelectionMode::Retrieved, "items"))
        .await;
    assert_ne!(
        changed.interpretation.cache_status,
        "selection_hit_re_rendered"
    );
    assert!(changed.interpretation.work.index_objects_visited > 0);
}
