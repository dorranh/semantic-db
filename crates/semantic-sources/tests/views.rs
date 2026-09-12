use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use futures::future::BoxFuture;
use semantic_ossie::OssieDocument;
use semantic_sources::{
    ConnectorFactory, Options, Project, Registry, Result, SecretResolver, SourceConnection,
};
use serde_json::{Value, json};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "semantic-views-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("views")).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, sql: &str) {
        std::fs::write(self.0.join("views").join(name), sql).unwrap();
    }
    fn project(&self, config: Value) -> Result<Project> {
        Project::new(
            serde_json::from_value(config).unwrap(),
            OssieDocument::parse(include_str!(
                "../../../examples/geospatial/wells.ossie.yaml"
            ))
            .unwrap(),
            self.0.clone(),
        )
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn config(views: Value) -> Value {
    json!({"ossie":"unused.yaml", "connections":{"local":{"connector":"csv"}},
        "sources":{"fixtures.geospatial.wells":{"connection":"local", "path":concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/geospatial/wells.csv")}}, "views":views})
}

#[tokio::test]
async fn nested_views_keep_descriptions_and_cached_sql_until_project_reload() {
    let files = Files::new();
    files.write(
        "deep.sql",
        "SELECT * FROM wells WHERE total_depth_m >= 2500",
    );
    files.write("active.sql", "WITH selected AS (SELECT * FROM z_deep) SELECT well_id FROM selected WHERE status = 'active' AND basin = 'North Basin'");
    let config = config(json!({
        "a_active":{"description":"Active deep wells in North Basin", "sql_file":"views/active.sql"},
        "z_deep":{"description":"Example depth convention", "sql_file":"views/deep.sql"}
    }));
    let project = files.project(config.clone()).unwrap();
    let inspection = project.inspect_project(&Registry::standard()).unwrap();
    assert_eq!(
        inspection
            .views
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["z_deep", "a_active"]
    );
    assert_eq!(inspection.views[0].dependencies, ["wells"]);
    assert_eq!(inspection.views[1].dependencies, ["z_deep"]);
    // Inspection and loading use the same SQL captured when the project is read.
    files.write(
        "deep.sql",
        "SELECT * FROM wells WHERE total_depth_m >= 4000",
    );
    let loaded = project
        .load(&Registry::standard(), &|_| panic!("CSV needs no secrets"))
        .await
        .unwrap();
    assert_eq!(
        loaded
            .engine
            .catalog()
            .relation("a_active")
            .unwrap()
            .description
            .as_deref(),
        Some("Active deep wells in North Basin")
    );
    let rows = loaded
        .engine
        .query("SELECT well_id FROM a_active ORDER BY well_id")
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|batch| batch.num_rows()).sum::<usize>(), 2);
    let reloaded = files
        .project(config)
        .unwrap()
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    let rows = reloaded
        .engine
        .query("SELECT * FROM a_active")
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|batch| batch.num_rows()).sum::<usize>(), 0);
}

struct NoConnect;
impl ConnectorFactory for NoConnect {
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
        Box::pin(async { panic!("invalid views must fail before constructing a connection") })
    }
}

#[tokio::test]
async fn invalid_definitions_fail_offline_before_connections_or_secrets() {
    let files = Files::new();
    let mut registry = Registry::new();
    registry.register("offline", NoConnect).unwrap();
    for (name, sql, other, code) in [
        (
            "bad",
            "SELECT * FROM absent",
            None,
            "missing_view_dependency",
        ),
        ("bad", "SELECT * FROM bad", None, "cyclic_views"),
        (
            "bad",
            "SELECT * FROM other",
            Some("SELECT * FROM bad"),
            "cyclic_views",
        ),
        ("wells", "SELECT * FROM wells", None, "duplicate_relation"),
        ("BadName", "SELECT * FROM wells", None, "view_definition"),
        ("bad", "SELECT FROM", None, "view_definition"),
        ("bad", "", None, "view_definition"),
        ("bad", "DROP TABLE wells", None, "view_definition"),
        (
            "bad",
            "SELECT * FROM wells; SELECT 1",
            None,
            "view_definition",
        ),
        ("bad", "SELECT * FROM public.wells", None, "view_definition"),
    ] {
        files.write("bad.sql", sql);
        let mut views = json!({name: {"sql_file":"views/bad.sql"}});
        if let Some(other) = other {
            files.write("other.sql", other);
            views["other"] = json!({"sql_file":"views/other.sql"});
        }
        let mut config = config(views);
        config["connections"]["local"]["connector"] = json!("offline");
        let project = files.project(config).unwrap();
        for error in [
            project.inspect(&registry).unwrap_err().to_string(),
            project
                .load(&registry, &|_| panic!("must not resolve secrets"))
                .await
                .err()
                .unwrap()
                .to_string(),
        ] {
            assert!(
                error.contains(code) && error.contains("/views"),
                "{sql}: {error}"
            );
        }
    }
}

#[tokio::test]
async fn column_checks_require_source_schemas_but_loading_does_not_execute_views() {
    let files = Files::new();
    files.write("bad.sql", "SELECT nonexistent FROM wells");
    let project = files
        .project(config(json!({"bad":{"sql_file":"views/bad.sql"}})))
        .unwrap();
    project.inspect(&Registry::standard()).unwrap();
    let error = project
        .load(&Registry::standard(), &|_| None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("view_plan")
            && error.contains("/views/bad/sql_file")
            && error.contains("nonexistent")
    );
    files.write(
        "bad.sql",
        "SELECT CAST('not-an-integer' AS INT) AS value FROM wells",
    );
    let loaded = files
        .project(config(json!({"bad":{"sql_file":"views/bad.sql"}})))
        .unwrap()
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    assert!(loaded.engine.query("SELECT * FROM bad").await.is_err());
}

#[test]
fn rejects_missing_files_empty_paths_and_unknown_view_options() {
    let files = Files::new();
    for (path, code) in [("views/missing.sql", "view_read"), ("", "view_file")] {
        let error = files
            .project(config(json!({"bad":{"sql_file":path}})))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(code) && error.contains("/views/bad/sql_file"));
    }
    for options in [
        json!({"sql":"SELECT 1"}),
        json!({"sql_file":"x.sql", "typo":true}),
    ] {
        assert!(
            serde_json::from_value::<semantic_sources::ProjectConfig>(config(
                json!({"bad":options})
            ))
            .is_err()
        );
    }
}
