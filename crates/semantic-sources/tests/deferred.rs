use arrow_schema::{DataType, Field, Schema};
use datafusion::{
    arrow::{array::Int64Array, record_batch::RecordBatch},
    catalog::TableProvider,
    datasource::MemTable,
};
use futures::future::BoxFuture;
use semantic_ossie::OssieDocument;
use semantic_sources::{
    ConnectorFactory, Options, Project, Registry, Result, SecretResolver, SourceConnection,
    SourceError,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

struct Factory {
    connects: Arc<AtomicUsize>,
    tables: Arc<AtomicUsize>,
    drift: bool,
}
struct Connection {
    tables: Arc<AtomicUsize>,
    drift: bool,
}
impl ConnectorFactory for Factory {
    fn validate_connection(&self, _: &Options) -> Result<()> {
        Ok(())
    }
    fn validate_source(&self, _: &Options) -> Result<()> {
        Ok(())
    }
    fn connect<'a>(
        &'a self,
        _: &'a Options,
        _: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(async move {
            self.connects.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Connection {
                tables: self.tables.clone(),
                drift: self.drift,
            }) as Arc<dyn SourceConnection>)
        })
    }
}
impl SourceConnection for Connection {
    fn table<'a>(
        &'a self,
        options: &'a Options,
        _: &'a Path,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            self.tables.fetch_add(1, Ordering::SeqCst);
            let column = if self.drift {
                "changed"
            } else {
                options["column"].as_str().unwrap()
            };
            let schema = Arc::new(Schema::new(vec![Field::new(
                column,
                DataType::Int64,
                false,
            )]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(Int64Array::from(vec![
                    options["value"].as_i64().unwrap(),
                ]))],
            )
            .map_err(datafusion::error::DataFusionError::from)?;
            Ok(Arc::new(MemTable::try_new(schema, vec![vec![batch]])?) as Arc<dyn TableProvider>)
        })
    }
}

fn recorded(column: &str) -> Value {
    serde_json::to_value(Schema::new(vec![Field::new(
        column,
        DataType::Int64,
        false,
    )]))
    .unwrap()
}

