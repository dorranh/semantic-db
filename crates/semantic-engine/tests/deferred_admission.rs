use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

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
use tokio::sync::{Notify, Semaphore};

struct BlockingBackend {
    calls: Mutex<BTreeMap<String, usize>>,
    started: Notify,
    release: Semaphore,
    fail_first: AtomicBool,
}
impl BlockingBackend {
    fn new(fail_first: bool) -> Self {
        Self {
            calls: Mutex::new(BTreeMap::new()),
            started: Notify::new(),
            release: Semaphore::new(0),
            fail_first: AtomicBool::new(fail_first),
        }
    }
    fn calls(&self, name: &str) -> usize {
        *self.calls.lock().unwrap().get(name).unwrap_or(&0)
    }
}
impl RelationBackend for BlockingBackend {
    async fn resolve(
        &self,
        relation: &Relation,
    ) -> datafusion::error::Result<Arc<dyn TableProvider>> {
        *self
            .calls
            .lock()
            .unwrap()
            .entry(relation.name.clone())
            .or_default() += 1;
        self.started.notify_one();
        self.release.acquire().await.unwrap().forget();
        if self.fail_first.swap(false, Ordering::SeqCst) {
            return Err(datafusion::error::DataFusionError::Execution(
                "controlled resolution failure".into(),
            ));
        }
        let batch = RecordBatch::try_new(
            relation.schema.clone(),
            vec![Arc::new(Int64Array::from(vec![7]))],
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
fn options() -> DeferredOptions {
    DeferredOptions {
        max_cached_providers: 1,
        max_concurrent_resolutions: 1,
        max_active_cold_keys: 1,
    }
}
async fn setup(backend: Arc<BlockingBackend>) -> (Arc<Engine>, DeferredBackend<BlockingBackend>) {
    let deferred = DeferredBackend::new(backend, options()).unwrap();
    let engine = Arc::new(
        Engine::from_catalog([relation("a"), relation("b")], &deferred)
            .await
            .unwrap(),
    );
    (engine, deferred)
}
async fn started(backend: &BlockingBackend) {
    tokio::time::timeout(Duration::from_secs(2), backend.started.notified())
        .await
        .expect("backend resolution should start");
}

#[tokio::test]
async fn distinct_cold_table_is_rejected_while_same_table_follower_coalesces() {
    let backend = Arc::new(BlockingBackend::new(false));
    let (engine, deferred) = setup(backend.clone()).await;
    let leader_engine = engine.clone();
    let leader = tokio::spawn(async move { leader_engine.query("SELECT id FROM a").await });
    started(&backend).await;
    assert_eq!(deferred.report().active_cold_keys, 1);

    let follower_engine = engine.clone();
    let follower = tokio::spawn(async move { follower_engine.query("SELECT id FROM a").await });
    let second = tokio::time::timeout(Duration::from_secs(2), engine.query("SELECT id FROM b"))
        .await
        .expect("overload must reject promptly")
        .unwrap_err();
    assert!(
        second
            .to_string()
            .contains("deferred provider admission exhausted")
    );
    assert!(!follower.is_finished());
    assert_eq!(deferred.report().admission_rejections, 1);
    assert_eq!(deferred.report().peak_active_cold_keys, 1);

    backend.release.add_permits(1);
    leader.await.unwrap().unwrap();
    follower.await.unwrap().unwrap();
    assert_eq!(backend.calls("a"), 1);
    assert_eq!(deferred.report().active_cold_keys, 0);

    backend.release.add_permits(1);
    engine.query("SELECT id FROM b").await.unwrap();
    assert_eq!(backend.calls("b"), 1);
    assert_eq!(deferred.report().evictions, 1);
}

#[tokio::test]
async fn aborted_leader_releases_admission_and_same_table_follower_retries() {
    let backend = Arc::new(BlockingBackend::new(false));
    let (engine, deferred) = setup(backend.clone()).await;
    let leader_engine = engine.clone();
    let leader = tokio::spawn(async move { leader_engine.query("SELECT id FROM a").await });
    started(&backend).await;
    let follower_engine = engine.clone();
    let follower = tokio::spawn(async move { follower_engine.query("SELECT id FROM a").await });
    leader.abort();
    assert!(leader.await.unwrap_err().is_cancelled());
    started(&backend).await;
    assert_eq!(backend.calls("a"), 2);
    assert_eq!(deferred.report().active_cold_keys, 1);
    backend.release.add_permits(1);
    follower.await.unwrap().unwrap();
    assert_eq!(deferred.report().active_cold_keys, 0);
    assert_eq!(deferred.report().cached_providers, 1);
}

#[tokio::test]
async fn failure_retry_and_eviction_leave_no_active_key_leak() {
    let backend = Arc::new(BlockingBackend::new(true));
    let (engine, deferred) = setup(backend.clone()).await;
    backend.release.add_permits(4);
    let error = engine.query("SELECT id FROM a").await.unwrap_err();
    assert!(error.to_string().contains("controlled resolution failure"));
    assert_eq!(deferred.report().active_cold_keys, 0);
    assert_eq!(deferred.report().cached_providers, 0);

    engine.query("SELECT id FROM a").await.unwrap();
    engine.query("SELECT id FROM b").await.unwrap();
    engine.query("SELECT id FROM a").await.unwrap();
    assert_eq!(backend.calls("a"), 3);
    assert_eq!(backend.calls("b"), 1);
    assert_eq!(deferred.report().evictions, 2);
    assert_eq!(deferred.report().active_cold_keys, 0);
    assert_eq!(deferred.report().peak_active_cold_keys, 1);
}
