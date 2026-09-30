use datafusion::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_engine::{DeferredBackend, DeferredOptions, Engine, RelationBackend, TableProvider};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
struct Backend {
    names: Mutex<Vec<String>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    drift: bool,
}
impl Backend {
    fn new(drift: bool) -> Self {
        Self {
            names: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            drift,
        }
    }
}
impl RelationBackend for Backend {
    async fn resolve(
        &self,
        relation: &Relation,
    ) -> datafusion::error::Result<Arc<dyn TableProvider>> {
        self.names.lock().unwrap().push(relation.name.clone());
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.drift {
            return Ok(Arc::new(MemTable::try_new(
                Arc::new(Schema::new(vec![Field::new(
                    "other",
                    DataType::Int64,
                    false,
                )])),
                vec![vec![]],
            )?));
        }
        let batch = RecordBatch::try_new(
            relation.schema.clone(),
            vec![Arc::new(Int64Array::from(vec![7, 9]))],
        )?;
        Ok(Arc::new(MemTable::try_new(
            relation.schema.clone(),
            vec![vec![batch]],
        )?))
    }
}
fn relation(name: &str) -> Relation {
    Relation::base(
        name,
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
        format!("source:{name}"),
    )
}
#[tokio::test]
async fn publication_and_planning_do_not_resolve_unselected_sources() {
    let backend = Arc::new(Backend::new(false));
    let deferred = DeferredBackend::new(backend.clone(), DeferredOptions::default()).unwrap();
    let view = Relation::view(
        "selected",
        relation("a").schema,
        "SELECT id FROM a WHERE id > 7",
    );
    let engine = Engine::from_catalog([relation("a"), relation("b"), view], &deferred)
        .await
        .unwrap();
    assert_eq!(deferred.report().resolutions, 0);
    engine
        .plan_generated_sql("SELECT id FROM selected")
        .await
        .unwrap();
    assert_eq!(deferred.report().resolutions, 0);
    let result = engine.query("SELECT id FROM selected").await.unwrap();
    assert_eq!(result[0].num_rows(), 1);
    assert_eq!(*backend.names.lock().unwrap(), vec!["a"]);
    engine.query("SELECT id FROM selected").await.unwrap();
    assert_eq!(deferred.report().resolutions, 1);
    assert!(deferred.report().cache_hits > 0);
}
#[tokio::test]
async fn cache_eviction_and_resolution_concurrency_are_bounded() {
    let backend = Arc::new(Backend::new(false));
    let deferred = DeferredBackend::new(
        backend.clone(),
        DeferredOptions {
            max_cached_providers: 1,
            max_concurrent_resolutions: 1,
            ..DeferredOptions::default()
        },
    )
    .unwrap();
    let engine = Engine::from_catalog([relation("a"), relation("b")], &deferred)
        .await
        .unwrap();
    engine.query("SELECT id FROM a").await.unwrap();
    engine.query("SELECT id FROM b").await.unwrap();
    engine.query("SELECT id FROM a").await.unwrap();
    assert_eq!(deferred.report().resolutions, 3);
    assert_eq!(deferred.report().evictions, 2);
    let (a, b) = tokio::join!(
        engine.query("SELECT id FROM a"),
        engine.query("SELECT id FROM b")
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(backend.peak.load(Ordering::SeqCst), 1);
    assert_eq!(deferred.report().cached_providers, 1);
}
#[tokio::test]
async fn schema_drift_fails_before_rows_and_failed_resolutions_are_not_cached() {
    let backend = Arc::new(Backend::new(true));
    let deferred = DeferredBackend::new(backend, DeferredOptions::default()).unwrap();
    let engine = Engine::from_catalog([relation("a")], &deferred)
        .await
        .unwrap();
    for _ in 0..2 {
        let error = engine.query("SELECT id FROM a").await.unwrap_err();
        assert!(error.to_string().contains("schema drift"));
    }
    assert_eq!(deferred.report().cached_providers, 0);
    assert_eq!(deferred.report().resolutions, 2);
}
#[tokio::test]
async fn view_cycles_and_type_contracts_still_fail_at_publication_without_io() {
    let backend = Arc::new(Backend::new(false));
    let deferred = DeferredBackend::new(backend, DeferredOptions::default()).unwrap();
    let a = Relation::view("a", relation("a").schema, "SELECT id FROM b");
    let b = Relation::view("b", relation("b").schema, "SELECT id FROM a");
    assert!(Engine::from_catalog([a, b], &deferred).await.is_err());
    let invalid = Relation::view(
        "invalid",
        relation("invalid").schema,
        "SELECT unknown_field AS id FROM a",
    );
    assert!(
        Engine::from_catalog([relation("a"), invalid], &deferred)
            .await
            .is_err()
    );
    assert_eq!(deferred.report().resolutions, 0);
}
