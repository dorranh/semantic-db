use datafusion::{catalog::TableProvider, prelude::SessionContext};
use futures::future::BoxFuture;
use semantic_ossie::OssieDocument;
use semantic_sources::{
    ConnectorFactory, Options, Project, Registry, Result, SecretResolver, SourceConnection,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn document() -> OssieDocument {
    let mut datasets = vec![];
    for (name, source) in [
        ("items", "fixture.items"),
        ("same_items", "fixture.items"),
        ("others", "fixture.other"),
    ] {
        datasets.push(
            json!({"name":name,"source":source,"fields":[{"name":"id","datatype":"Integer",
            "expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"id"}]}}]}),
        );
    }
    OssieDocument::parse(
        &json!({"version":"0.2.0.dev0","semantic_model":[{"name":"fixture","datasets":datasets}]})
            .to_string(),
    )
    .unwrap()
}
fn config() -> Value {
    json!({"ossie":"unused.yaml","connections":{"test":{"connector":"fixture"}},
        "sources":{"fixture.items":{"connection":"test"}, "fixture.other":{"connection":"test"}}})
}
fn project(config: Value) -> Project {
    Project::new(
        serde_json::from_value(config).unwrap(),
        document(),
        PathBuf::from("."),
    )
    .unwrap()
}
struct Factory {
    connects: Arc<AtomicUsize>,
    tables: Arc<AtomicUsize>,
}
struct Connection {
    tables: Arc<AtomicUsize>,
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
        secrets: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(async move {
            assert_eq!(secrets("fixture_key").as_deref(), Some("fixture_value"));
            self.connects.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(Connection {
                tables: self.tables.clone(),
            }) as Arc<dyn SourceConnection>)
        })
    }
}
impl SourceConnection for Connection {
    fn table<'a>(
        &'a self,
        _: &'a Options,
        _: &'a Path,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            self.tables.fetch_add(1, Ordering::Relaxed);
            Ok(SessionContext::new()
                .sql("SELECT 42 AS id")
                .await?
                .into_view())
        })
    }
}

#[tokio::test]
async fn shared_loader_reuses_connections_and_sources_and_validates_before_connecting() {
    let connects = Arc::new(AtomicUsize::new(0));
    let tables = Arc::new(AtomicUsize::new(0));
    let mut registry = Registry::new();
    registry
        .register(
            "fixture",
            Factory {
                connects: connects.clone(),
                tables: tables.clone(),
            },
        )
        .unwrap();
    let fixture = project(config());
    assert_eq!(fixture.inspect(&registry).unwrap().datasets.len(), 3);
    assert_eq!(connects.load(Ordering::Relaxed), 0);
    assert_eq!(tables.load(Ordering::Relaxed), 0);
    let mut invalid = config();
    invalid["sources"]
        .as_object_mut()
        .unwrap()
        .remove("fixture.other");
    let error = project(invalid)
        .load(&registry, &|_| panic!("must not resolve secrets"))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("missing_binding") && error.contains("fixture.other"));
    assert_eq!(connects.load(Ordering::Relaxed), 0);
    let imported = fixture
        .load(&registry, &|name| {
            (name == "fixture_key").then(|| "fixture_value".into())
        })
        .await
        .unwrap();
    assert_eq!(connects.load(Ordering::Relaxed), 1);
    assert_eq!(tables.load(Ordering::Relaxed), 2);
    let rows = imported
        .engine
        .query("SELECT a.id FROM items a JOIN others b ON a.id = b.id")
        .await
        .unwrap();
    assert_eq!(rows[0].num_rows(), 1);
}

#[test]
fn validates_registry_options_and_connection_references_offline() {
    let registry = Registry::standard();
    for (value, code) in [
        (
            json!({"ossie":"x", "connections":{"x":{"connector":"unknown"}}}),
            "unknown_connector",
        ),
        (
            json!({"ossie":"x", "connections":{"x":{"connector":"csv", "typo":true}}}),
            "unknown field",
        ),
        (
            json!({"ossie":"x", "sources":{"fixture.items":{"connection":"missing"}}}),
            "missing_connection",
        ),
        (
            json!({"ossie":"x", "connections":{"x":{"connector":"csv"}},"sources":{"fixture.items":{"connection":"x","path":""}}}),
            "CSV path must be nonempty",
        ),
    ] {
        assert!(
            project(value)
                .inspect(&registry)
                .unwrap_err()
                .to_string()
                .contains(code)
        );
    }
    let mut registry = Registry::standard();
    assert!(
        registry
            .register("csv", semantic_sources::CsvConnector)
            .is_err()
    );
}

#[tokio::test]
async fn csv_paths_are_relative_to_project_not_working_directory() {
    let project = Project::from_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/geospatial/semantic-db.yaml"
    ))
    .unwrap();
    let imported = project
        .load(&Registry::standard(), &|_| panic!("CSV needs no secrets"))
        .await
        .unwrap();
    let rows = imported
        .engine
        .query(include_str!("../../../examples/geospatial/query.sql"))
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|b| b.num_rows()).sum::<usize>(), 2);
}

#[cfg(feature = "github")]
#[tokio::test]
async fn github_project_inspects_without_secrets_and_schema_load_needs_no_http() {
    let project = Project::from_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/github/semantic-db.yaml"
    ))
    .unwrap();
    assert_eq!(
        project
            .inspect(&Registry::standard())
            .unwrap()
            .datasets
            .len(),
        3
    );
    let error = project
        .load(&Registry::standard(), &|_| None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("GITHUB_TOKEN") && error.contains("/connections/github"));
    // Construction uses fixed GitHub schemas, so this token is never sent.
    let imported = project
        .load(&Registry::standard(), &|_| Some("not-a-real-token".into()))
        .await
        .unwrap();
    imported
        .engine
        .plan_sql("SELECT * FROM issues WHERE state = 'OPEN'")
        .await
        .unwrap();
}
