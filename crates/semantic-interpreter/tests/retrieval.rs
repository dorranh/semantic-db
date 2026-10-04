use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::MemTable,
};
use semantic_catalog::{AiContext, FieldSemantics, Relation, RelationSemantics};
use semantic_compiler::typed::{TypedOutcome, compile_rows};
use semantic_engine::Engine;
use semantic_interpreter::{
    Interpreter,
    provider::{Message, ModelProvider, ProviderError},
    typed::{InterpretOptions, SelectionMode},
};
use semantic_plan::typed::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
type Calls = Arc<Mutex<Vec<Vec<Message>>>>;
struct Scripted {
    replies: Mutex<Vec<String>>,
    calls: Calls,
}
impl ModelProvider for Scripted {
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.calls.lock().unwrap().push(messages.to_vec());
        Ok(self.replies.lock().unwrap().remove(0))
    }
}
fn make_compiler(replies: Vec<Value>) -> (Interpreter<Scripted>, Calls) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        Interpreter::new(Scripted {
            replies: Mutex::new(replies.into_iter().map(|r| r.to_string()).collect()),
            calls: calls.clone(),
        }),
        calls,
    )
}
fn fixture() -> Engine {
    let mut engine = Engine::new();
    for name in ["wide", "alternative", "distractor"] {
        let mut fields = (0..1000)
            .map(|i| Field::new(format!("f{i}"), DataType::Int64, false))
            .collect::<Vec<_>>();
        fields.push(Field::new("rare_signal", DataType::Int64, true));
        let schema = Arc::new(Schema::new(fields));
        let mut relation = Relation::base(name, schema.clone(), "private:source");
        let mut semantics = RelationSemantics {
            model_description: Some("No historical status".into()),
            ai_context: Some(AiContext {
                synonyms: vec![format!("{name} concept")],
                ..Default::default()
            }),
            ..Default::default()
        };
        semantics.fields.insert(
            "rare_signal".into(),
            FieldSemantics {
                description: Some("Current measurements only".into()),
                ..Default::default()
            },
        );
        semantics.fields.insert(
            "f950".into(),
            FieldSemantics {
                description: Some("UNSELECTED_FIELD_PROSE".into()),
                ..Default::default()
            },
        );
        relation.semantics = Some(semantics);
        engine
            .register_table(
                relation,
                Arc::new(MemTable::try_new(schema, vec![vec![]]).unwrap()),
            )
            .unwrap();
    }
    engine
}
fn proposal(relation: &str, field: &str) -> Value {
    json!({"status":"query","query":query(relation,field)})
}
fn query(relation: &str, field: &str) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: relation.into(),
            instance: "r".into(),
        },
        requirements: vec![Requirement {
            id: "output".into(),
            source_text: field.into(),
            operation: RowOperation::Project {
                field: FieldRef {
                    instance: "r".into(),
                    field: field.into(),
                },
                alias: "value".into(),
            },
        }],
        unresolved: vec![],
    }
}
fn retrieved() -> InterpretOptions {
    let mut options = InterpretOptions::default();
    options.selection_mode = SelectionMode::Retrieved;
    options.max_context_fields = 32;
    options
}
#[tokio::test]
async fn field_search_hydrates_governing_metadata_without_full_wide_schemas() {
    let engine = fixture();
    let (compiler, calls) = make_compiler(vec![proposal("wide", "rare_signal")]);
    let result = compiler
        .compile_typed(&engine, "rare_signal", retrieved())
        .await;
    assert!(
        matches!(result.outcome, TypedOutcome::Compiled { .. }),
        "{:?}",
        result.outcome
    );
    assert_eq!(result.interpretation.work.context_fields, 15);
    assert_eq!(result.interpretation.contexts[0].included.len(), 3);
    assert!(
        result.interpretation.contexts[0]
            .included
            .iter()
            .all(|r| !r.field_inventory_complete)
    );
    let text = &calls.lock().unwrap()[0][1].content;
    assert!(text.contains("No historical status") && text.contains("Current measurements only"));
    assert!(!text.contains("UNSELECTED_FIELD_PROSE") && !text.contains("private:source"));
    assert!(!result.interpretation.contexts[0].semantic_sufficiency_proven);
}
#[tokio::test]
async fn unhydrated_proposals_expand_and_reconsider_before_binding() {
    let engine = fixture();
    let (compiler, calls) = make_compiler(vec![proposal("wide", "f900"), proposal("wide", "f900")]);
    let result = compiler.compile_typed(&engine, "wide", retrieved()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Compiled { .. }),
        "{:?}",
        result.outcome
    );
    assert_eq!(result.interpretation.work.context_expansions, 1);
    assert_eq!(result.interpretation.work.model_calls, 2);
    assert_eq!(result.interpretation.contexts.len(), 2);
    let first_call_bytes = {
        let calls = calls.lock().unwrap();
        assert!(!calls[0][1].content.contains("f900"));
        assert!(calls[1][1].content.contains("f900"));
        assert!(calls[1].last().unwrap().content.contains("Reconsider"));

        let first_call_bytes = calls[0]
            .iter()
            .map(|message| message.content.len())
            .sum::<usize>();
        let second_call_bytes = calls[1]
            .iter()
            .map(|message| message.content.len())
            .sum::<usize>();
        assert!(second_call_bytes > first_call_bytes);
        first_call_bytes
    };
    let (compiler, limited_calls) =
        make_compiler(vec![proposal("wide", "f900"), proposal("wide", "f900")]);
    let mut options = retrieved();
    options.max_model_call_bytes = first_call_bytes + options.max_model_output_bytes;
    let result = compiler.compile_typed(&engine, "wide", options).await;
    assert!(matches!(
        result.outcome,
        TypedOutcome::Unresolved { ref diagnostic } if diagnostic.code == "context_limit"
    ));
    assert_eq!(limited_calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn explicit_search_and_inventory_expansion_share_one_budget() {
    let engine = fixture();
    let (compiler, _) = make_compiler(vec![
        json!({"status":"need_context","requests":[{"kind":"inventory","relation":"wide","offset":900,"count":2}]}),
        proposal("wide", "f901"),
    ]);
    let result = compiler.compile_typed(&engine, "wide", retrieved()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Compiled { .. }),
        "{:?}",
        result.outcome
    );
    assert_eq!(result.interpretation.work.context_expansions, 1);
    let (compiler, _) = make_compiler(vec![
        json!({"status":"need_context","requests":[{"kind":"search","terms":"f999","relation":"wide"}]}),
        proposal("wide", "f999"),
    ]);
    let result = compiler
        .compile_typed(&engine, "unknown concept", retrieved())
        .await;
    assert!(
        matches!(result.outcome, TypedOutcome::Compiled { .. }),
        "{:?}",
        result.outcome
    );
}
#[tokio::test]
async fn limits_and_partial_absence_are_unresolved_without_silent_fallback() {
    let engine = fixture();
    let (compiler, calls) = make_compiler(vec![]);
    let mut options = retrieved();
    options.max_search_candidates = 1;
    let result = compiler
        .compile_typed(&engine, "rare_signal", options)
        .await;
    assert!(
        matches!(result.outcome,TypedOutcome::Unresolved {ref diagnostic} if diagnostic.code=="search_limit")
    );
    assert!(calls.lock().unwrap().is_empty());
    let (compiler, _) = make_compiler(vec![
        json!({"status":"unsupported","reason":"No such field"}),
    ]);
    assert!(matches!(
        compiler
            .compile_typed(&engine, "missing", retrieved())
            .await
            .outcome,
        TypedOutcome::Unresolved { .. }
    ));
    let (compiler, _) = make_compiler(vec![proposal("wide", "f999")]);
    let mut options = retrieved();
    options.max_expansions = 0;
    assert!(
        matches!(compiler.compile_typed(&engine,"wide",options).await.outcome,TypedOutcome::Unresolved {ref diagnostic} if diagnostic.code=="expansion_limit")
    );
}
#[tokio::test]
async fn access_scope_applies_to_search_hydration_and_structured_binding() {
    let engine = fixture();
    let mut options = retrieved();
    options.allowed_relations = Some(["wide".to_owned()].into_iter().collect());
    let (compiler, calls) = make_compiler(vec![proposal("alternative", "rare_signal")]);
    let result = compiler
        .compile_typed(&engine, "rare_signal", options.clone())
        .await;
    assert!(
        matches!(result.outcome,TypedOutcome::Rejected {ref diagnostic} if diagnostic.code=="access_scope")
    );
    assert!(!calls.lock().unwrap()[0][1].content.contains("alternative"));
    let result = compile_rows(&engine, query("alternative", "f1"), options.compiler).await;
    assert!(
        matches!(result.outcome,TypedOutcome::Rejected {ref diagnostic} if diagnostic.code=="access_scope")
    );
}
#[tokio::test]
async fn auto_falls_back_to_retrieved_and_model_input_budget_includes_protocol() {
    let engine = fixture();
    let (compiler, _) = make_compiler(vec![proposal("wide", "rare_signal")]);
    let mut options = retrieved();
    options.selection_mode = SelectionMode::Auto;
    let result = compiler
        .compile_typed(&engine, "rare_signal", options)
        .await;
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
    assert_eq!(
        result.interpretation.contexts[0].selection_mode,
        SelectionMode::Retrieved
    );
    let (compiler, calls) = make_compiler(vec![]);
    let mut options = retrieved();
    options.max_total_model_input_bytes = 1;
    assert!(
        matches!(compiler.compile_typed(&engine,"wide",options).await.outcome,TypedOutcome::Unresolved {ref diagnostic} if diagnostic.code=="model_input_limit")
    );
    assert!(calls.lock().unwrap().is_empty());

    let (compiler, calls) = make_compiler(vec![]);
    let mut options = retrieved();
    options.max_model_call_bytes = options.max_model_output_bytes;
    assert!(matches!(
        compiler.compile_typed(&engine, "wide", options).await.outcome,
        TypedOutcome::Unresolved { ref diagnostic } if diagnostic.code == "context_limit"
    ));
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn auto_uses_retrieved_context_when_full_catalog_exceeds_model_call_envelope() {
    let engine = fixture();
    let (compiler, calls) = make_compiler(vec![proposal("wide", "rare_signal")]);
    let result = compiler
        .compile_typed(&engine, "rare_signal", retrieved())
        .await;
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
    let retrieved_input_bytes = calls.lock().unwrap()[0]
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>();

    let (compiler, calls) = make_compiler(vec![proposal("wide", "rare_signal")]);
    let mut options = retrieved();
    options.selection_mode = SelectionMode::Auto;
    options.max_context_bytes = 1024 * 1024;
    options.max_model_call_bytes = retrieved_input_bytes + options.max_model_output_bytes;
    let result = compiler
        .compile_typed(&engine, "rare_signal", options)
        .await;
    assert!(
        matches!(result.outcome, TypedOutcome::Compiled { .. }),
        "{:?}",
        result.outcome
    );
    assert_eq!(
        result.interpretation.contexts[0].selection_mode,
        SelectionMode::Retrieved
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn repair_conversation_rechecks_the_complete_model_call_envelope() {
    let engine = fixture();
    let replies = vec![json!({}), proposal("wide", "rare_signal")];
    let (compiler, calls) = make_compiler(replies.clone());
    let result = compiler
        .compile_typed(&engine, "rare_signal", retrieved())
        .await;
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
    let first_call_bytes = {
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let first_call_bytes = calls[0]
            .iter()
            .map(|message| message.content.len())
            .sum::<usize>();
        let second_call_bytes = calls[1]
            .iter()
            .map(|message| message.content.len())
            .sum::<usize>();
        assert!(second_call_bytes > first_call_bytes);
        first_call_bytes
    };

    let (compiler, limited_calls) = make_compiler(replies);
    let mut options = retrieved();
    options.max_model_call_bytes = first_call_bytes + options.max_model_output_bytes;
    let result = compiler
        .compile_typed(&engine, "rare_signal", options)
        .await;
    assert!(matches!(
        result.outcome,
        TypedOutcome::Unresolved { ref diagnostic } if diagnostic.code == "model_context_limit"
    ));
    assert_eq!(limited_calls.lock().unwrap().len(), 1);
}
