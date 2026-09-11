use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use semantic_db::{
    Catalog, Engine, EngineError, Relation, RelationBackend, RelationKind, TableProvider,
    arrow::{array::Int64Array, record_batch::RecordBatch},
    catalog::{CatalogError, DataType, Field, Schema, SchemaRef},
    datafusion::{
        datasource::MemTable,
        error::{DataFusionError, Result},
    },
};

#[derive(Default)]
struct Backend {
    providers: BTreeMap<String, Arc<dyn TableProvider>>,
    calls: Mutex<Vec<String>>,
}

impl RelationBackend for Backend {
    async fn resolve(&self, relation: &Relation) -> Result<Arc<dyn TableProvider>> {
        self.calls.lock().unwrap().push(relation.name.clone());
        let RelationKind::Base { source } = &relation.kind else {
            panic!("views must not reach the backend")
        };
        self.providers
            .get(source)
            .cloned()
            .ok_or_else(|| DataFusionError::Plan("source unavailable".into()))
    }
}

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]))
}

fn table(values: Vec<i64>) -> Arc<dyn TableProvider> {
    let batch = RecordBatch::try_new(schema(), vec![Arc::new(Int64Array::from(values))]).unwrap();
    Arc::new(MemTable::try_new(schema(), vec![vec![batch]]).unwrap())
}

fn base(name: &str) -> Relation {
    Relation::base(name, schema(), "private-source")
}

fn view(name: &str, sql: &str) -> Relation {
    Relation::view(name, schema(), sql)
}

async fn fixture() -> Engine {
    let backend = Backend {
        providers: BTreeMap::from([
            ("private-source".into(), table(vec![1, 2, 3])),
            ("selection-source".into(), table(vec![1, 3])),
        ]),
        ..Default::default()
    };
    let mut selected = view(
        "selected",
        "WITH chosen AS (SELECT id FROM ids WHERE id > 1) SELECT id FROM chosen",
    );
    // Imported lineage can be stale: SQL is authoritative.
    if let RelationKind::View { dependencies, .. } = &mut selected.kind {
        dependencies.push("old_name".into());
    }
    let catalog = Catalog::from_relations([
        view(
            "answer",
            "SELECT selected.id FROM selected JOIN allowed ON selected.id = allowed.id",
        )
        .with_description("Allowed IDs greater than one")
        .with_grain("One row per ID"),
        selected,
        base("ids")
            .with_description("Customer IDs")
            .with_owner("internal-team"),
        Relation::base("allowed", schema(), "selection-source"),
    ])
    .unwrap();
    let engine = Engine::from_catalog(catalog, &backend).await.unwrap();
    assert_eq!(*backend.calls.lock().unwrap(), vec!["allowed", "ids"]);
    engine
}

#[tokio::test]
async fn embeds_custom_catalog_backend_and_nested_views() {
    let engine = fixture().await;
    let batches = engine.query("SELECT id FROM answer").await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    assert_eq!(
        batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0),
        3
    );
    let selected = engine.catalog().relation("selected").unwrap();
    assert!(
        matches!(&selected.kind, RelationKind::View { dependencies, .. } if dependencies == &["ids"])
    );
    let answer = engine.catalog().relation("answer").unwrap();
    assert_eq!(
        answer.description.as_deref(),
        Some("Allowed IDs greater than one")
    );
    assert_eq!(answer.grain.as_deref(), Some("One row per ID"));
    assert!(
        matches!(&answer.kind, RelationKind::View { dependencies, .. } if dependencies == &["allowed", "selected"])
    );
    assert_eq!(
        engine.catalog().relation("ids").unwrap().owner.as_deref(),
        Some("internal-team")
    );
    assert!(
        engine
            .plan_generated_sql("SELECT id FROM answer")
            .await
            .is_ok()
    );
    assert!(
        engine
            .plan_generated_sql("SELECT * FROM information_schema.tables")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rejects_invalid_catalogs_before_calling_backend() {
    let backend = Backend::default();
    let error = Engine::from_catalog([base("ids"), base("ids")], &backend)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(error, EngineError::Catalog(CatalogError::DuplicateRelation(name)) if name == "ids")
    );
    let error = Engine::from_catalog([base("Invalid.Name")], &backend)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, EngineError::InvalidName(_)));
    let error = Engine::from_catalog(
        [
            base("ids"),
            view("v", "SELECT (SELECT id FROM absent) AS id"),
        ],
        &backend,
    )
    .await
    .err()
    .unwrap();
    assert!(
        matches!(error, EngineError::MissingDependency { view, dependency } if view == "v" && dependency == "absent")
    );
    for relations in [
        vec![
            base("ids"),
            view("a", "SELECT * FROM b"),
            view("b", "SELECT * FROM a"),
        ],
        vec![base("ids"), view("a", "SELECT * FROM a")],
    ] {
        assert!(matches!(
            Engine::from_catalog(relations, &backend).await,
            Err(EngineError::CyclicViews(_))
        ));
    }
    for sql in [
        "DROP TABLE ids",
        "SELECT * FROM ids; SELECT 1",
        "SELECT * FROM information_schema.tables",
        "SELECT * FROM public.ids",
    ] {
        assert!(
            Engine::from_catalog([base("ids"), view("v", sql)], &backend)
                .await
                .is_err(),
            "accepted {sql}"
        );
    }
    assert!(backend.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn checks_base_and_view_schema_contracts() {
    let backend = Backend {
        providers: BTreeMap::from([("private-source".into(), table(vec![1]))]),
        ..Default::default()
    };
    let mismatched = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
    let error = Engine::from_catalog(
        [Relation::base("ids", mismatched, "private-source")],
        &backend,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, EngineError::SchemaMismatch { relation, .. } if relation == "ids"));
    let error = Engine::from_catalog(
        [
            base("ids"),
            view("v", "SELECT CAST(id AS VARCHAR) AS id FROM ids"),
        ],
        &backend,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, EngineError::SchemaMismatch { relation, .. } if relation == "v"));
}

