use semantic_clickhouse::{ClickHouse, ClickHouseConfig};
use std::time::Duration;
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

pub const PASSWORD: &str = "fixture-password";
pub struct Database {
    _container: ContainerAsync<GenericImage>,
    pub endpoint: String,
    pub admin: clickhouse::Client,
}
impl Database {
    pub async fn start() -> Self {
        let container = GenericImage::new(
            "clickhouse/clickhouse-server",
            &std::env::var("CLICKHOUSE_TEST_TAG").unwrap_or_else(|_| "25.8.3.66".into()),
        )
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
        let sql = include_str!("../../examples/clickhouse/drilling.sql")
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        for statement in sql.split(';').filter(|sql| !sql.trim().is_empty()) {
            database.execute(statement).await;
        }
        database
    }
    pub async fn execute(&self, sql: &str) {
        self.admin
            .query(sql)
            .execute()
            .await
            .unwrap_or_else(|error| panic!("fixture statement failed: {sql}\n{error}"));
    }
    pub fn connection(&self, federation: bool) -> ClickHouse {
        let mut config = ClickHouseConfig::new(&self.endpoint, "drilling", "fixture", PASSWORD);
        config.federation = federation;
        config.filter_pushdown = federation;
        ClickHouse::new(config).unwrap()
    }
}
