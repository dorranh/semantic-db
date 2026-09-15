use datafusion::{arrow::array::StringArray, common::ScalarValue};
use semantic_engine::*;
use semantic_postgres::Postgres;
use std::sync::Arc;
#[path = "../../../tests/support/postgres.rs"]
mod fixture;
async fn register(engine: &mut Engine, pg: &Postgres, name: &str, physical: &str) {
    let (provider, read, write) = pg.bindings("public", physical).await.unwrap();
    engine
        .register_table(
            Relation::base(name, provider.schema(), "postgres"),
            provider,
        )
        .unwrap();
    engine.attach_read_binding(name, read).unwrap();
    if let Some(write) = write {
        engine.attach_write_binding(name, write).await.unwrap();
    }
}
fn merge(value: &str) -> String {
    format!(
        "REQUIRE IDEMPOTENT MERGE INTO items AS t USING (SELECT 1::bigint AS id, '{value}' AS label) AS s ON t.id=s.id WHEN MATCHED THEN UPDATE SET label=s.label WHEN NOT MATCHED THEN INSERT(id,label) VALUES(s.id,s.label)"
    )
}
#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_rejects_unverified_index_effects_and_comparisons() {
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE items(id bigint PRIMARY KEY,label text NOT NULL); CREATE TABLE alternate_key(id text COLLATE \"C\" NOT NULL,label text); CREATE UNIQUE INDEX alternate_key_unique ON alternate_key(id COLLATE \"POSIX\")").await.unwrap();
    let pg = Postgres::new(&db.url, 2, 2).unwrap().with_writes();
    let mut engine = Engine::new();
    register(&mut engine, &pg, "items", "items").await;
    let prepared = engine.prepare_write(&merge("new")).await.unwrap();
    db.client
        .batch_execute("CREATE INDEX label_expression ON items(lower(label))")
        .await
        .unwrap();
    assert_eq!(
        prepared
            .execute(vec![], WriteOptions::default())
            .await
            .unwrap()
            .outcome,
        WriteOutcome::Aborted
    );
    assert!(engine.prepare_write(&merge("new")).await.is_err());
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM items", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    db.client
        .batch_execute("DROP INDEX label_expression; CREATE TABLE child_items() INHERITS(items)")
        .await
        .unwrap();
    assert!(engine.prepare_write(&merge("new")).await.is_err());
    register(&mut engine, &pg, "alternate_key", "alternate_key").await;
    assert!(engine.prepare_write("REQUIRE IDEMPOTENT MERGE INTO alternate_key t USING (SELECT 'key' AS id, 'value' AS label) s ON t.id=s.id WHEN NOT MATCHED THEN INSERT(id,label) VALUES(s.id,s.label)").await.is_err());
}
async fn label(engine: &Engine) -> String {
    let b = engine
        .query("SELECT label FROM items WHERE id=1")
        .await
        .unwrap();
    b[0].column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .value(0)
        .into()
}
#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_writes_and_sessions() {
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE items(id bigint PRIMARY KEY,label text NOT NULL,human text NOT NULL DEFAULT 'preserved');").await.unwrap();
    let pg = Postgres::new(&db.url, 8, 2).unwrap().with_writes();
    let mut engine = Engine::new();
    register(&mut engine, &pg, "items", "items").await;
    register(&mut engine, &pg, "alias", "items").await;
    let p = engine.prepare_write(&merge("first")).await.unwrap();
    let first = p.execute(vec![], WriteOptions::default()).await.unwrap();
    assert_eq!(first.outcome, WriteOutcome::Committed, "{first:?}");
    assert_eq!(first.affected_rows, Some(1));
    let replay = p.execute(vec![], WriteOptions::default()).await.unwrap();
    assert_eq!(replay.outcome, WriteOutcome::Committed, "{replay:?}");
    assert_eq!(replay.affected_rows, Some(0));
    assert_eq!(label(&engine).await, "first");
    let update = engine
        .prepare_write("UPDATE items SET label=$2::text WHERE id=$1::bigint")
        .await
        .unwrap()
        .execute(
            vec![
                ScalarValue::Int64(Some(1)),
                ScalarValue::Utf8(Some("';DELETE FROM items;--".into())),
            ],
            WriteOptions::default(),
        )
        .await
        .unwrap();
    assert!(update.success(), "{update:?}");
    assert_eq!(label(&engine).await, "';DELETE FROM items;--");
    let feedback = "REQUIRE IDEMPOTENT MERGE INTO items AS t USING (SELECT id,label FROM alias) AS s ON t.id=s.id WHEN MATCHED THEN UPDATE SET label=s.label";
    assert!(engine.prepare_write(feedback).await.is_err());
    let alternate = Postgres::new(&db.url, 2, 2).unwrap();
    register(&mut engine, &alternate, "alternate", "items").await;
    assert!(
        engine
            .prepare_write(&feedback.replace("FROM alias", "FROM alternate"))
            .await
            .is_err()
    );
    assert!(engine.query("DELETE FROM items").await.is_err());
    assert!(
        engine
            .prepare_write("UPDATE items SET label='bad'; DELETE FROM items")
            .await
            .is_err()
    );
    let invalid = "REQUIRE IDEMPOTENT MERGE INTO items AS t USING (SELECT id,label FROM input) AS s ON t.id=s.id WHEN MATCHED THEN UPDATE SET label=s.label";
    db.client
        .batch_execute(
            "CREATE TABLE input(id bigint,label text);INSERT INTO input VALUES (1,'a'),(1,'b')",
        )
        .await
        .unwrap();
    register(&mut engine, &pg, "input", "input").await;
    let bad = engine
        .prepare_write(invalid)
        .await
        .unwrap()
        .execute(vec![], WriteOptions::default())
        .await
        .unwrap();
    assert_eq!(bad.outcome, WriteOutcome::Aborted, "{bad:?}");
    assert_eq!(label(&engine).await, "';DELETE FROM items;--");
    let mut session = engine
        .begin_read_session(&["items".into()], ReadSessionOptions::default())
        .await
        .unwrap();
    db.client
        .execute("UPDATE items SET label='outside'", &[])
        .await
        .unwrap();
    let pinned = session
        .query(
            "SELECT a.label FROM items a JOIN alias b ON a.id=b.id",
            vec![],
            ReadOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(pinned.report.established, ReadConsistency::Snapshot);
    assert_eq!(
        pinned.batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "';DELETE FROM items;--"
    );
    session.close().await.unwrap();
    let mut tx = engine
        .begin_transaction(Arc::new(pg.clone()), TransactionOptions::default())
        .await
        .unwrap();
    assert!(
        tx.execute_write("UPDATE items SET label='inside' WHERE id=1", vec![])
            .await
            .unwrap()
            .success()
    );
    let own = tx
        .query(
            "SELECT label FROM alias WHERE id=1",
            vec![],
            ReadOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        own.batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "inside"
    );
    assert_eq!(label(&engine).await, "outside");
    tx.rollback().await.unwrap();
    assert_eq!(label(&engine).await, "outside");
    let committed = engine
        .prepare_write(&merge("final"))
        .await
        .unwrap()
        .execute(vec![], WriteOptions::default())
        .await
        .unwrap();
    let visible = engine
        .execute_read(
            "SELECT * FROM items",
            vec![],
            ReadOptions {
                after_commits: vec![committed.receipt.unwrap()],
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(visible.report.completion, ReadCompletion::Complete);
    assert!(visible.report.cache_bypassed);
    let human: String = db
        .client
        .query_one("SELECT human FROM items", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(human, "preserved");
    let readonly = Postgres::new(&db.url, 2, 2).unwrap();
    let mut reads = Engine::new();
    register(&mut reads, &readonly, "items", "items").await;
    assert!(reads.prepare_write("DELETE FROM items").await.is_err());
    reads
        .execute_read(
            "SELECT * FROM items",
            vec![],
            ReadOptions {
                consistency: ReadConsistency::Snapshot,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_composite_keys_bootstrap_and_failures() {
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use std::{collections::BTreeMap, time::Duration};
    let db = fixture::Database::start().await;
    let pg = Postgres::new(&db.url, 1, 1).unwrap().with_writes();
    let mut e = Engine::new();
    e.create_table(
        Arc::new(pg.clone()),
        TableDefinition {
            name: "pairs".into(),
            target: BTreeMap::from([
                ("schema".into(), "public".into()),
                ("table".into(), "pairs".into()),
            ]),
            schema: Arc::new(Schema::new(vec![
                Field::new("a", DataType::Int64, false),
                Field::new("b", DataType::Int64, false),
                Field::new("value", DataType::Utf8, false),
            ])),
            primary_key: vec!["a".into(), "b".into()],
        },
    )
    .await
    .unwrap();
    let sql = "REQUIRE IDEMPOTENT MERGE INTO pairs t USING (SELECT 1::bigint AS a,2::bigint AS b,'v' AS value) s ON t.a=s.a AND t.b=s.b WHEN NOT MATCHED THEN INSERT(a,b,value) VALUES(s.a,s.b,s.value)";
    assert!(
        e.prepare_write(sql)
            .await
            .unwrap()
            .execute(vec![], WriteOptions::default())
            .await
            .unwrap()
            .success()
    );
    assert_eq!(
        e.prepare_write(sql)
            .await
            .unwrap()
            .execute(vec![], WriteOptions::default())
            .await
            .unwrap()
            .affected_rows,
        Some(0)
    );
    let mut tx = e
        .begin_transaction(Arc::new(pg.clone()), TransactionOptions::default())
        .await
        .unwrap();
    assert!(
        tx.execute_write("UPDATE pairs SET value='own' WHERE a=1", vec![])
            .await
            .unwrap()
            .success()
    );
    let own = tx
        .query("SELECT value FROM pairs", vec![], ReadOptions::default())
        .await
        .unwrap();
    assert_eq!(
        own.batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "own"
    );
    assert_eq!(tx.commit().await.unwrap().outcome, WriteOutcome::Committed);
    let mut session = e
        .begin_read_session(
            &["pairs".into()],
            ReadSessionOptions {
                lifetime: Duration::from_millis(30),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        session
            .query("SELECT * FROM pairs", vec![], ReadOptions::default())
            .await
            .is_err()
    );
    let prepared = e
        .prepare_write("UPDATE pairs SET value='stale'")
        .await
        .unwrap();
    db.client
        .batch_execute("ALTER TABLE pairs ADD COLUMN changed integer")
        .await
        .unwrap();
    assert_eq!(
        prepared
            .execute(vec![], WriteOptions::default())
            .await
            .unwrap()
            .outcome,
        WriteOutcome::Aborted
    );
    let v: String = db
        .client
        .query_one("SELECT value FROM pairs", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(v, "own");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_lost_commit_acknowledgement() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let db = fixture::Database::start().await;
    db.client
        .batch_execute("CREATE TABLE items(id bigint PRIMARY KEY,label text NOT NULL)")
        .await
        .unwrap();
    let config: tokio_postgres::Config = db.url.parse().unwrap();
    let upstream = config.get_ports()[0];
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            tokio::spawn(async move {
                let server = tokio::net::TcpStream::connect(("127.0.0.1", upstream))
                    .await
                    .unwrap();
                let (mut client_read, mut client_write) = client.into_split();
                let (mut server_read, mut server_write) = server.into_split();
                let inbound = tokio::spawn(async move {
                    let _ = tokio::io::copy(&mut client_read, &mut server_write).await;
                });
                loop {
                    let mut header = [0u8; 5];
                    if server_read.read_exact(&mut header).await.is_err() {
                        break;
                    }
                    let len = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
                    if !(4..=16 * 1024 * 1024).contains(&len) {
                        break;
                    }
                    let mut body = vec![0; len - 4];
                    if server_read.read_exact(&mut body).await.is_err() {
                        break;
                    }
                    // Backend has committed and produced the acknowledgement. Lose it
                    // before the client can observe success, without undoing the commit.
                    if header[0] == b'C' && body == b"COMMIT\0" {
                        break;
                    }
                    if client_write.write_all(&header).await.is_err()
                        || client_write.write_all(&body).await.is_err()
                    {
                        break;
                    }
                }
                inbound.abort();
            });
        }
    });
    let url = db
        .url
        .replace(&format!(":{upstream}/"), &format!(":{port}/"));
    let pg = Postgres::new(&url, 2, 2).unwrap().with_writes();
    let mut e = Engine::new();
    register(&mut e, &pg, "items", "items").await;
    let result = e
        .prepare_write(&merge("committed despite lost ack"))
        .await
        .unwrap()
        .execute(vec![], WriteOptions::default())
        .await
        .unwrap();
    assert_eq!(result.outcome, WriteOutcome::OutcomeUnknown, "{result:?}");
    let persisted: String = db
        .client
        .query_one("SELECT label FROM items WHERE id=1", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(persisted, "committed despite lost ack");
    proxy.abort();
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_dml_empty_inputs_and_concurrent_keys() {
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE items(id bigint PRIMARY KEY,label text,human text NOT NULL DEFAULT 'preserved')").await.unwrap();
    let pg = Postgres::new(&db.url, 4, 2).unwrap().with_writes();
    let mut engine = Engine::new();
    register(&mut engine, &pg, "items", "items").await;
    let inserted = engine
        .prepare_write("INSERT INTO items(id,label) VALUES($1::bigint,$2::text)")
        .await
        .unwrap()
        .execute(
            vec![ScalarValue::Int64(Some(2)), ScalarValue::Utf8(None)],
            WriteOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(inserted.outcome, WriteOutcome::Committed);
    assert_eq!(inserted.affected_rows, Some(1));
    assert!(
        db.client
            .query_one("SELECT label IS NULL FROM items WHERE id=2", &[])
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    let update_only = "REQUIRE IDEMPOTENT MERGE INTO items t USING (SELECT 2::bigint AS id,'changed'::text AS label) s ON t.id=s.id WHEN MATCHED THEN UPDATE SET label=s.label";
    assert_eq!(
        engine
            .prepare_write(update_only)
            .await
            .unwrap()
            .execute(vec![], WriteOptions::default())
            .await
            .unwrap()
            .affected_rows,
        Some(1)
    );
    let empty = update_only.replace("AS label)", "AS label WHERE false)");
    assert_eq!(
        engine
            .prepare_write(&empty)
            .await
            .unwrap()
            .execute(vec![], WriteOptions::default())
            .await
            .unwrap()
            .affected_rows,
        Some(0)
    );
    let deleted = engine
        .prepare_write("DELETE FROM items WHERE id=$1::bigint")
        .await
        .unwrap()
        .execute(vec![ScalarValue::Int64(Some(2))], WriteOptions::default())
        .await
        .unwrap();
    assert_eq!(deleted.outcome, WriteOutcome::Committed);
    assert_eq!(deleted.affected_rows, Some(1));
    assert_eq!(
        engine
            .prepare_write(update_only)
            .await
            .unwrap()
            .execute(vec![], WriteOptions::default())
            .await
            .unwrap()
            .affected_rows,
        Some(0)
    );

    // Hold the destination lock until both Serializable sessions have acquired
    // their snapshots, then let them compete to create the same previously absent key.
    db.client
        .batch_execute("BEGIN; LOCK TABLE items IN SHARE ROW EXCLUSIVE MODE")
        .await
        .unwrap();
    let engine = Arc::new(engine);
    let start = |engine: Arc<Engine>| {
        tokio::spawn(async move {
            engine
                .prepare_write(&merge("concurrent"))
                .await
                .unwrap()
                .execute(vec![], WriteOptions::default())
                .await
                .unwrap()
        })
    };
    let first = start(engine.clone());
    let second = start(engine.clone());
    let both_waiting = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: i64 = db.client.query_one("SELECT count(*) FROM pg_locks WHERE relation='items'::regclass AND mode='ShareRowExclusiveLock' AND NOT granted", &[]).await.unwrap().get(0);
            if waiting == 2 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await;
    db.client.batch_execute("COMMIT").await.unwrap();
    both_waiting.unwrap();
    let outcomes = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| r.outcome == WriteOutcome::Committed)
            .count(),
        1,
        "{outcomes:?}"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| r.outcome == WriteOutcome::Aborted && r.code.as_deref() == Some("40001"))
            .count(),
        1,
        "{outcomes:?}"
    );
    assert_eq!(
        db.client
            .query_one("SELECT count(*) FROM items", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        engine
            .prepare_write(&merge("concurrent"))
            .await
            .unwrap()
            .execute(vec![], WriteOptions::default())
            .await
            .unwrap()
            .affected_rows,
        Some(0)
    );
}