#[tokio::test]
async fn failed_registration_preserves_metadata_and_executable_provider() {
    let mut engine = Engine::new();
    let nullable = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
    assert!(matches!(
        engine.register_table(Relation::base("ids", nullable, "source"), table(vec![1])),
        Err(EngineError::SchemaMismatch { .. })
    ));
    assert!(engine.catalog().relation("ids").is_none());
    assert!(engine.query("SELECT * FROM ids").await.is_err());
    engine.register_table(base("ids"), table(vec![1])).unwrap();
    assert!(engine.register_table(base("ids"), table(vec![2])).is_err());
    assert!(matches!(
        engine.register_table(view("v", "SELECT * FROM ids"), table(vec![1])),
        Err(EngineError::ExpectedBaseRelation(_))
    ));
    let batches = engine.query("SELECT * FROM ids").await.unwrap();
    assert_eq!(
        batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0),
        1
    );
    assert_eq!(engine.catalog().relations().count(), 1);
}

#[tokio::test]
async fn backend_errors_identify_the_relation_and_preserve_the_cause() {
    let error = Engine::from_catalog([base("ids")], &Backend::default())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(error, EngineError::Resolution { relation, source: DataFusionError::Plan(message) } if relation == "ids" && message == "source unavailable")
    );
}

#[tokio::test]
async fn empty_catalogs_and_constant_views_need_no_backend() {
    let backend = Backend::default();
    let engine = Engine::from_catalog([], &backend).await.unwrap();
    assert_eq!(engine.catalog().relations().count(), 0);
    let engine = Engine::from_catalog([view("constant", "SELECT 1 AS id")], &backend)
        .await
        .unwrap();
    assert_eq!(
        engine.query("SELECT * FROM constant").await.unwrap()[0].num_rows(),
        1
    );
    assert!(backend.calls.lock().unwrap().is_empty());
}

#[cfg(feature = "compiler")]
#[tokio::test]
async fn compiler_uses_team_metadata_and_validates_custom_relations() {
    use semantic_db::{
        Compiler, GroundingOutcome,
        compiler::provider::{Message, ModelProvider, ProviderError},
    };

    struct Model;
    impl ModelProvider for Model {
        async fn complete(
            &self,
            messages: &[Message],
        ) -> std::result::Result<String, ProviderError> {
            let context: serde_json::Value = serde_json::from_str(&messages[1].content).unwrap();
            let answer = context["catalog"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"] == "answer")
                .unwrap();
            assert_eq!(answer["description"], "Allowed IDs greater than one");
            assert_eq!(answer["grain"], "One row per ID");
            assert!(!messages[1].content.contains("private-source"));
            assert!(!messages[1].content.contains("internal-team"));
            Ok(serde_json::json!({"status": "grounded", "query": {
                "sql": "SELECT id FROM answer",
                "evidence": [{"phrase": "allowed IDs greater than one", "catalog_reference": "answer", "interpretation": "Use the team's curated view"}]
            }}).to_string())
        }
    }
    let result = Compiler::new(Model)
        .compile(&fixture().await, "Show allowed IDs greater than one")
        .await
        .unwrap();
    assert_eq!(result.attempts, 1);
    assert!(matches!(result.outcome, GroundingOutcome::Grounded { .. }));
}
