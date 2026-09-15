#![cfg(feature = "postgres")]
use semantic_sources::{Project, Registry};
use serde_json::json;
use std::path::PathBuf;
#[path = "../../../tests/support/postgres.rs"]
mod fixture;
#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_ossie() {
    let db = fixture::Database::start().await;
    db.client.batch_execute("CREATE TABLE items (id BIGINT, label TEXT); INSERT INTO items VALUES (1,'one'),(2,NULL)").await.unwrap();
    let model=semantic_ossie::OssieDocument::parse(&json!({"version":"0.2.0.dev0","semantic_model":[{"name":"app","datasets":[{"name":"renamed","source":"app.items","fields":[{"name":"item_id","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"id"}]}},{"name":"label","datatype":"String","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"label"}]}}]}]}]}).to_string()).unwrap();
    let config=serde_json::from_value(json!({"ossie":"unused.yaml","connections":{"app":{"connector":"postgres","connection_string_env":"TEST_PG"}},"sources":{"app.items":{"connection":"app","schema":"public","table":"items"}}})).unwrap();
    let project = Project::new(config, model, PathBuf::from(".")).unwrap();
    let loaded = project
        .load(&Registry::standard(), &|name| {
            (name == "TEST_PG").then(|| db.url.clone())
        })
        .await
        .unwrap();
    let rows = loaded
        .engine
        .query("SELECT item_id FROM renamed WHERE label IS NULL")
        .await
        .unwrap();
    assert_eq!(rows[0].num_rows(), 1);
}
#[test]
fn postgres_configuration_is_checked_offline() {
    use semantic_sources::{ConnectorFactory, PostgresConnector};
    for config in [
        json!({"connection_string_env":""}),
        json!({"connection_string_env":"PG","pool_size":0}),
        json!({"connection_string_env":"PG","batch_size":8193}),
        json!({"connection_string_env":"PG","password":"secret"}),
    ] {
        assert!(
            PostgresConnector
                .validate_connection(config.as_object().unwrap())
                .is_err()
        );
    }
    assert!(
        PostgresConnector
            .validate_source(
                json!({"schema":"public","table":"items"})
                    .as_object()
                    .unwrap()
            )
            .is_ok()
    );
    assert!(
        PostgresConnector
            .validate_source(json!({"table":"items"}).as_object().unwrap())
            .is_err()
    );
}