fn project() -> Project {
    let document = OssieDocument::parse(
        &json!({"version":"0.2.0.dev0","semantic_model":[{
            "name":"fixture","datasets":[{"name":"items","source":"fixture.items",
                "fields":[{"name":"id","datatype":"Integer",
                    "expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"physical_id"}]}}]}]
        }]})
        .to_string(),
    )
    .unwrap();
    let base_dir = std::env::temp_dir().join(format!(
        "semantic-deferred-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::write(base_dir.join("item_view.sql"), "SELECT id FROM items").unwrap();
    std::fs::write(base_dir.join("nested_view.sql"), "SELECT id FROM item_view").unwrap();
    let config = json!({
        "ossie":"unused.yaml",
        "connections":{"used":{"connector":"fixture"},"unused":{"connector":"fixture"}},
        "sources":{
            "fixture.items":{"connection":"used","column":"physical_id","value":7,
                "recorded_schema":recorded("physical_id")},
            "fixture.unused":{"connection":"unused","column":"other_id","value":9,
                "recorded_schema":recorded("other_id")}
        },
        "app_tables":{"extra":{"connection":"used","column":"id","value":11,
            "recorded_schema":recorded("id")}},
        "views":{
            "item_view":{"sql_file":"item_view.sql"},
            "nested_view":{"sql_file":"nested_view.sql"}
        }
    });
    Project::new(
        serde_json::from_value(config).unwrap(),
        document,
        PathBuf::from(base_dir),
    )
    .unwrap()
}

fn registry(drift: bool) -> (Arc<Registry>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let connects = Arc::new(AtomicUsize::new(0));
    let tables = Arc::new(AtomicUsize::new(0));
    let mut registry = Registry::new();
    registry
        .register(
            "fixture",
            Factory {
                connects: connects.clone(),
                tables: tables.clone(),
                drift,
            },
        )
        .unwrap();
    (Arc::new(registry), connects, tables)
}

#[tokio::test]
async fn project_defers_unrelated_sources_and_loads_views_and_app_tables() {
    let (registry, connects, tables) = registry(false);
    let loaded = project()
        .load_deferred_read_only(
            registry,
            Arc::new(|_| None),
            semantic_engine::DeferredOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(connects.load(Ordering::SeqCst), 0);
    assert_eq!(tables.load(Ordering::SeqCst), 0);
    assert_eq!(loaded.provider_report().resolutions, 0);
    let rows = loaded
        .engine
        .query("SELECT id FROM item_view")
        .await
        .unwrap();
    assert_eq!(rows.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
    assert_eq!(connects.load(Ordering::SeqCst), 1);
    assert_eq!(tables.load(Ordering::SeqCst), 1);
    assert_eq!(loaded.provider_report().resolutions, 1);
    let rows = loaded.engine.query("SELECT id FROM extra").await.unwrap();
    assert_eq!(rows.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
    assert_eq!(connects.load(Ordering::SeqCst), 1);
    assert_eq!(tables.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn physical_schema_drift_fails_before_rows_execute() {
    let (registry, connects, tables) = registry(true);
    let loaded = project()
        .load_deferred_read_only(
            registry,
            Arc::new(|_| None),
            semantic_engine::DeferredOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(connects.load(Ordering::SeqCst), 0);
    let error = loaded
        .engine
        .query("SELECT id FROM items")
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("physical schema drift"));
    assert_eq!(tables.load(Ordering::SeqCst), 1);
    assert_eq!(loaded.provider_report().cached_providers, 0);
}

#[tokio::test]
async fn missing_recorded_schema_fails_offline() {
    let (registry, connects, _) = registry(false);
    let path = std::env::temp_dir().join(format!(
        "semantic-deferred-missing-{}.yaml",
        std::process::id()
    ));
    std::fs::write(
        &path,
        serde_json::to_string(&json!({
            "connections":{"used":{"connector":"fixture"}},
            "app_tables":{"extra":{"connection":"used","column":"id","value":1}}
        }))
        .unwrap(),
    )
    .unwrap();
    let loaded = Project::from_path(&path)
        .unwrap()
        .load_deferred_read_only(
            registry,
            Arc::new(|_| None),
            semantic_engine::DeferredOptions::default(),
        )
        .await;
    assert!(
        loaded
            .err()
            .unwrap()
            .to_string()
            .contains("missing_recorded_schema")
    );
    assert_eq!(connects.load(Ordering::SeqCst), 0);
}

#[derive(Clone)]
struct Probe {
    connects: Arc<AtomicUsize>,
    attempts: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    fail_once: Arc<AtomicBool>,
    started: Arc<Notify>,
    delay: Duration,
}
impl Probe {
    fn new(delay: Duration, fail_once: bool) -> Self {
        Self {
            connects: Arc::new(AtomicUsize::new(0)),
            attempts: Arc::new(AtomicUsize::new(0)),
            active: Arc::new(AtomicUsize::new(0)),
            peak: Arc::new(AtomicUsize::new(0)),
            fail_once: Arc::new(AtomicBool::new(fail_once)),
            started: Arc::new(Notify::new()),
            delay,
        }
    }
}
struct Activity(Arc<AtomicUsize>);
impl Drop for Activity {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
struct ControlledFactory(Probe);
struct ControlledConnection(Probe);
impl ConnectorFactory for ControlledFactory {
    fn validate_connection(&self, _: &Options) -> Result<()> {
        Ok(())
    }
    fn validate_source(&self, _: &Options) -> Result<()> {
        Ok(())
    }
    fn connect<'a>(
        &'a self,
        _: &'a Options,
        _: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(async move {
            self.0.connects.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(ControlledConnection(self.0.clone())) as Arc<dyn SourceConnection>)
        })
    }
}
impl SourceConnection for ControlledConnection {
    fn table<'a>(
        &'a self,
        options: &'a Options,
        _: &'a Path,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            self.0.attempts.fetch_add(1, Ordering::SeqCst);
            let active = self.0.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.0.peak.fetch_max(active, Ordering::SeqCst);
            let _activity = Activity(self.0.active.clone());
            self.0.started.notify_one();
            tokio::time::sleep(self.0.delay).await;
            if self.0.fail_once.swap(false, Ordering::SeqCst) {
                return Err(SourceError::configuration(
                    "fixture_failure",
                    "/fixture",
                    "one controlled metadata failure",
                ));
            }
            let column = options["column"].as_str().unwrap();
            let schema = Arc::new(Schema::new(vec![Field::new(
                column,
                DataType::Int64,
                false,
            )]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(Int64Array::from(vec![
                    options["value"].as_i64().unwrap(),
                ]))],
            )
            .map_err(datafusion::error::DataFusionError::from)?;
            Ok(Arc::new(MemTable::try_new(schema, vec![vec![batch]])?) as Arc<dyn TableProvider>)
        })
    }
}

fn controlled_registry(probe: &Probe) -> Arc<Registry> {
    let mut registry = Registry::new();
    registry
        .register("fixture", ControlledFactory(probe.clone()))
        .unwrap();
    Arc::new(registry)
}

async fn controlled_project(
    probe: &Probe,
    options: semantic_engine::DeferredOptions,
) -> semantic_sources::DeferredProject {
    project()
        .load_deferred_read_only(controlled_registry(probe), Arc::new(|_| None), options)
        .await
        .unwrap()
}

#[tokio::test]
async fn concurrent_cold_scans_share_selected_nested_view_resolution() {
    let probe = Probe::new(Duration::from_millis(40), false);
    let loaded = controlled_project(&probe, semantic_engine::DeferredOptions::default()).await;
    let (left, right) = tokio::join!(
        loaded.engine.query("SELECT id FROM nested_view"),
        loaded.engine.query("SELECT id FROM nested_view")
    );
    assert!(left.is_ok() && right.is_ok());
    assert_eq!(probe.connects.load(Ordering::SeqCst), 1);
    assert_eq!(probe.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(loaded.provider_report().resolutions, 1);
}

#[tokio::test]
async fn cancelled_leader_releases_cold_resolution_for_retry() {
    let probe = Probe::new(Duration::from_millis(80), false);
    let loaded = controlled_project(&probe, semantic_engine::DeferredOptions::default()).await;
    let started = probe.started.notified();
    tokio::pin!(started);
    {
        let leader = loaded.engine.query("SELECT id FROM items");
        tokio::pin!(leader);
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                _ = &mut started => {},
                result = &mut leader => panic!("cold scan unexpectedly completed: {result:?}"),
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(probe.active.load(Ordering::SeqCst), 0);
    assert!(loaded.engine.query("SELECT id FROM items").await.is_ok());
    assert_eq!(probe.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(loaded.provider_report().cached_providers, 1);
}

#[tokio::test]
async fn failed_metadata_resolution_retries_and_bounded_cache_evicts() {
    let probe = Probe::new(Duration::from_millis(10), true);
    let loaded = controlled_project(
        &probe,
        semantic_engine::DeferredOptions {
            max_cached_providers: 1,
            max_concurrent_resolutions: 1,
            ..semantic_engine::DeferredOptions::default()
        },
    )
    .await;
    assert!(loaded.engine.query("SELECT id FROM items").await.is_err());
    assert_eq!(loaded.provider_report().cached_providers, 0);
    assert!(loaded.engine.query("SELECT id FROM items").await.is_ok());
    assert!(loaded.engine.query("SELECT id FROM extra").await.is_ok());
    assert!(loaded.engine.query("SELECT id FROM items").await.is_ok());
    assert_eq!(probe.attempts.load(Ordering::SeqCst), 4);
    assert_eq!(probe.connects.load(Ordering::SeqCst), 1);
    assert_eq!(loaded.provider_report().cached_providers, 1);
    assert_eq!(loaded.provider_report().evictions, 2);
    assert_eq!(probe.peak.load(Ordering::SeqCst), 1);
}
