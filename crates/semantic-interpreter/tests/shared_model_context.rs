use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::MemTable,
};
use semantic_catalog::{AiContext, FieldSemantics, Relation, RelationSemantics, SemanticOrigin};
use semantic_engine::Engine;
use semantic_interpreter::{
    Interpreter,
    provider::{Message, ModelProvider, ProviderError},
    typed::{InterpretOptions, SelectionMode, audit_context_manifest},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
struct Observe(Arc<Mutex<Vec<String>>>);
impl ModelProvider for Observe {
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.0.lock().unwrap().push(messages[1].content.clone());
        Ok(r#"{"status":"needs_clarification","phrases":["IDs"],"question":"Which IDs?"}"#.into())
    }
}
fn options(mode: SelectionMode, allowed: BTreeSet<String>) -> InterpretOptions {
    let mut options = InterpretOptions::default();
    options.compiler.allowed_relations = Some(allowed);
    options.selection_mode = mode;
    options
}
fn fixture() -> (Engine, BTreeSet<String>) {
    let mut engine = Engine::new();
    let names: BTreeSet<String> = (0..13)
        .map(|n| format!("family_{n}"))
        .chain(["other_model".into()])
        .collect();
    for name in names.iter().map(String::as_str).chain(["hidden"]) {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let mut relation = Relation::base(name, schema.clone(), "memory");
        let model = if name == "other_model" {
            "other"
        } else if name == "hidden" {
            "secret"
        } else {
            "family"
        };
        let instructions = if name == "hidden" {
            "OUTSIDE_SCOPE_SECRET".into()
        } else {
            "Complete authored knowledge. ".repeat(400)
        };
        let mut semantics = RelationSemantics {
            model_description: Some("Whole model description".into()),
            model_ai_context: Some(AiContext {
                instructions: Some(instructions),
                synonyms: vec!["model alias".into()],
                examples: vec!["model example".into()],
            }),
            ai_context: Some(AiContext {
                instructions: Some(format!("Dataset facts for {name}")),
                ..Default::default()
            }),
            origin: Some(SemanticOrigin {
                format: "ossie".into(),
                version: "1".into(),
                schema_revision: "schema".into(),
                schema_sha256: "schema-sha".into(),
                document_sha256: "pinned-document".into(),
                adapter_version: "adapter".into(),
                model: model.into(),
                dataset: name.into(),
            }),
            ..Default::default()
        };
        semantics.fields.insert(
            "id".into(),
            FieldSemantics {
                ai_context: Some(AiContext {
                    instructions: Some(format!("Field facts for {name}.id")),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        if name == "other_model" {
            semantics.relationships.insert(
                "role".into(),
                semantic_catalog::RelationshipDefinition {
                    ai_context: Some(AiContext {
                        instructions: Some("Role-specific authored hint".into()),
                        ..Default::default()
                    }),
                    id: "roles/illustrative".into(),
                    right_relation: "family_0".into(),
                    role: "illustrative".into(),
                    key_pairs: vec![semantic_catalog::RelationshipKey {
                        left_field: "id".into(),
                        right_field: "id".into(),
                    }],
                    null_keys_match: false,
                    cardinality: semantic_catalog::FactResolution::Unknown,
                    source_refs: vec![],
                },
            );
        }
        relation.semantics = Some(semantics);
        engine
            .register_table(
                relation,
                Arc::new(MemTable::try_new(schema, vec![vec![]]).unwrap()),
            )
            .unwrap();
    }
    (engine, names)
}
fn legacy_payload(value: &Value) -> String {
    let mut legacy = value.clone();
    for relation in legacy["relations"].as_array_mut().unwrap() {
        let semantics = relation["semantics"].as_object_mut().unwrap();
        let reference = semantics.remove("model_context_ref").unwrap();
        let shared = &value["shared_model_contexts"][reference.as_str().unwrap()];
        semantics.insert("model_description".into(), shared["description"].clone());
        semantics.insert("model_ai_context".into(), shared["ai_context"].clone());
    }
    legacy
        .as_object_mut()
        .unwrap()
        .remove("shared_model_contexts");
    legacy["version"] = 1.into();
    serde_json::to_string(&legacy).unwrap()
}
#[tokio::test]
async fn model_information_is_shared_without_losing_scope_or_inline_facts() {
    let (engine, allowed) = fixture();
    let payloads = Arc::new(Mutex::new(vec![]));
    let interpreter = Interpreter::new(Observe(payloads.clone()));
    let mut full_options = options(SelectionMode::Full, allowed.clone());
    full_options.max_context_bytes = 64 * 1024;
    let result = interpreter
        .compile_typed(&engine, "family IDs", full_options.clone())
        .await;
    assert_eq!(
        result.interpretation.work.model_calls, 1,
        "{:?}",
        result.outcome
    );
    let payload = payloads.lock().unwrap()[0].clone();
    let value: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["version"], 2);
    assert_eq!(value["shared_model_contexts"].as_object().unwrap().len(), 2);
    assert!(!payload.contains("OUTSIDE_SCOPE_SECRET") && !payload.contains("\"hidden\""));
    assert_eq!(value["relations"].as_array().unwrap().len(), 14);
    let mut family_ref = None;
    for relation in value["relations"].as_array().unwrap() {
        let name = relation["name"].as_str().unwrap();
        let reference = relation["semantics"]["model_context_ref"].as_str().unwrap();
        if name.starts_with("family_") {
            assert_eq!(*family_ref.get_or_insert(reference), reference);
        } else {
            assert_ne!(Some(reference), family_ref);
        }
        let shared = &value["shared_model_contexts"][reference];
        assert_eq!(
            shared["identity"]["model"],
            if name == "other_model" {
                "other"
            } else {
                "family"
            }
        );
        assert_eq!(
            shared["ai_context"]["synonyms"],
            serde_json::json!(["model alias"])
        );
        assert_eq!(
            shared["ai_context"]["examples"],
            serde_json::json!(["model example"])
        );
        assert_eq!(
            relation["semantics"]["ai_context"]["instructions"],
            format!("Dataset facts for {name}")
        );
        if name == "other_model" {
            assert_eq!(
                relation["semantics"]["relationships"]["role"]["ai_context"]["instructions"],
                "Role-specific authored hint"
            );
        }
        assert_eq!(
            relation["columns"][0]["semantics"]["ai_context"]["instructions"],
            format!("Field facts for {name}.id")
        );
    }
    let legacy = legacy_payload(&value);
    assert!(
        legacy.len() > payload.len() + 100_000,
        "{} -> {} bytes",
        legacy.len(),
        payload.len()
    );
    assert_eq!(
        result.interpretation.contexts[0].payload_bytes,
        payload.len()
    );
    assert!(result.interpretation.contexts[0].audit.complete);
    // Old inline and new referenced payloads have exactly the same pinned facts.
    let mut old_manifest = result.interpretation.contexts[0].clone();
    old_manifest.version = 1;
    old_manifest.payload_bytes = legacy.len();
    use sha2::Digest;
    old_manifest.payload_digest = format!("{:x}", sha2::Sha256::digest(legacy.as_bytes()));
    assert!(audit_context_manifest(&engine.catalog().snapshot(), &old_manifest, &legacy).complete);
    for mixed in ["reference", "group"] {
        let mut candidate: Value = serde_json::from_str(&legacy).unwrap();
        if mixed == "reference" {
            candidate["relations"][0]["semantics"]["model_context_ref"] = "untrusted".into();
        } else {
            candidate["shared_model_contexts"] = value["shared_model_contexts"].clone();
        }
        let payload = serde_json::to_string(&candidate).unwrap();
        let mut manifest = old_manifest.clone();
        manifest.payload_bytes = payload.len();
        manifest.payload_digest = format!("{:x}", sha2::Sha256::digest(payload.as_bytes()));
        assert!(
            !audit_context_manifest(&engine.catalog().snapshot(), &manifest, &payload).complete,
            "v1 {mixed}"
        );
    }

    let repeated = interpreter
        .compile_typed(&engine, "family IDs", full_options)
        .await;
    assert_eq!(payloads.lock().unwrap()[1], payload);
    assert_eq!(
        repeated.interpretation.contexts[0].payload_digest,
        result.interpretation.contexts[0].payload_digest
    );
}
#[tokio::test]
async fn shared_references_and_extra_model_context_are_audited_fail_closed() {
    let (engine, allowed) = fixture();
    let payloads = Arc::new(Mutex::new(vec![]));
    let full_options = options(SelectionMode::Full, allowed);
    let result = Interpreter::new(Observe(payloads.clone()))
        .compile_typed(&engine, "family IDs", full_options)
        .await;
    let value: Value = serde_json::from_str(&payloads.lock().unwrap()[0]).unwrap();
    for mutation in [
        "missing",
        "dangling",
        "wrong_model",
        "changed",
        "extra",
        "inline",
        "mismatched_version",
        "unknown_version",
    ] {
        let mut candidate = value.clone();
        let first = candidate["relations"][0]["semantics"]["model_context_ref"]
            .as_str()
            .unwrap()
            .to_owned();
        match mutation {
            "missing" => {
                candidate["shared_model_contexts"]
                    .as_object_mut()
                    .unwrap()
                    .remove(&first);
            }
            "dangling" => {
                candidate["relations"][0]["semantics"]["model_context_ref"] = "unavailable".into()
            }
            "wrong_model" => {
                let other = candidate["shared_model_contexts"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .find(|id| *id != &first)
                    .unwrap()
                    .clone();
                candidate["relations"][0]["semantics"]["model_context_ref"] = other.into();
            }
            "changed" => {
                candidate["shared_model_contexts"][&first]["ai_context"]["instructions"] =
                    "changed".into()
            }
            "extra" => {
                candidate["shared_model_contexts"]["orphan"] =
                    candidate["shared_model_contexts"][&first].clone()
            }
            "inline" => {
                candidate["relations"][0]["semantics"]["model_ai_context"] =
                    serde_json::json!({"instructions":"UNPINNED_MODEL_INSTRUCTIONS"})
            }
            "mismatched_version" => candidate["version"] = 1.into(),
            "unknown_version" => candidate["version"] = 99.into(),
            _ => unreachable!(),
        }
        let payload = serde_json::to_string(&candidate).unwrap();
        let mut manifest = result.interpretation.contexts[0].clone();
        use sha2::Digest;
        manifest.payload_digest = format!("{:x}", sha2::Sha256::digest(payload.as_bytes()));
        manifest.payload_bytes = payload.len();
        assert!(
            !audit_context_manifest(&engine.catalog().snapshot(), &manifest, &payload).complete,
            "{mutation}"
        );
    }
    let retrieved = options(SelectionMode::Retrieved, ["family_0".into()].into());
    let result = Interpreter::new(Observe(payloads.clone()))
        .compile_typed(&engine, "family_0 id", retrieved)
        .await;
    assert_eq!(
        result.interpretation.work.model_calls, 1,
        "{:?}",
        result.outcome
    );
    assert!(result.interpretation.contexts[0].audit.complete);
    let payload = payloads.lock().unwrap().last().unwrap().clone();
    let value: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["relations"].as_array().unwrap().len(), 1);
    assert_eq!(value["shared_model_contexts"].as_object().unwrap().len(), 1);
    assert_eq!(
        value["shared_model_contexts"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()["identity"]["model"],
        "family"
    );
}

#[tokio::test]
async fn relation_without_model_facts_has_no_shared_entry() {
    let mut engine = Engine::new();
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    engine
        .register_table(
            Relation::base("plain", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![]]).unwrap()),
        )
        .unwrap();
    let payloads = Arc::new(Mutex::new(vec![]));
    let result = Interpreter::new(Observe(payloads.clone()))
        .compile_typed(
            &engine,
            "plain IDs",
            options(SelectionMode::Full, ["plain".into()].into()),
        )
        .await;
    assert_eq!(result.interpretation.work.model_calls, 1);
    let payload: Value = serde_json::from_str(&payloads.lock().unwrap()[0]).unwrap();
    assert_eq!(payload["shared_model_contexts"], serde_json::json!({}));
    assert!(result.interpretation.contexts[0].audit.complete);
}
