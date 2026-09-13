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
use std::{path::PathBuf, sync::Arc};
#[path = "../../../tests/support/clickhouse.rs"]
mod clickhouse_fixture;
use clickhouse_fixture::{Database, PASSWORD};

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
        "SELECT SQRT(SUM(depth_m)) FROM samples GROUP BY well_id ORDER BY well_id",
        "SELECT SUM(depth_m) FILTER (WHERE sample_id > 2), COUNT(*) FILTER (WHERE load_kn > 0), COUNT(DISTINCT well_id) FILTER (WHERE sample_id > 2) FROM samples",
        "SELECT well_id, depth_m * 2.0 + 1.0, CASE WHEN load_kn IS NULL THEN 0.0 ELSE load_kn END, COALESCE(load_kn, 0.0), NULLIF(depth_m, 0.0) FROM samples ORDER BY well_id,sample_id",
        "SELECT well_id FROM samples WHERE sample_id BETWEEN 2 AND 5 AND well_id IN (1,2) ORDER BY well_id,sample_id",
        "SELECT well_id FROM wells WHERE basin LIKE 'N%' ORDER BY well_id",
        "SELECT well_id FROM wells UNION ALL SELECT well_id FROM wells ORDER BY well_id",
        "SELECT well_id,sample_id,ROW_NUMBER() OVER (PARTITION BY well_id ORDER BY sample_id),RANK() OVER (ORDER BY sample_id),DENSE_RANK() OVER (ORDER BY sample_id) FROM samples ORDER BY well_id,sample_id",
        "SELECT w.well_id FROM wells w LEFT ANTI JOIN samples s ON w.well_id=s.well_id ORDER BY w.well_id",
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
    assert_eq!(
        remote
            .query("SELECT random() FROM samples LIMIT 2")
            .await
            .unwrap()
            .iter()
            .map(|b| b.num_rows())
            .sum::<usize>(),
        2
    );
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

