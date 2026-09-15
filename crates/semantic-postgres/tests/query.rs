use datafusion::{
    arrow::array::{Array, StringArray, TimestampMicrosecondArray},
    common::ScalarValue,
};
use futures::StreamExt;
use semantic_catalog::Relation;
use semantic_engine::{Engine, QueryOptions};
use semantic_postgres::Postgres;
#[path = "../../../tests/support/postgres.rs"]
mod fixture;
#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres() {
    let db = fixture::Database::start().await;
    db.client.batch_execute(r#"
        CREATE SCHEMA "odd space";
        CREATE TABLE "odd space"."Mixed" (id BIGINT, label TEXT, flag BOOLEAN, small SMALLINT, n INTEGER, f REAL, d DOUBLE PRECISION, day DATE, local TIMESTAMP, utc TIMESTAMPTZ);
        INSERT INTO "odd space"."Mixed" VALUES (1,'hello',true,2,3,1.5,2.5,'2026-09-01','2026-09-01 12:34:56.123456','2026-09-01 14:34:56.123456+02'),(2,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL),(3,'last',false,-2,-3,0,0,'1969-12-31','1969-12-31','1969-12-31+00');
        CREATE VIEW slow AS SELECT 1::bigint AS id WHERE pg_sleep(4)::text = '';
        CREATE TABLE unsupported (x uuid);
    "#).await.unwrap();
    let pg = Postgres::new(&db.url, 1, 1).unwrap();
    assert!(pg.table("public", "unsupported").await.is_err());
    let table = pg.table("odd space", "Mixed").await.unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(Relation::base("items", table.schema(), "postgres"), table)
        .unwrap();
    let rows = engine
        .query("SELECT * FROM items ORDER BY id")
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|b| b.num_rows()).sum::<usize>(), 3);
    let combined = datafusion::arrow::compute::concat_batches(&rows[0].schema(), &rows).unwrap();
    assert!(combined.column(1).is_null(1));
    assert_eq!(
        combined
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "hello"
    );
    assert_eq!(
        combined
            .column(8)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap()
            .value(0),
        combined
            .column(9)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap()
            .value(0)
    );
    assert_eq!(
        combined
            .column(8)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap()
            .value(0)
            % 1_000_000,
        123456
    );
    let rows = engine.query("SELECT COUNT(*) FROM items").await.unwrap();
    assert_eq!(
        ScalarValue::try_from_array(rows[0].column(0), 0).unwrap(),
        ScalarValue::Int64(Some(3))
    );
    assert_eq!(
        engine
            .query("SELECT a.id FROM items a JOIN items b ON a.id=b.id WHERE a.id=3")
            .await
            .unwrap()[0]
            .num_rows(),
        1
    );
    // Abandon a cursor after one row; the sole pool slot must remain usable.
    let mut execution = engine
        .execute("SELECT * FROM items", QueryOptions::default())
        .await
        .unwrap();
    assert!(execution.stream.next().await.unwrap().is_ok());
    drop(execution);
    assert!(engine.query("SELECT id FROM items LIMIT 1").await.is_ok());
    let slow = pg.table("public", "slow").await.unwrap();
    engine
        .register_table(Relation::base("slow", slow.schema(), "postgres"), slow)
        .unwrap();
    assert!(
        engine
            .execute(
                "SELECT * FROM slow",
                QueryOptions {
                    timeout_seconds: 1,
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
    assert!(engine.query("SELECT id FROM items LIMIT 1").await.is_ok());
    db.client
        .batch_execute("INSERT INTO \"odd space\".\"Mixed\" (id) VALUES (4)")
        .await
        .unwrap();
    assert_eq!(
        engine
            .query("SELECT id FROM items WHERE id=4")
            .await
            .unwrap()[0]
            .num_rows(),
        1
    );
    db.client
        .batch_execute(
            "ALTER TABLE \"odd space\".\"Mixed\" ALTER COLUMN label TYPE integer USING NULL",
        )
        .await
        .unwrap();
    assert!(engine.query("SELECT label FROM items").await.is_err());
}
#[test]
fn invalid_configuration_is_redacted() {
    assert!(
        format!(
            "{:?}",
            Postgres::new("not a connection secret", 1, 1).unwrap_err()
        )
        .contains("invalid Postgres")
    );
    assert!(Postgres::new("postgres://localhost/x", 1, 1).is_err());
}
