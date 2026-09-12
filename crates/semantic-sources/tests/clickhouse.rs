#![cfg(feature = "clickhouse")]

use datafusion::{
    arrow::{array::RecordBatch, util::pretty::pretty_format_batches},
    catalog::TableProvider,
    prelude::SessionContext,
};
use semantic_clickhouse::{ClickHouse, ClickHouseConfig};
use semantic_engine::Engine;
use semantic_ossie::{OssieDocument, SourceBindings};
use semantic_sources::{Project, ProjectConfig, Registry, conformance::check_query_equivalence};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

const PASSWORD: &str = "fixture-password";
type TableFixture = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);
const TABLES: &[TableFixture] = &[
    (
        "samples",
        "drilling_samples",
        &[
            ("well_id", "well_id"),
            ("sample_id", "sample_id"),
            ("depth_m", "drilled_m"),
            ("duration_min", "duration_min"),
            ("load_kn", "load_kn"),
        ],
    ),
    (
        "wells",
        "wells",
        &[("well_id", "well_id"), ("basin", "basin")],
    ),
    (
        "totals",
        "drilling_totals",
        &[
            ("well_id", "well_id"),
            ("depth_m", "drilled_m"),
            ("duration_min", "duration_min"),
        ],
    ),
    (
        "summary",
        "drilling_summary",
        &[
            ("well_id", "well_id"),
            ("depth_m", "drilled_m"),
            ("duration_min", "duration_min"),
            ("mean_load_kn", "mean_load_kn"),
            ("samples", "samples"),
        ],
    ),
];

struct Database {
    _container: ContainerAsync<GenericImage>,
    endpoint: String,
    admin: clickhouse::Client,
}
impl Database {
    async fn start() -> Self {
        let container = GenericImage::new("clickhouse/clickhouse-server", "25.8.3.66")
            .with_exposed_port(8123.tcp())
            .with_wait_for(WaitFor::http(
                HttpWaitStrategy::new("/ping")
                    .with_port(8123.tcp())
                    .with_expected_status_code(200u16),
            ))
            .with_env_var("CLICKHOUSE_DB", "drilling")
            .with_env_var("CLICKHOUSE_USER", "fixture")
            .with_env_var("CLICKHOUSE_PASSWORD", PASSWORD)
            .with_env_var("CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT", "1")
            .with_startup_timeout(Duration::from_secs(90))
            .start()
            .await
            .expect("start ClickHouse (requires Docker)");
        let endpoint = format!(
            "http://{}:{}",
            container.get_host().await.unwrap(),
            container.get_host_port_ipv4(8123).await.unwrap()
        );
        let admin = clickhouse::Client::default()
            .with_url(&endpoint)
            .with_database("drilling")
            .with_user("fixture")
            .with_password(PASSWORD);
        let database = Self {
            _container: container,
            endpoint,
            admin,
        };
        // Fixture SQL is test-owned; one statement per request, no multiquery mode.
        let sql = include_str!("../../../examples/clickhouse/drilling.sql")
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        for statement in sql.split(';').filter(|sql| !sql.trim().is_empty()) {
            database.execute(statement).await;
        }
        database
    }
    async fn execute(&self, sql: &str) {
        self.admin
            .query(sql)
            .execute()
            .await
            .unwrap_or_else(|error| panic!("fixture statement failed: {sql}\n{error}"));
    }
    fn connection(&self, federation: bool) -> ClickHouse {
        let mut config = ClickHouseConfig::new(&self.endpoint, "drilling", "fixture", PASSWORD);
        config.federation = federation;
        ClickHouse::new(config).unwrap()
    }
}

fn document() -> OssieDocument {
    let datasets: Vec<Value> = TABLES
        .iter()
        .map(|(name, table, fields)| {
            let fields: Vec<Value> = fields
                .iter()
                .map(|(name, column)| {
                    json!({"name": name,
            "expression":{"dialects":[{"dialect":"ANSI_SQL","expression":column}]}})
                })
                .collect();
            json!({"name":name,"source":table,"fields":fields})
        })
        .collect();
    OssieDocument::parse(&json!({"version":"0.2.0.dev0", "semantic_model":[{"name":"drilling","datasets":datasets}]}).to_string()).unwrap()
}

