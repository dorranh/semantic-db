use super::*;
use semantic_clickhouse::{ClickHouse, ClickHouseConfig};
use std::time::Duration;

pub struct ClickHouseConnector;
struct ClickHouseConnection(ClickHouse);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionOptions {
    endpoint: String,
    database: String,
    user: String,
    password_env: Option<String>,
    bearer_token_env: Option<String>,
    ca_pem_env: Option<String>,
    identity_pem_env: Option<String>,
    query_timeout_seconds: Option<u64>,
    connect_timeout_seconds: Option<u64>,
    read_timeout_seconds: Option<u64>,
    pool_idle_timeout_seconds: Option<u64>,
    filter_pushdown: Option<bool>,
    runtime_filters: Option<bool>,
    runtime_filter_max_keys: Option<usize>,
    runtime_filter_max_bytes: Option<usize>,
    federation: Option<bool>,
    max_response_bytes: Option<usize>,
    max_decoded_bytes: Option<usize>,
    max_concurrent_requests: Option<usize>,
    max_attempts: Option<usize>,
    max_block_size: Option<usize>,
    codec: Option<semantic_clickhouse::ArrowCodec>,
    dictionary_output: Option<bool>,
    string_as_binary: Option<bool>,
    date_as_uint16: Option<bool>,
    server: Option<semantic_clickhouse::ServerLimits>,
    roles: Option<Vec<String>>,
    failover_endpoints: Option<Vec<String>>,
    proxy: Option<String>,
}
impl ConnectionOptions {
    fn config(
        &self,
        secrets: &SecretResolver<'_>,
        validate_only: bool,
    ) -> Result<ClickHouseConfig> {
        fn secret(
            name: &Option<String>,
            secrets: &SecretResolver<'_>,
            validate_only: bool,
        ) -> Result<Option<String>> {
            match name {
                None => Ok(None),
                Some(name) if name.trim().is_empty() => Err(SourceError::configuration(
                    "options",
                    "/credentials",
                    "secret reference must be nonempty",
                )),
                Some(_) if validate_only => Ok(Some("validation-placeholder".into())),
                Some(name) => secrets(name).map(Some).ok_or_else(|| {
                    SourceError::configuration(
                        "missing_secret",
                        "/credentials",
                        "provide the named secret through the secret resolver",
                    )
                }),
            }
        }
        if self.bearer_token_env.is_some()
            && (self.password_env.is_some() || self.identity_pem_env.is_some())
        {
            return Err(SourceError::configuration(
                "options",
                "/credentials",
                "bearer authentication cannot be combined with password or mTLS",
            ));
        }
        let mut config = ClickHouseConfig::new(
            &self.endpoint,
            &self.database,
            &self.user,
            secret(&self.password_env, secrets, validate_only)?.unwrap_or_default(),
        );
        config.bearer_token = secret(&self.bearer_token_env, secrets, validate_only)?;
        config.ca_pem = secret(&self.ca_pem_env, secrets, validate_only)?;
        config.identity_pem = secret(&self.identity_pem_env, secrets, validate_only)?;
        if let Some(value) = self.query_timeout_seconds {
            config.query_timeout = Duration::from_secs(value);
        }
        if let Some(value) = self.connect_timeout_seconds {
            config.connect_timeout = Duration::from_secs(value);
        }
        if let Some(value) = self.read_timeout_seconds {
            config.read_timeout = Duration::from_secs(value);
        }
        if let Some(value) = self.pool_idle_timeout_seconds {
            config.pool_idle_timeout = Duration::from_secs(value);
        }
        if let Some(value) = &self.filter_pushdown {
            config.filter_pushdown = *value;
        }
        if let Some(value) = &self.federation {
            config.federation = *value;
        }
        if let Some(value) = &self.max_response_bytes {
            config.max_response_bytes = *value;
        }
        if let Some(value) = &self.max_decoded_bytes {
            config.max_decoded_bytes = *value;
        }
        if let Some(value) = &self.max_concurrent_requests {
            config.max_concurrent_requests = *value;
        }
        if let Some(value) = &self.max_attempts {
            config.max_attempts = *value;
        }
        if let Some(value) = &self.max_block_size {
            config.max_block_size = *value;
        }
        if let Some(value) = &self.codec {
            config.codec = *value;
        }
        if let Some(value) = &self.dictionary_output {
            config.dictionary_output = *value;
        }
        if let Some(value) = &self.string_as_binary {
            config.string_as_binary = *value;
        }
        config.date_as_uint16 = self.date_as_uint16;
        if let Some(value) = &self.server {
            config.server = value.clone();
        }
        if let Some(value) = &self.roles {
            config.roles = value.clone();
        }
        if let Some(value) = &self.failover_endpoints {
            config.failover_endpoints = value.clone();
        }
        if let Some(value) = &self.proxy {
            config.proxy = Some(value.clone());
        }
        if let Some(value) = self.runtime_filters {
            config.runtime_filters = value;
        }
        if let Some(value) = self.runtime_filter_max_keys {
            config.runtime_filter_max_keys = value;
        }
        if let Some(value) = self.runtime_filter_max_bytes {
            config.runtime_filter_max_bytes = value;
        }
        config.validate()?;
        Ok(config)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceOptions {
    table: String,
    columns: Option<Vec<String>>,
    #[serde(default)]
    final_read: bool,
}
impl SourceOptions {
    fn table_options(&self) -> semantic_clickhouse::TableOptions {
        semantic_clickhouse::TableOptions {
            columns: self.columns.clone(),
            final_read: self.final_read,
        }
    }
    fn validate(&self) -> Result<()> {
        if self.table.trim().is_empty()
            || self.table.chars().any(char::is_control)
            || self.table.contains('\\')
        {
            return Err(SourceError::configuration(
                "options",
                "/table",
                "table must be a nonempty identifier without control characters or backslashes",
            ));
        }
        self.table_options().validate()?;
        Ok(())
    }
}
impl ConnectorFactory for ClickHouseConnector {
    fn resource_namespace(&self) -> Option<&'static str> {
        Some("clickhouse")
    }
    fn validate_connection(&self, value: &Options) -> Result<()> {
        options::<ConnectionOptions>(value)?
            .config(&|_| None, true)
            .map(|_| ())
    }
    fn validate_source(&self, value: &Options) -> Result<()> {
        options::<SourceOptions>(value)?.validate()
    }
    fn connect<'a>(
        &'a self,
        value: &'a Options,
        secrets: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(async move {
            Ok(Arc::new(ClickHouseConnection(ClickHouse::new(
                options::<ConnectionOptions>(value)?.config(secrets, false)?,
            )?)) as Arc<dyn SourceConnection>)
        })
    }
}
impl SourceConnection for ClickHouseConnection {
    fn authorization_scope(&self) -> Option<String> {
        Some(self.0.authorization_scope())
    }
    fn table<'a>(
        &'a self,
        value: &'a Options,
        _: &'a Path,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            let options: SourceOptions = options(value)?;
            Ok(self
                .0
                .table_with_options(&options.table, options.table_options())
                .await?)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn connection() -> Options {
        json!({"endpoint":"http://localhost:8123", "database":"drilling", "user":"reader", "password_env":"CH_PASSWORD"}).as_object().unwrap().clone()
    }
    #[tokio::test]
    async fn validation_is_offline_and_connect_uses_explicit_secrets() {
        let connector = ClickHouseConnector;
        connector.validate_connection(&connection()).unwrap();
        let result = connector.connect(&connection(), &|_| None).await;
        assert!(result.is_err());
        connector
            .connect(&connection(), &|name| {
                assert_eq!(name, "CH_PASSWORD");
                Some("not-logged".into())
            })
            .await
            .unwrap();
        let mut invalid = connection();
        invalid.insert("password".into(), json!("no-inline-secrets"));
        assert!(connector.validate_connection(&invalid).is_err());
        invalid = connection();
        invalid.insert("query_timeout_seconds".into(), json!(0));
        assert!(connector.validate_connection(&invalid).is_err());
        assert!(
            connector
                .validate_source(json!({"table":""}).as_object().unwrap())
                .is_err()
        );
        assert!(
            connector
                .validate_source(json!({"query":"SELECT 1"}).as_object().unwrap())
                .is_err()
        );
    }
}
