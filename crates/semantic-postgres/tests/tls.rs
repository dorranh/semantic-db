use datafusion::common::ScalarValue;
use semantic_catalog::Relation;
use semantic_engine::{Engine, WriteOptions};
use semantic_postgres::{Postgres, PostgresTlsConfig};

#[path = "../../../tests/support/postgres.rs"]
mod plain_fixture;
#[path = "../../../tests/support/postgres_tls.rs"]
mod tls_fixture;

async fn ssl_state(pg: Postgres) -> bool {
    let table = pg.table("public", "ssl_state").await.unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(Relation::base("state", table.schema(), "postgres"), table)
        .unwrap();
    let batches = engine.query("SELECT ssl FROM state").await.unwrap();
    assert_eq!(batches.iter().map(|b| b.num_rows()).sum::<usize>(), 1);
    match ScalarValue::try_from_array(batches[0].column(0), 0).unwrap() {
        ScalarValue::Boolean(Some(value)) => value,
        other => panic!("unexpected pg_stat_ssl value: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires Docker and OpenSSL"]
async fn connector_integration_postgres_ssl_modes() {
    let db = tls_fixture::Database::start(false).await;
    db.plain_client
        .as_ref()
        .unwrap()
        .batch_execute(
            "CREATE VIEW ssl_state AS SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid(); CREATE TABLE tls_items (id bigint PRIMARY KEY, label text)",
        )
        .await
        .unwrap();
    for (mode, expected) in [
        ("disable", false),
        ("allow", false),
        ("prefer", true),
        ("require", true),
        ("verify-ca", true),
        ("verify-full", true),
    ] {
        let tls = if mode.starts_with("verify-") {
            PostgresTlsConfig {
                ca_pem: Some(db.certs.ca_pem.clone()),
                ..Default::default()
            }
        } else {
            PostgresTlsConfig::default()
        };
        let pg =
            Postgres::new_with_tls(&db.connection_string(mode, "localhost"), 2, 1, tls).unwrap();
        assert_eq!(ssl_state(pg).await, expected, "sslmode={mode}");
    }
    let trusted = PostgresTlsConfig {
        ca_pem: Some(db.certs.ca_pem.clone()),
        ..Default::default()
    };
    assert!(
        ssl_state(
            Postgres::new_with_tls(
                &db.connection_url("verify-full", "localhost"),
                1,
                1,
                trusted.clone(),
            )
            .unwrap()
        )
        .await
    );
    assert!(
        ssl_state(
            Postgres::new_with_tls(
                &db.connection_string("verify-ca", "wrong.example"),
                1,
                1,
                trusted.clone()
            )
            .unwrap()
        )
        .await
    );
    assert!(
        Postgres::new_with_tls(
            &db.connection_string("verify-full", "wrong.example"),
            1,
            1,
            trusted
        )
        .unwrap()
        .table("public", "ssl_state")
        .await
        .is_err()
    );
    let untrusted = PostgresTlsConfig {
        ca_pem: Some(db.certs.other_ca_pem.clone()),
        ..Default::default()
    };
    assert!(
        Postgres::new_with_tls(
            &db.connection_string("verify-ca", "localhost"),
            1,
            1,
            untrusted.clone()
        )
        .unwrap()
        .table("public", "ssl_state")
        .await
        .is_err()
    );
    assert!(
        Postgres::new_with_tls(
            &db.connection_string("require", "localhost"),
            1,
            1,
            untrusted
        )
        .unwrap()
        .table("public", "ssl_state")
        .await
        .is_err()
    );

    let pg = Postgres::new_with_tls(
        &db.connection_string("verify-full", "localhost"),
        2,
        2,
        PostgresTlsConfig {
            ca_pem: Some(db.certs.ca_pem.clone()),
            ..Default::default()
        },
    )
    .unwrap()
    .with_writes();
    let (provider, read, write) = pg.bindings("public", "tls_items").await.unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("tls_items", provider.schema(), "postgres"),
            provider,
        )
        .unwrap();
    engine.attach_read_binding("tls_items", read).unwrap();
    engine
        .attach_write_binding("tls_items", write.unwrap())
        .await
        .unwrap();
    let result = engine
        .prepare_write("INSERT INTO tls_items(id,label) VALUES(1,'encrypted')")
        .await
        .unwrap()
        .execute(vec![], WriteOptions::default())
        .await
        .unwrap();
    assert!(result.success(), "{result:?}");
    let label: String = db
        .plain_client
        .as_ref()
        .unwrap()
        .query_one("SELECT label FROM tls_items WHERE id=1", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(label, "encrypted");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_plaintext_fallback() {
    let db = plain_fixture::Database::start().await;
    db.client
        .batch_execute("CREATE TABLE items (id bigint)")
        .await
        .unwrap();
    for mode in ["allow", "prefer"] {
        let url = db
            .url
            .replace("sslmode=disable", &format!("sslmode={mode}"));
        assert!(
            Postgres::new(&url, 1, 1)
                .unwrap()
                .table("public", "items")
                .await
                .is_ok()
        );
    }
    for mode in ["require", "verify-ca", "verify-full"] {
        let url = db
            .url
            .replace("sslmode=disable", &format!("sslmode={mode}"));
        assert!(
            Postgres::new(&url, 1, 1)
                .unwrap()
                .table("public", "items")
                .await
                .is_err()
        );
    }
}

#[tokio::test]
#[ignore = "requires Docker and OpenSSL"]
async fn connector_integration_postgres_mutual_tls() {
    let db = tls_fixture::Database::start(true).await;
    let tls = PostgresTlsConfig {
        ca_pem: Some(db.certs.ca_pem.clone()),
        client_cert_pem: Some(db.certs.client_cert_pem.clone()),
        client_key_pem: Some(db.certs.client_key_pem.clone()),
    };
    for mode in ["verify-full", "allow", "prefer"] {
        let pg = Postgres::new_with_tls(
            &db.connection_string(mode, "localhost"),
            1,
            1,
            PostgresTlsConfig {
                ca_pem: if mode == "verify-full" {
                    tls.ca_pem.clone()
                } else {
                    None
                },
                client_cert_pem: tls.client_cert_pem.clone(),
                client_key_pem: tls.client_key_pem.clone(),
            },
        )
        .unwrap();
        assert!(
            pg.table("public", "tls_probe").await.is_ok(),
            "sslmode={mode}"
        );
    }
    assert!(
        Postgres::new_with_tls(
            &db.connection_string("verify-full", "localhost"),
            1,
            1,
            PostgresTlsConfig {
                ca_pem: tls.ca_pem.clone(),
                ..Default::default()
            }
        )
        .unwrap()
        .table("public", "tls_probe")
        .await
        .is_err()
    );
    let error = Postgres::new_with_tls(
        &db.connection_string("verify-full", "localhost"),
        1,
        1,
        PostgresTlsConfig {
            ca_pem: tls.ca_pem,
            client_cert_pem: tls.client_cert_pem,
            client_key_pem: Some("SENSITIVE-BAD-KEY".into()),
        },
    );
    assert!(error.is_err());
    assert!(!format!("{error:?}").contains("SENSITIVE-BAD-KEY"));
}
