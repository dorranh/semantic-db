use datafusion::{
    arrow::{
        array::*, datatypes::*, record_batch::RecordBatch, util::pretty::pretty_format_batches,
    },
    datasource::MemTable,
};
use futures::StreamExt;
use semantic_catalog::Relation;
use semantic_engine::{Engine, QueryOptions};
use semantic_postgres::{Postgres, PostgresOptions, PostgresTlsConfig};
use std::sync::Arc;
#[path = "../../../tests/support/postgres.rs"]
mod fixture;
fn connection(db: &fixture::Database, optimized: bool) -> Postgres {
    Postgres::new_with_options(
        &db.url,
        2,
        32,
        PostgresTlsConfig::default(),
        PostgresOptions {
            allow_insecure_transport: true,
            filter_pushdown: optimized,
            federation: optimized,
            ..Default::default()
        },
    )
    .unwrap()
}
async fn engine(pg: &Postgres) -> Engine {
    let mut e = Engine::new();
    for name in ["items", "keys"] {
        let t = pg.table("public", name).await.unwrap();
        e.register_table(Relation::base(name, t.schema(), "postgres"), t)
            .unwrap();
    }
    e
}
fn reference() -> Engine {
    let mut e = Engine::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("k", DataType::Int64, true),
        Field::new("label", DataType::Utf8, true),
    ]));
    let b = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])),
            Arc::new(Int64Array::from(vec![
                Some(1),
                Some(1),
                Some(2),
                None,
                Some(9),
            ])),
            Arc::new(StringArray::from(vec![
                Some("one"),
                None,
                Some("three"),
                Some("four"),
                Some("TARGET"),
            ])),
        ],
    )
    .unwrap();
    e.register_table(
        Relation::base("items", schema.clone(), "local"),
        Arc::new(MemTable::try_new(schema, vec![vec![b]]).unwrap()),
    )
    .unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new("k", DataType::Int64, true)]));
    let b = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int64Array::from(vec![
            Some(1),
            Some(1),
            Some(2),
            Some(7),
            None,
        ]))],
    )
    .unwrap();
    e.register_table(
        Relation::base("keys", schema.clone(), "local"),
        Arc::new(MemTable::try_new(schema, vec![vec![b]]).unwrap()),
    )
    .unwrap();
    e
}
async fn text(e: &Engine, sql: &str) -> String {
    pretty_format_batches(&e.query(sql).await.unwrap_or_else(|e| panic!("{sql}: {e}")))
        .unwrap()
        .to_string()
}
#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_optimized_reference_and_placement() {
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE items(id bigint NOT NULL,k bigint,label text); INSERT INTO items VALUES(1,1,'one'),(2,1,NULL),(3,2,'three'),(4,NULL,'four'),(5,9,'TARGET'); CREATE TABLE keys(k bigint); INSERT INTO keys VALUES(1),(1),(2),(7),(NULL); ANALYZE items;").await.unwrap();
    let pg = connection(&db, true);
    let optimized = engine(&pg).await;
    let baseline = engine(&connection(&db, false)).await;
    let local = reference();
    for sql in [
        "SELECT label FROM items WHERE id>=3 ORDER BY id",
        "SELECT id FROM items WHERE k IN (1,9) AND label IS NOT NULL ORDER BY id",
        "SELECT id FROM items WHERE NOT (k=1 OR k IS NULL) ORDER BY id",
        "SELECT id FROM items WHERE lower(label)='target' LIMIT 1",
        "SELECT COUNT(*) FROM items WHERE id>2",
        "SELECT k,COUNT(*),SUM(id),MIN(id),MAX(id) FROM items GROUP BY k ORDER BY k NULLS FIRST",
        "SELECT SUM(id),MIN(id),MAX(id),COUNT(*) FROM items WHERE id>99",
        "SELECT SUM(k),AVG(k),COUNT(k) FROM items WHERE id=4",
        "SELECT AVG(id),SUM(id) FROM items",
        "SELECT a.id,b.k FROM items a JOIN keys b ON a.k=b.k ORDER BY a.id,b.k",
        "SELECT a.id,b.k FROM items a LEFT JOIN keys b ON a.k=b.k ORDER BY a.id,b.k",
        "SELECT a.id,b.k FROM items a FULL JOIN keys b ON a.k=b.k ORDER BY a.id,b.k",
        "SELECT a.id FROM items a LEFT SEMI JOIN keys b ON a.k=b.k ORDER BY a.id",
        "SELECT a.id FROM items a LEFT ANTI JOIN keys b ON a.k=b.k ORDER BY a.id",
        "SELECT id FROM items UNION ALL SELECT k FROM keys ORDER BY id NULLS FIRST",
        "SELECT id FROM items ORDER BY id DESC LIMIT 2 OFFSET 1",
        "SELECT id, ROW_NUMBER() OVER (ORDER BY id), RANK() OVER (ORDER BY k), DENSE_RANK() OVER (ORDER BY k) FROM items ORDER BY id",
        "SELECT id,COALESCE(k,0),NULLIF(k,1) FROM items ORDER BY id",
    ] {
        let expected = text(&local, sql).await;
        assert_eq!(
            text(&optimized, sql).await,
            expected,
            "optimized {sql}\n{}",
            text(&optimized, &format!("EXPLAIN {sql}")).await
        );
        assert_eq!(text(&baseline, sql).await, expected, "baseline {sql}");
    }
    let before = pg.metrics();
    text(&optimized, "SELECT k,SUM(id) FROM items GROUP BY k").await;
    let after = pg.metrics();
    assert_eq!(after.executions - before.executions, 1);
    assert_eq!(after.rows - before.rows, 4);
    let plan = text(&optimized, "EXPLAIN SELECT k,SUM(id) FROM items GROUP BY k").await;
    assert!(
        plan.contains("delegated=true") && plan.contains("GROUP BY"),
        "{plan}"
    );
    let before = pg.metrics();
    text(
        &optimized,
        "SELECT a.id FROM items a JOIN keys b ON a.k=b.k",
    )
    .await;
    assert_eq!(pg.metrics().executions - before.executions, 1);
    let plan = text(
        &optimized,
        "EXPLAIN SELECT id FROM items WHERE lower(label)='target' LIMIT 1",
    )
    .await;
    assert!(plan.contains("FilterExec"), "{plan}");
    let plan = text(&optimized, "EXPLAIN SELECT id FROM items WHERE id=5").await;
    assert!(plan.contains("$1::bigint"), "{plan}");
    let before = pg.metrics();
    text(&optimized, "SELECT label FROM items WHERE id=5").await;
    assert_eq!(pg.metrics().rows - before.rows, 1);
    assert_eq!(pg.capabilities().await.unwrap().version_number / 10000, 18);
    let meta = pg.metadata("public", "items").await.unwrap();
    assert!(!meta.schema.field(0).is_nullable());
    assert!(meta.estimated_rows.is_some());
    db.client
        .batch_execute("ALTER TABLE items ALTER COLUMN id DROP NOT NULL")
        .await
        .unwrap();
    assert!(optimized.query("SELECT id FROM items").await.is_err());
    assert!(
        pg.refresh_table("public", "items")
            .await
            .unwrap()
            .schema()
            .field(0)
            .is_nullable()
    );
}
#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_types_budgets_and_overflow() {
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE types(n numeric(20,4),u uuid,b bytea,j jsonb,t time,i interval,a bigint[]); INSERT INTO types VALUES(1234567890123456.1234,'12345678-1234-1234-1234-123456789012','\\x00ff','{\"x\":1}','12:34:56.123456','1 month 2 days 3 seconds',ARRAY[1,NULL,3]::bigint[]),(NULL,NULL,NULL,NULL,NULL,NULL,NULL); CREATE TABLE totals(n bigint); INSERT INTO totals VALUES(9223372036854775807),(1); CREATE TABLE wide(id bigint,label text); INSERT INTO wide SELECT n,repeat('x',2000) FROM generate_series(1,3) n;").await.unwrap();
    let pg = connection(&db, true);
    let mut e = Engine::new();
    for name in ["types", "totals", "wide"] {
        let t = pg.table("public", name).await.unwrap();
        e.register_table(Relation::base(name, t.schema(), "postgres"), t)
            .unwrap();
    }
    let rows = e.query("SELECT * FROM types").await.unwrap();
    assert_eq!(rows.iter().map(|b| b.num_rows()).sum::<usize>(), 2);
    assert_eq!(
        rows[0]
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap()
            .value(0),
        12345678901234561234
    );
    assert_eq!(
        rows[0]
            .column(6)
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap()
            .value(0)
            .len(),
        3
    );
    assert!(
        text(&e, "SELECT SUM(n) FROM totals")
            .await
            .contains("-9223372036854775808")
    );
    assert!(
        text(&e, "SELECT SUM(n)>0 AS positive FROM totals")
            .await
            .contains("false")
    );
    let baseline = connection(&db, false);
    let t = baseline.table("public", "totals").await.unwrap();
    let mut local = Engine::new();
    local
        .register_table(Relation::base("totals", t.schema(), "postgres"), t)
        .unwrap();
    assert_eq!(
        text(&e, "SELECT SUM(n) FROM totals").await,
        text(&local, "SELECT SUM(n) FROM totals").await
    );
    assert!(
        text(&e, "SELECT SUM(n) FROM totals WHERE n=1")
            .await
            .contains('1')
    );
    let pg = Postgres::new_with_options(
        &db.url,
        1,
        1,
        PostgresTlsConfig::default(),
        PostgresOptions {
            allow_insecure_transport: true,
            max_batch_bytes: 16384,
            max_session_bytes: 16384,
            ..Default::default()
        },
    )
    .unwrap();
    let t = pg.table("public", "wide").await.unwrap();
    let mut e = Engine::new();
    e.register_table(Relation::base("wide", t.schema(), "postgres"), t)
        .unwrap();
    let mut result = e
        .execute(
            "SELECT * FROM wide",
            QueryOptions {
                max_remote_bytes: 3000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.stream.next().await.unwrap().is_ok());
    assert!(result.stream.next().await.unwrap().is_err());
    drop(result);
    assert!(e.query("SELECT id FROM wide LIMIT 1").await.is_ok());
    let mut result = e
        .execute(
            "SELECT * FROM wide",
            QueryOptions {
                max_decoded_bytes: 12000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.stream.next().await.unwrap().is_ok());
    assert!(result.stream.next().await.unwrap().is_err());
    drop(result);
    db.client
        .batch_execute("UPDATE wide SET label=repeat('x',100000) WHERE id=1")
        .await
        .unwrap();
    assert!(e.query("SELECT * FROM wide WHERE id=1").await.is_err());
    assert!(e.query("SELECT id FROM wide LIMIT 1").await.is_ok());
}
#[test]
fn production_policy_is_explicit() {
    for mode in ["disable", "allow", "prefer", "require", "verify-ca"] {
        assert!(
            Postgres::new_with_options(
                &format!("postgres://localhost/x?sslmode={mode}"),
                1,
                1,
                PostgresTlsConfig::default(),
                PostgresOptions::default()
            )
            .is_err()
        );
    }
    assert!(
        Postgres::new_with_options(
            "postgres://localhost/x?sslmode=verify-full",
            1,
            1,
            PostgresTlsConfig::default(),
            PostgresOptions::default()
        )
        .is_ok()
    );
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_mixed_context_and_pool_backpressure() {
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE items(id bigint); INSERT INTO items VALUES(1),(2),(3),(4),(5); CREATE VIEW delayed AS SELECT 1::bigint id WHERE pg_sleep(3)::text=''").await.unwrap();
    let pg = Postgres::new_with_options(
        &db.url,
        1,
        1,
        PostgresTlsConfig::default(),
        PostgresOptions {
            allow_insecure_transport: true,
            acquire_timeout_ms: 1000,
            ..Default::default()
        },
    )
    .unwrap();
    let fallback = connection(&db, false);
    let mut e = Engine::new();
    for (name, conn, table) in [
        ("remote", &pg, "items"),
        ("fallback", &fallback, "items"),
        ("delayed", &pg, "delayed"),
    ] {
        let t = conn.table("public", table).await.unwrap();
        e.register_table(Relation::base(name, t.schema(), "postgres"), t)
            .unwrap();
    }
    let sql = "SELECT COUNT(*) AS id FROM remote UNION ALL SELECT id FROM fallback";
    let plan = text(&e, &format!("EXPLAIN {sql}")).await;
    assert!(
        plan.contains("delegated=true") && plan.contains("delegated=false"),
        "{plan}"
    );
    for sql in ["SELECT COUNT(*) FROM remote", "SELECT id FROM fallback"] {
        e.execute(
            sql,
            QueryOptions {
                max_remote_bytes: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    }
    let result = e
        .execute(
            sql,
            QueryOptions {
                max_remote_bytes: 100,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let context = result.context.clone();
    assert!(result.collect().await.is_err());
    assert_eq!(
        context.metrics().estimated_remote_bytes,
        context.metrics().remote_bytes
    );
    let result = e
        .execute(
            sql,
            QueryOptions {
                max_remote_requests: 4,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.collect().await.is_ok());
    let result = e
        .execute(
            sql,
            QueryOptions {
                max_remote_requests: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.collect().await.is_err());
    // Two independently configured clients at the same URL must remain separate.
    let plan = text(
        &e,
        "EXPLAIN SELECT r.id FROM remote r JOIN fallback f ON r.id=f.id",
    )
    .await;
    assert!(plan.contains("HashJoinExec"), "{plan}");
    let mut held = e
        .execute("SELECT * FROM remote", QueryOptions::default())
        .await
        .unwrap();
    held.stream.next().await.unwrap().unwrap();
    let blocked = e
        .execute("SELECT * FROM remote", QueryOptions::default())
        .await
        .unwrap();
    assert!(blocked.collect().await.is_err());
    drop(held);
    assert!(e.query("SELECT id FROM remote LIMIT 1").await.is_ok());
    let mut running = e
        .execute("SELECT * FROM delayed", QueryOptions::default())
        .await
        .unwrap();
    let context = running.context.clone();
    let cancel = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        context.cancel();
    });
    assert!(running.stream.next().await.unwrap().is_err());
    cancel.await.unwrap();
    drop(running);
    let mut active = 1i64;
    for _ in 0..40 {
        active=db.client.query_one("SELECT count(*) FROM pg_stat_activity WHERE state='active' AND query LIKE '%delayed%' AND pid<>pg_backend_pid()",&[]).await.unwrap().get(0);
        if active == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(active, 0, "cancelled server work must stop promptly");
    assert!(e.query("SELECT id FROM remote LIMIT 1").await.is_ok());
    let report = e
        .execute_read("SELECT id FROM remote LIMIT 1", vec![], Default::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap()
        .report;
    assert!(report.metrics.estimated_remote_bytes > 0);
    assert_eq!(pg.pool_health().waiting, 0);
    pg.close();
    assert!(pg.pool_health().closed);
    assert!(pg.table("public", "items").await.is_err());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_index_and_transfer() {
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE indexed(id bigint PRIMARY KEY,label text); INSERT INTO indexed SELECT n,repeat('x',64) FROM generate_series(1,10000)n; ANALYZE indexed").await.unwrap();
    let remote = connection(&db, true);
    let baseline = connection(&db, false);
    for (pg, optimized) in [(&remote, true), (&baseline, false)] {
        let mut e = Engine::new();
        e.set_query_options(QueryOptions {
            max_remote_requests: 1024,
            ..Default::default()
        })
        .unwrap();
        let t = pg.table("public", "indexed").await.unwrap();
        e.register_table(Relation::base("indexed", t.schema(), "postgres"), t)
            .unwrap();
        for (predicate, expected) in [("id=9999", 1), ("id>0", 10000)] {
            let before = pg.metrics();
            let started = std::time::Instant::now();
            let rows = e
                .query(&format!("SELECT id,label FROM indexed WHERE {predicate}"))
                .await
                .unwrap();
            assert_eq!(rows.iter().map(|b| b.num_rows()).sum::<usize>(), expected);
            let after = pg.metrics();
            assert_eq!(
                after.rows - before.rows,
                if optimized { expected as u64 } else { 10000 }
            );
            eprintln!(
                "optimized={optimized} predicate={predicate} rows={} estimated_wire_bytes={} elapsed_ms={}",
                after.rows - before.rows,
                after.estimated_wire_bytes - before.estimated_wire_bytes,
                started.elapsed().as_millis()
            );
        }
    }
    let plan = db
        .client
        .query(
            "EXPLAIN (ANALYZE, BUFFERS) SELECT id,label FROM indexed WHERE id=9999",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("Index Scan") && plan.contains("rows=1"),
        "{plan}"
    );
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_pinned_collection_limit() {
    use semantic_engine::{ReadOptions, ReadSessionOptions};
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE items(id bigint PRIMARY KEY,label text); INSERT INTO items SELECT n,repeat('x',2000) FROM generate_series(1,20)n").await.unwrap();
    let pg = Postgres::new_with_options(
        &db.url,
        1,
        1,
        PostgresTlsConfig::default(),
        PostgresOptions {
            allow_insecure_transport: true,
            max_batch_bytes: 16384,
            max_session_bytes: 16384,
            ..Default::default()
        },
    )
    .unwrap();
    let (table, read, _) = pg.bindings("public", "items").await.unwrap();
    let mut e = Engine::new();
    e.register_table(Relation::base("items", table.schema(), "postgres"), table)
        .unwrap();
    e.attach_read_binding("items", read).unwrap();
    let mut session = e
        .begin_read_session(&["items".into()], ReadSessionOptions::default())
        .await
        .unwrap();
    let before = pg.metrics();
    let result = session
        .query(
            "SELECT label FROM items WHERE id=20",
            vec![],
            ReadOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.batches.iter().map(|b| b.num_rows()).sum::<usize>(),
        1
    );
    assert_eq!(pg.metrics().rows - before.rows, 1);
    assert!(
        session
            .query("SELECT * FROM items", vec![], ReadOptions::default())
            .await
            .is_err()
    );
    assert!(
        session
            .query(
                "SELECT id FROM items LIMIT 1",
                vec![],
                ReadOptions::default()
            )
            .await
            .is_err()
    );
    drop(session);
    assert!(e.query("SELECT id FROM items LIMIT 1").await.is_ok());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_late_error_and_privileges() {
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE FUNCTION public.fail_late(n bigint) RETURNS bigint LANGUAGE plpgsql AS $$ BEGIN IF n=3 THEN RAISE EXCEPTION 'private-payload'; END IF; RETURN n; END $$; CREATE VIEW failing AS SELECT public.fail_late(n) AS id FROM generate_series(1,4)n; CREATE ROLE limited LOGIN PASSWORD 'fixture'; CREATE TABLE secret(id bigint); INSERT INTO secret VALUES(1);").await.unwrap();
    let pg = Postgres::new(&db.url, 1, 1).unwrap();
    let t = pg.table("public", "failing").await.unwrap();
    let mut e = Engine::new();
    e.register_table(Relation::base("failing", t.schema(), "postgres"), t)
        .unwrap();
    let mut result = e
        .execute_read("SELECT * FROM failing", vec![], Default::default())
        .await
        .unwrap();
    assert!(result.stream.next().await.unwrap().is_ok());
    assert!(result.stream.next().await.unwrap().is_ok());
    let error = result.stream.next().await.unwrap().unwrap_err();
    assert!(!error.to_string().contains("private-payload"));
    assert_eq!(
        result.report.lock().unwrap().completion,
        semantic_engine::ReadCompletion::Failed
    );
    drop(result);
    let t = pg.table("public", "secret").await.unwrap();
    e.register_table(Relation::base("secret", t.schema(), "postgres"), t)
        .unwrap();
    assert!(e.query("SELECT * FROM secret").await.is_ok());
    let limited = Postgres::new(
        &db.url.replace("postgres:fixture@", "limited:fixture@"),
        1,
        1,
    )
    .unwrap();
    assert!(limited.table("public", "secret").await.is_err());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_extended_type_writes() {
    use datafusion::common::ScalarValue;
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE typed(n numeric(20,4),u uuid,b bytea,j jsonb,t time,i interval,a bigint[])").await.unwrap();
    let pg = Postgres::new(&db.url, 2, 32).unwrap().with_writes();
    let (table, read, write) = pg.bindings("public", "typed").await.unwrap();
    let mut e = Engine::new();
    e.register_table(Relation::base("typed", table.schema(), "postgres"), table)
        .unwrap();
    e.attach_read_binding("typed", read).unwrap();
    e.attach_write_binding("typed", write.unwrap())
        .await
        .unwrap();
    let list = ScalarValue::List(ScalarValue::new_list(
        &[ScalarValue::Int64(Some(1)), ScalarValue::Int64(None)],
        &DataType::Int64,
        true,
    ));
    let values = vec![
        ScalarValue::Decimal128(Some(123456), 20, 4),
        ScalarValue::FixedSizeBinary(16, Some(vec![1; 16])),
        ScalarValue::Binary(Some(vec![0, 255])),
        ScalarValue::Utf8(Some("{\"answer\":42}".into())),
        ScalarValue::Time64Microsecond(Some(123456)),
        ScalarValue::IntervalMonthDayNano(Some(IntervalMonthDayNanoType::make_value(
            1, 2, 3_000_000,
        ))),
        list,
    ];
    let prepared = e
        .prepare_write("INSERT INTO typed(n,u,b,j,t,i,a) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .await
        .unwrap();
    let result = prepared.execute(values, Default::default()).await.unwrap();
    assert!(result.success(), "{result:?}");
    let rows = e.query("SELECT * FROM typed").await.unwrap();
    assert_eq!(rows[0].num_rows(), 1);
    assert_eq!(
        rows[0]
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap()
            .value(0),
        123456
    );
    assert_eq!(
        rows[0]
            .column(1)
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .unwrap()
            .value(0),
        &[1; 16]
    );
    assert_eq!(
        rows[0]
            .column(2)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .value(0),
        &[0, 255]
    );
    assert!(
        rows[0]
            .column(3)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0)
            .contains("42")
    );
    assert_eq!(
        rows[0]
            .column(4)
            .as_any()
            .downcast_ref::<Time64MicrosecondArray>()
            .unwrap()
            .value(0),
        123456
    );
    assert_eq!(
        rows[0]
            .column(5)
            .as_any()
            .downcast_ref::<IntervalMonthDayNanoArray>()
            .unwrap()
            .value(0),
        IntervalMonthDayNanoType::make_value(1, 2, 3_000_000)
    );
    assert_eq!(
        rows[0]
            .column(6)
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap()
            .value(0)
            .null_count(),
        1
    );
}
