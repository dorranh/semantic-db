use datafusion::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
    },
    common::ScalarValue,
    datasource::MemTable,
};
use futures::future::BoxFuture;
use semantic_engine::*;
use semantic_runtime::staging::StagedInput;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct State {
    value: AtomicI64,
    opens: AtomicUsize,
    binds: AtomicUsize,
    closes: AtomicUsize,
    drops: AtomicUsize,
    stall: AtomicBool,
}
struct Connection(Arc<State>, &'static str);
struct Session {
    state: Arc<State>,
    domain: String,
    value: i64,
}
fn provider(value: i64) -> Arc<dyn TableProvider> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int64Array::from(vec![value]))],
    )
    .unwrap();
    Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap())
}
impl ReadConnection for Connection {
    fn domain(&self) -> String {
        self.1.into()
    }
    fn read_provider<'a>(
        &'a self,
        _: &'a str,
        receipts: &'a [CommitReceipt],
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            if !receipts.is_empty() {
                return Err(semantic_runtime::failure("fake receipts unsupported").into());
            }
            Ok(provider(self.0.value.load(Ordering::SeqCst)))
        })
    }
    fn open_read_session<'a>(
        &'a self,
        _: ReadSessionOptions,
    ) -> BoxFuture<'a, Result<Arc<dyn ConnectorSession>>> {
        Box::pin(async move {
            self.0.opens.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Session {
                state: self.0.clone(),
                domain: self.domain(),
                value: self.0.value.load(Ordering::SeqCst),
            }) as Arc<dyn ConnectorSession>)
        })
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.state.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl ConnectorSession for Session {
    fn domain(&self) -> String {
        self.domain.clone()
    }
    fn snapshot(&self) -> String {
        format!("snapshot-{}", self.value)
    }
    fn read_provider<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            self.state.binds.fetch_add(1, Ordering::SeqCst);
            if self.state.stall.load(Ordering::SeqCst) {
                futures::future::pending::<()>().await;
            }
            Ok(provider(self.value))
        })
    }
    fn apply<'a>(
        &'a self,
        _: &'a MutationPlan,
        _: &'a StagedInput,
        _: &'a [ScalarValue],
        _: Arc<QueryContext>,
    ) -> BoxFuture<'a, Result<WriteResult>> {
        Box::pin(async { Err(semantic_runtime::failure("read only").into()) })
    }
    fn finish<'a>(&'a self, commit: bool) -> BoxFuture<'a, Result<WriteResult>> {
        Box::pin(async move {
            assert!(!commit);
            self.state.closes.fetch_add(1, Ordering::SeqCst);
            Ok(WriteResult {
                outcome: WriteOutcome::Aborted,
                operation_id: "fake".into(),
                boundary: self.domain(),
                atomic: true,
                idempotent: false,
                receipt: None,
                affected_rows: None,
                code: None,
                message: None,
                external_observations: vec![],
            })
        })
    }
}
async fn setup() -> (Engine, Arc<State>) {
    let mut engine = Engine::new();
    let state = Arc::new(State::default());
    for (name, domain) in [("items", "one"), ("alias", "one"), ("external", "two")] {
        let p = provider(0);
        engine
            .register_table(Relation::base(name, p.schema(), "fake"), p)
            .unwrap();
        engine
            .attach_read_binding(
                name,
                ReadBinding {
                    connection: Arc::new(Connection(state.clone(), domain)),
                    resource: "physical".into(),
                    columns: BTreeMap::from([("value".into(), "value".into())]),
                },
            )
            .unwrap();
    }
    engine
        .create_view(
            "expanded",
            "SELECT a.value FROM items a JOIN alias b ON a.value=b.value",
        )
        .await
        .unwrap();
    (engine, state)
}
fn value(batches: &[RecordBatch]) -> i64 {
    batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}
#[tokio::test]
async fn snapshots_bind_expanded_views_without_replacing_shared_providers() {
    let (engine, state) = setup().await;
    state.value.store(1, Ordering::SeqCst);
    let mut session = engine
        .begin_read_session(&["expanded".into()], ReadSessionOptions::default())
        .await
        .unwrap();
    assert_eq!(state.opens.load(Ordering::SeqCst), 1);
    state.value.store(2, Ordering::SeqCst);
    let result = session
        .query("SELECT * FROM expanded", vec![], ReadOptions::default())
        .await
        .unwrap();
    assert_eq!(value(&result.batches), 1);
    assert_eq!(result.report.snapshot.as_deref(), Some("snapshot-1"));
    assert_eq!(result.report.established, ReadConsistency::Snapshot);
    assert_eq!(
        value(&engine.query("SELECT * FROM expanded").await.unwrap()),
        0
    );
    session.close().await.unwrap();
    let execution = engine
        .execute_read(
            "SELECT * FROM expanded",
            vec![],
            ReadOptions {
                consistency: ReadConsistency::Snapshot,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let report = execution.report.clone();
    let drops = state.drops.load(Ordering::SeqCst);
    drop(execution);
    assert_eq!(report.lock().unwrap().completion, ReadCompletion::Abandoned);
    assert_eq!(state.drops.load(Ordering::SeqCst), drops + 1);
}
#[tokio::test]
async fn participation_is_validated_before_any_scan_and_external_reads_are_reported() {
    let (engine, state) = setup().await;
    let sql = "SELECT a.value FROM expanded a CROSS JOIN external b";
    assert!(
        engine
            .execute_read(
                sql,
                vec![],
                ReadOptions {
                    consistency: ReadConsistency::Snapshot,
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(state.opens.load(Ordering::SeqCst), 0);
    let mut strict = engine
        .begin_read_session(&["items".into()], ReadSessionOptions::default())
        .await
        .unwrap();
    assert!(
        strict
            .query(sql, vec![], ReadOptions::default())
            .await
            .is_err()
    );
    assert_eq!(state.binds.load(Ordering::SeqCst), 0);
    let mut allowed = engine
        .begin_read_session(
            &["items".into()],
            ReadSessionOptions {
                external_reads: ExternalReads::AllowObserved,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let result = allowed
        .query(sql, vec![], ReadOptions::default())
        .await
        .unwrap();
    assert_eq!(result.report.external_observations, vec!["external"]);
    assert_eq!(result.report.established, ReadConsistency::Observed);
    assert!(
        allowed
            .query(
                "SELECT * FROM items",
                vec![],
                ReadOptions {
                    after_commits: vec![CommitReceipt {
                        domain: "one".into(),
                        evidence: "test".into(),
                        resources: vec!["physical".into()]
                    }],
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
}
#[tokio::test]
async fn cancelled_session_operation_releases_resources_and_never_reopens() {
    let (engine, state) = setup().await;
    let mut session = engine
        .begin_read_session(&["items".into()], ReadSessionOptions::default())
        .await
        .unwrap();
    state.stall.store(true, Ordering::SeqCst);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            session.query("SELECT * FROM items", vec![], ReadOptions::default())
        )
        .await
        .is_err()
    );
    tokio::task::yield_now().await;
    assert_eq!(state.closes.load(Ordering::SeqCst), 1);
    state.stall.store(false, Ordering::SeqCst);
    assert!(
        session
            .query("SELECT * FROM items", vec![], ReadOptions::default())
            .await
            .is_err()
    );
    assert_eq!(state.opens.load(Ordering::SeqCst), 1);
}