async fn engine(connection: &ClickHouse) -> Engine {
    let mut bindings = SourceBindings::new();
    for (_, table, _) in TABLES {
        bindings
            .bind(*table, connection.table(table).await.unwrap())
            .unwrap();
    }
    document().load(None, &bindings).unwrap().engine
}

async fn pretty(engine: &Engine, sql: &str) -> String {
    let result = engine
        .query(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql}\n{error}"));
    pretty_format_batches(&result).unwrap().to_string()
}

async fn local_reference(remote: &Engine) -> Engine {
    let mut reference = Engine::new();
    // Independently authored values, using remote schemas only to match Arrow
    // metadata/nullability exactly across the conformance boundary.
    for (table, sql) in [
        (
            "samples",
            "SELECT * FROM (VALUES (1,1,10.0,2.0,10.0),(1,2,20.0,4.0,20.0),(2,1,5.0,1.0,30.0),(1,3,30.0,6.0,90.0),(1,4,0.0,1.0,NULL),(2,2,15.0,3.0,NULL)) AS t(well_id,sample_id,depth_m,duration_min,load_kn)",
        ),
        (
            "wells",
            "SELECT * FROM (VALUES (1,'North'),(2,'South'),(3,'Unmeasured')) AS t(well_id,basin)",
        ),
    ] {
        let schema = remote.catalog().relation(table).unwrap().schema.clone();
        let batches = SessionContext::new()
            .sql(sql)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        let batches: Vec<RecordBatch> = batches
            .into_iter()
            .map(|batch| {
                let columns = batch
                    .columns()
                    .iter()
                    .zip(schema.fields())
                    .map(|(array, field)| {
                        datafusion::arrow::compute::cast(array, field.data_type()).unwrap()
                    })
                    .collect();
                RecordBatch::try_new(schema.clone(), columns).unwrap()
            })
            .collect();
        let provider: Arc<dyn TableProvider> = Arc::new(
            datafusion::datasource::MemTable::try_new(schema.clone(), vec![batches]).unwrap(),
        );
        reference
            .register_table(
                semantic_catalog::Relation::base(table, schema, "local-fixture"),
                provider,
            )
            .unwrap();
    }
    reference
}

#[test]
fn clickhouse_example_inspects_offline() {
    let project = Project::from_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/clickhouse/semantic-db.yaml"
    ))
    .unwrap();
    assert_eq!(
        project
            .inspect(&Registry::standard())
            .unwrap()
            .datasets
            .len(),
        4
    );
}

