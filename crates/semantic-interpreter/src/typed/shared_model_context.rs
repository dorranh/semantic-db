//! Model-level context stays shared only within its pinned authored identity.
use semantic_catalog::{AiContext, Relation, canonical_digest};
use serde::Serialize;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ModelIdentity<'a> {
    Document {
        document_sha256: &'a str,
        model: &'a str,
    },
    Relation {
        relation: &'a str,
    },
}
#[derive(Serialize)]
pub(super) struct SharedModelContext<'a> {
    pub identity: ModelIdentity<'a>,
    pub content_revision: String,
    pub description: &'a Option<String>,
    pub ai_context: &'a Option<AiContext>,
}
pub(super) fn from_relation(relation: &Relation) -> Option<(String, SharedModelContext<'_>)> {
    let semantics = relation.semantics.as_ref()?;
    if semantics.model_description.is_none() && semantics.model_ai_context.is_none() {
        return None;
    }
    let identity = match &semantics.origin {
        Some(origin) => ModelIdentity::Document {
            document_sha256: &origin.document_sha256,
            model: &origin.model,
        },
        None => ModelIdentity::Relation {
            relation: &relation.name,
        },
    };
    let content_revision = canonical_digest(
        &serde_json::json!({"description": &semantics.model_description, "ai_context": &semantics.model_ai_context}),
    );
    let context = SharedModelContext {
        identity,
        content_revision,
        description: &semantics.model_description,
        ai_context: &semantics.model_ai_context,
    };
    let id = canonical_digest(&serde_json::to_value(&context).expect("model context serializes"));
    Some((id, context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_catalog::{RelationSemantics, SemanticOrigin};
    fn relation(name: &str, model: Option<&str>) -> Relation {
        let mut relation = Relation::base(
            name,
            std::sync::Arc::new(datafusion::arrow::datatypes::Schema::empty()),
            "memory",
        );
        relation.semantics = Some(RelationSemantics {
            model_description: Some("complete model description".into()),
            model_ai_context: Some(AiContext {
                instructions: Some("authored instructions".into()),
                synonyms: vec!["alias".into()],
                examples: vec!["authored example".into()],
            }),
            origin: model.map(|model| SemanticOrigin {
                format: "ossie".into(),
                version: "1".into(),
                schema_revision: "schema".into(),
                schema_sha256: "schema-sha".into(),
                document_sha256: "document-sha".into(),
                adapter_version: "adapter".into(),
                model: model.into(),
                dataset: name.into(),
            }),
            ..Default::default()
        });
        relation
    }
    #[test]
    fn sharing_requires_pinned_identity_and_exact_content() {
        let first = relation("first", Some("one"));
        let second = relation("second", Some("one"));
        let other_model = relation("third", Some("two"));
        assert_eq!(
            from_relation(&first).unwrap().0,
            from_relation(&second).unwrap().0
        );
        assert_ne!(
            from_relation(&first).unwrap().0,
            from_relation(&other_model).unwrap().0
        );
        let mut other_document = relation("fourth", Some("one"));
        other_document
            .semantics
            .as_mut()
            .unwrap()
            .origin
            .as_mut()
            .unwrap()
            .document_sha256 = "other-document".into();
        assert_ne!(
            from_relation(&first).unwrap().0,
            from_relation(&other_document).unwrap().0
        );
        let mut changed = second;
        changed
            .semantics
            .as_mut()
            .unwrap()
            .model_ai_context
            .as_mut()
            .unwrap()
            .synonyms
            .push("new alias".into());
        assert_ne!(
            from_relation(&first).unwrap().0,
            from_relation(&changed).unwrap().0
        );
        assert_ne!(
            from_relation(&relation("a", None)).unwrap().0,
            from_relation(&relation("b", None)).unwrap().0
        );
        let (_, context) = from_relation(&first).unwrap();
        let serialized = serde_json::to_value(context).unwrap();
        assert_eq!(
            serialized["ai_context"]["instructions"],
            "authored instructions"
        );
        assert_eq!(
            serialized["ai_context"]["synonyms"],
            serde_json::json!(["alias"])
        );
        assert_eq!(
            serialized["ai_context"]["examples"],
            serde_json::json!(["authored example"])
        );
    }
}