#[tokio::test]
#[ignore = "connector_integration: requires Docker"]
async fn connector_integration_types_final_and_partial_transfer() {
    use semantic_catalog::Relation;
    use semantic_clickhouse::TableOptions;
    let database = Database::start().await;
    database.execute("CREATE TABLE typed (id Int32, n Nullable(Int64), value Decimal(18,4), tag LowCardinality(String), items Array(Nullable(Int32)), pair Tuple(k Int32,v String), attrs Map(String,Int64), day Date32, stamp DateTime64(6,'UTC')) ENGINE=MergeTree ORDER BY id").await;
    database.execute("INSERT INTO typed VALUES (1,NULL,12.3456,'Straße',[1,NULL],(1,'α'),{'a':1},'1969-12-31','1969-12-31 23:59:59.123456'),(2,-1,-2.0001,'İΣ',[],(2,'β'),{},'2024-02-29','2024-02-29 10:20:30.000001')").await;
    let connection = database.connection(true);
    let mut remote = Engine::new();
    let table = connection.table("typed").await.unwrap();
    remote
        .register_table(Relation::base("typed", table.schema(), "typed"), table)
        .unwrap();
    let mut reference = Engine::new();
    let batches = remote.query("SELECT * FROM typed").await.unwrap();
    let schema = remote.catalog().relation("typed").unwrap().schema.clone();
    reference
        .register_table(
            Relation::base("typed", schema.clone(), "reference"),
            Arc::new(datafusion::datasource::MemTable::try_new(schema, vec![batches]).unwrap()),
        )
        .unwrap();
    for sql in [
        "SELECT * FROM typed ORDER BY id",
        "SELECT id,CAST(id AS BIGINT) AS wide_id,n,value,COALESCE(n,0),NULLIF(n,-1) FROM typed ORDER BY id",
        "SELECT id,lower(tag),upper(tag) FROM typed ORDER BY id",
        "SELECT id,EXTRACT(YEAR FROM stamp),EXTRACT(MONTH FROM stamp),DATE_TRUNC('day',stamp) FROM typed ORDER BY id",
    ] {
        check_query_equivalence(&remote, &reference, &[sql])
            .await
            .unwrap();
    }
    database
        .execute(
            "CREATE TABLE nonfinite (id Int32,v Nullable(Float64)) ENGINE=MergeTree ORDER BY id",
        )
        .await;
    database
        .execute("INSERT INTO nonfinite VALUES (1,nan),(2,inf),(3,-inf),(4,NULL),(5,-0.0),(6,0.0)")
        .await;
    let provider = connection.table("nonfinite").await.unwrap();
    remote
        .register_table(
            Relation::base("nonfinite", provider.schema(), "nonfinite"),
            provider,
        )
        .unwrap();
    let schema = remote
        .catalog()
        .relation("nonfinite")
        .unwrap()
        .schema
        .clone();
    let batches = remote
        .query("SELECT * FROM nonfinite ORDER BY id")
        .await
        .unwrap();
    reference
        .register_table(
            Relation::base("nonfinite", schema.clone(), "reference"),
            Arc::new(datafusion::datasource::MemTable::try_new(schema, vec![batches]).unwrap()),
        )
        .unwrap();
    for sql in [
        "SELECT id FROM nonfinite WHERE v=v ORDER BY id",
        "SELECT id FROM nonfinite WHERE v>0.0 ORDER BY id",
        "SELECT MIN(v),MAX(v),COUNT(v) FROM nonfinite",
        "SELECT id FROM nonfinite ORDER BY v DESC NULLS LAST,id",
        "SELECT id,NULLIF(v,v) FROM nonfinite ORDER BY id",
        "SELECT id,CASE v WHEN v THEN 1 ELSE 0 END FROM nonfinite ORDER BY id",
        "SELECT SUM(v),AVG(v) FROM nonfinite",
    ] {
        check_query_equivalence(&remote, &reference, &[sql])
            .await
            .unwrap();
    }
    for codec in [
        semantic_clickhouse::ArrowCodec::Lz4,
        semantic_clickhouse::ArrowCodec::Zstd,
        semantic_clickhouse::ArrowCodec::None,
    ] {
        for dictionary_output in [false, true] {
            let mut config =
                ClickHouseConfig::new(&database.endpoint, "drilling", "fixture", PASSWORD);
            config.codec = codec;
            config.dictionary_output = dictionary_output;
            let connection = ClickHouse::new(config).unwrap();
            let provider = connection
                .table_with_options(
                    "typed",
                    TableOptions {
                        columns: Some(vec!["id".into(), "tag".into()]),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            let mut engine = Engine::new();
            engine
                .register_table(
                    Relation::base("encoded", provider.schema(), "typed"),
                    provider,
                )
                .unwrap();
            assert_eq!(
                engine
                    .query("SELECT * FROM encoded ORDER BY id")
                    .await
                    .unwrap()
                    .iter()
                    .map(|b| b.num_rows())
                    .sum::<usize>(),
                2
            );
        }
    }
    database.execute("CREATE TABLE versions (id UInt64,version UInt64,value String) ENGINE=ReplacingMergeTree(version) ORDER BY id").await;
    database.execute("SYSTEM STOP MERGES versions").await;
    database
        .execute("INSERT INTO versions VALUES (1,1,'old')")
        .await;
    database
        .execute("INSERT INTO versions VALUES (1,2,'new'),(2,1,'other')")
        .await;
    for federation in [false, true] {
        let connection = database.connection(federation);
        let table = connection
            .table_with_options(
                "versions",
                TableOptions {
                    columns: Some(vec!["id".into(), "value".into()]),
                    final_read: true,
                },
            )
            .await
            .unwrap();
        let mut engine = Engine::new();
        engine
            .register_table(
                Relation::base("versions", table.schema(), "versions"),
                table,
            )
            .unwrap();
        let result = pretty(&engine, "SELECT * FROM versions ORDER BY id").await;
        assert!(
            result.contains("new") && !result.contains("old"),
            "{result}"
        );
    }
    let connection = database.connection(true);
    let engine = engine(&connection).await;
    let before = connection.metrics();
    pretty(
        &engine,
        "SELECT SQRT(SUM(depth_m)) FROM samples GROUP BY well_id ORDER BY well_id",
    )
    .await;
    assert_eq!(
        connection.metrics().rows - before.rows,
        2,
        "unsupported scalar must retain remote aggregation"
    );
    assert!(
        connection
            .metadata("typed")
            .await
            .unwrap()
            .columns
            .iter()
            .any(|c| c.native_type.starts_with("Decimal"))
    );
    let discovered = connection.table_names().await.unwrap();
    assert!(discovered.contains(&"typed".into()));
    assert!(connection.table("drilling_states").await.is_err());
}

#[tokio::test]
#[ignore = "connector_integration: requires Docker"]
async fn connector_integration_runtime_join_filter() {
    use datafusion::arrow::{
        array::UInt64Array,
        datatypes::{DataType, Field, Schema},
    };
    use semantic_catalog::Relation;
    let database = Database::start().await;
    database.execute("CREATE TABLE keys ENGINE=MergeTree ORDER BY id AS SELECT number AS id FROM numbers(10000)").await;
    let connection = database.connection(false);
    let mut engine = Engine::new();
    let provider = connection.table("keys").await.unwrap();
    engine
        .register_table(
            Relation::base("remote_keys", provider.schema(), "keys"),
            provider,
        )
        .unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::UInt64, false)]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(UInt64Array::from(vec![1, 2]))],
    )
    .unwrap();
    engine
        .register_table(
            Relation::base("lookup", schema.clone(), "local"),
            Arc::new(datafusion::datasource::MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    let sql = "SELECT r.id FROM lookup l JOIN remote_keys r ON l.id=r.id ORDER BY r.id";
    let before = connection.metrics();
    let result = engine.query(sql).await.unwrap();
    assert_eq!(result.iter().map(|b| b.num_rows()).sum::<usize>(), 2);
    let rows = connection.metrics().rows - before.rows;
    assert_eq!(
        rows,
        2,
        "runtime filter should fetch only two keys; plan: {}",
        pretty(&engine, &format!("EXPLAIN ANALYZE {sql}")).await
    );
}