#[tokio::test]
#[ignore = "connector_integration: requires Docker; run just test-connectors"]
async fn connector_integration_drilling_mergetree_and_aggregate_states() {
    let database = Database::start().await;
    let connection = database.connection(true);
    let mut remote = engine(&connection).await;
    let baseline = engine(&database.connection(false)).await;
    let reference = local_reference(&remote).await;
    assert_eq!(
        connection.metrics().queries,
        0,
        "registration only obtains schemas"
    );

    let queries = [
        "SELECT well_id, sample_id, depth_m FROM samples WHERE depth_m >= 15 ORDER BY well_id, sample_id",
        "SELECT well_id, SUM(depth_m) AS drilled, AVG(load_kn) AS mean_load, COUNT(*) AS n FROM samples GROUP BY well_id ORDER BY well_id",
        "SELECT w.basin, SUM(s.depth_m) AS drilled FROM wells w JOIN samples s ON w.well_id = s.well_id GROUP BY w.basin ORDER BY w.basin",
        "SELECT w.well_id, s.sample_id FROM wells w LEFT JOIN samples s ON w.well_id = s.well_id ORDER BY w.well_id, s.sample_id",
        "SELECT depth_m FROM samples ORDER BY depth_m DESC LIMIT 2",
        "SELECT well_id, sample_id, load_kn FROM samples ORDER BY load_kn ASC NULLS FIRST, well_id, sample_id",
        "SELECT SUM(depth_m), AVG(load_kn), MIN(depth_m), MAX(depth_m), COUNT(*) FROM samples WHERE well_id = 99",
        "SELECT SUM(load_kn), AVG(load_kn), COUNT(load_kn) FROM samples WHERE sample_id = 4",
        "SELECT COUNT(*) FROM samples",
        "SELECT well_id FROM wells WHERE lower(basin) = 'south' LIMIT 1",
        "SELECT COUNT(DISTINCT well_id) FROM samples",
        "SELECT well_id FROM wells WHERE well_id IN (SELECT well_id FROM samples) ORDER BY well_id",
        "SELECT well_id, load_kn FROM samples WHERE well_id = 99",
        r"SELECT well_id FROM wells WHERE basin <> '\North' ORDER BY well_id",
        include_str!("../../../examples/clickhouse/query.sql"),
    ];
    for sql in queries {
        if let Err(error) = check_query_equivalence(&remote, &reference, &[sql]).await {
            database.execute("SYSTEM FLUSH LOGS").await;
            let exceptions = database.admin.query("SELECT exception FROM system.query_log WHERE type IN ('ExceptionBeforeStart', 'ExceptionWhileProcessing') ORDER BY event_time_microseconds DESC LIMIT 1").fetch_all::<String>().await;
            panic!(
                "{sql}\n{error}\n{exceptions:?}\n{}",
                pretty(&remote, &format!("EXPLAIN {sql}")).await
            );
        }
    }
    check_query_equivalence(&baseline, &reference, &queries)
        .await
        .unwrap();
    let before = connection.metrics();
    let sql =
        "SELECT well_id, SUM(depth_m) AS drilled FROM samples GROUP BY well_id ORDER BY well_id";
    let result = pretty(&remote, sql).await;
    assert!(
        result.contains("60.0") && result.contains("20.0"),
        "{result}"
    );
    let after = connection.metrics();
    assert_eq!(after.queries - before.queries, 1);
    assert_eq!(
        after.rows - before.rows,
        2,
        "six raw rows must aggregate remotely to two"
    );
    let plan = pretty(&remote, &format!("EXPLAIN {sql}")).await;
    assert!(
        plan.contains("VirtualExecutionPlan") && plan.contains("sumOrNull"),
        "{plan}"
    );
    let join_plan = pretty(&remote, "EXPLAIN SELECT w.basin, SUM(s.depth_m) FROM wells w JOIN samples s ON w.well_id=s.well_id GROUP BY w.basin").await;
    assert!(
        join_plan.contains("JOIN") && join_plan.contains("VirtualExecutionPlan"),
        "{join_plan}"
    );
    let before_join = connection.metrics();
    pretty(&remote, queries[2]).await;
    let after_join = connection.metrics();
    assert_eq!(after_join.queries - before_join.queries, 1);
    assert_eq!(after_join.rows - before_join.rows, 2);

    // Test the public configured loader as well as the direct Rust bindings.
    let mut config: ProjectConfig = serde_saphyr::from_str(include_str!(
        "../../../examples/clickhouse/semantic-db.yaml"
    ))
    .unwrap();
    config
        .connections
        .get_mut("drilling")
        .unwrap()
        .options
        .insert("endpoint".into(), json!(database.endpoint));
    let project = Project::new(
        config,
        OssieDocument::parse(include_str!(
            "../../../examples/clickhouse/drilling.ossie.yaml"
        ))
        .unwrap(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/clickhouse"),
    )
    .unwrap();
    project.inspect(&Registry::standard()).unwrap();
    let loaded = project
        .load(&Registry::standard(), &|name| {
            (name == "CLICKHOUSE_PASSWORD").then(|| PASSWORD.into())
        })
        .await
        .unwrap();
    check_query_equivalence(&loaded.engine, &reference, &queries)
        .await
        .unwrap();

    // Local relation forces residual execution above a genuinely remote source.
    remote
        .create_view("local_teams", "SELECT 1 AS well_id, 'A' AS team")
        .await
        .unwrap();
    let joined = pretty(&remote, "SELECT t.team, SUM(s.depth_m) AS drilled FROM local_teams t JOIN samples s ON t.well_id=s.well_id GROUP BY t.team").await;
    assert!(joined.contains("60.0"), "{joined}");
    remote
        .create_view(
            "productive",
            "SELECT well_id, depth_m FROM samples WHERE depth_m > 0",
        )
        .await
        .unwrap();
    assert!(
        pretty(&remote, "SELECT SUM(depth_m) FROM productive")
            .await
            .contains("80.0")
    );

    let summary_sql =
        "SELECT well_id, depth_m, mean_load_kn, samples FROM summary ORDER BY well_id";
    let totals_sql =
        "SELECT well_id, SUM(depth_m) AS drilled FROM totals GROUP BY well_id ORDER BY well_id";
    let unmerged = pretty(&remote, summary_sql).await;
    assert_eq!(unmerged, pretty(&reference, "SELECT well_id, SUM(depth_m) AS depth_m, AVG(load_kn) AS mean_load_kn, COUNT(*) AS samples FROM samples GROUP BY well_id ORDER BY well_id").await, "states must merge to the independently computed weighted average and totals");
    assert_eq!(pretty(&remote, totals_sql).await, result);
    for table in ["drilling_states", "drilling_totals"] {
        database
            .execute(&format!("SYSTEM START MERGES {table}"))
            .await;
        database
            .execute(&format!("OPTIMIZE TABLE {table} FINAL"))
            .await;
    }
    assert_eq!(pretty(&remote, summary_sql).await, unmerged);
    assert_eq!(pretty(&remote, totals_sql).await, result);
    database
        .execute("INSERT INTO drilling_samples VALUES (1,5,12,2,40)")
        .await;
    let refreshed = pretty(&remote, summary_sql).await;
    assert!(
        refreshed.contains("72.0") && refreshed.contains("40.0"),
        "fresh scans must observe new states: {refreshed}"
    );
    assert!(
        connection.table("drilling_states").await.is_err(),
        "raw aggregate states must not be treated as finalized numbers"
    );
}

#[tokio::test]
#[ignore = "connector_integration: requires Docker; run just test-connectors"]
async fn connector_integration_isolation_limits_and_failures() {
    let database = Database::start().await;
    let first = database.connection(true);
    let second = database.connection(true);
    let mut engine = Engine::new();
    for (name, connection) in [("a", &first), ("b", &second)] {
        let provider = connection.table("wells").await.unwrap();
        engine
            .register_table(
                semantic_catalog::Relation::base(name, provider.schema(), "fixture"),
                provider,
            )
            .unwrap();
    }
    let frame = engine
        .plan_sql("SELECT a.well_id FROM a JOIN b ON a.well_id=b.well_id")
        .await
        .unwrap();
    assert_eq!(first.metrics().queries + second.metrics().queries, 0);
    frame.collect().await.unwrap();
    assert_eq!(
        first.metrics().queries,
        1,
        "independent connection contexts must not fuse"
    );
    assert_eq!(second.metrics().queries, 1);

    let mut config = ClickHouseConfig::new(&database.endpoint, "drilling", "fixture", PASSWORD);
    config.max_response_bytes = 4096;
    let limited = ClickHouse::new(config).unwrap();
    database.execute("CREATE VIEW large_result AS SELECT number AS id, repeat('x', 200) AS payload FROM numbers(1000)").await;
    let provider = limited.table("large_result").await.unwrap();
    engine
        .register_table(
            semantic_catalog::Relation::base("large", provider.schema(), "fixture"),
            provider,
        )
        .unwrap();
    assert!(
        engine
            .query("SELECT * FROM large")
            .await
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
    assert!(first.table("missing_table").await.is_err());
    assert!(engine.query("DROP TABLE a").await.is_err());
    let fallback = database.connection(false);
    let provider = fallback.table("wells").await.unwrap();
    engine
        .register_table(
            semantic_catalog::Relation::base("fallback", provider.schema(), "fixture"),
            provider,
        )
        .unwrap();
    let before_alter = pretty(&engine, "SELECT * FROM fallback ORDER BY well_id").await;
    database
        .execute("ALTER TABLE wells ADD COLUMN extra Int64 DEFAULT 99 FIRST")
        .await;
    assert_eq!(
        pretty(&engine, "SELECT * FROM fallback ORDER BY well_id").await,
        before_alter
    );
    let bad = ClickHouse::new(ClickHouseConfig::new(
        &database.endpoint,
        "drilling",
        "fixture",
        "wrong-secret-must-not-appear",
    ))
    .unwrap();
    let error = bad.table("wells").await.unwrap_err().to_string();
    assert!(!error.contains("wrong-secret") && !error.contains(PASSWORD));

    // A failure after successful registration must fail execution, not end a scan.
    database.execute("DROP TABLE wells").await;
    assert!(engine.query("SELECT * FROM a").await.is_err());
}
