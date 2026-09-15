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

#[tokio::test]
#[ignore = "requires Docker"]
async fn connector_integration_postgres_app_only_loading() {
    let db = fixture::Database::start().await;
    db.client
        .batch_execute("CREATE TABLE app_items(id bigint PRIMARY KEY,label text NOT NULL)")
        .await
        .unwrap();
    let directory = std::env::temp_dir().join(format!(
        "semantic-app-only-{}",
        semantic_runtime::unique_id()
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("v.sql"), "SELECT id,label FROM items").unwrap();
    let config = json!({"connections":{"app":{"connector":"postgres","connection_string_env":"PG","write_enabled":true}},"app_tables":{"items":{"connection":"app","schema":"public","table":"app_items"}},"views":{"labels":{"sql_file":"v.sql"}}});
    std::fs::write(directory.join("semantic-db.yaml"), config.to_string()).unwrap();
    let project = Project::from_path(directory.join("semantic-db.yaml")).unwrap();
    assert!(
        project
            .inspect_project(&Registry::standard())
            .unwrap()
            .model
            .datasets
            .is_empty()
    );
    let loaded = project
        .load(&Registry::standard(), &|_| Some(db.url.clone()))
        .await
        .unwrap();
    let result = loaded
        .engine
        .prepare_write("INSERT INTO items(id,label) VALUES(1,'app only')")
        .await
        .unwrap()
        .execute(vec![], semantic_engine::WriteOptions::default())
        .await
        .unwrap();
    assert!(result.success(), "{result:?}");
    assert_eq!(
        loaded.engine.query("SELECT * FROM labels").await.unwrap()[0].num_rows(),
        1
    );
    assert!(
        loaded
            .engine
            .prepare_write("DELETE FROM labels")
            .await
            .is_err()
    );
    let mut missing = config.clone();
    missing["app_tables"]["items"]["table"] = "absent_table".into();
    std::fs::write(directory.join("missing.yaml"), missing.to_string()).unwrap();
    assert!(
        Project::from_path(directory.join("missing.yaml"))
            .unwrap()
            .load(&Registry::standard(), &|_| Some(db.url.clone()))
            .await
            .is_err()
    );
    let absent: Option<String> = db
        .client
        .query_one("SELECT to_regclass('public.absent_table')::text", &[])
        .await
        .unwrap()
        .get(0);
    assert!(absent.is_none());
    std::fs::remove_dir_all(directory).unwrap();
}
