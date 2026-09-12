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
    password_env: String,
    query_timeout_seconds: Option<u64>,
    max_response_bytes: Option<usize>,
    federation: Option<bool>,
}

impl ConnectionOptions {
    fn config(&self, password: String) -> ClickHouseConfig {
        let mut config =
            ClickHouseConfig::new(&self.endpoint, &self.database, &self.user, password);
        if let Some(seconds) = self.query_timeout_seconds {
            config.query_timeout = Duration::from_secs(seconds);
        }
        if let Some(bytes) = self.max_response_bytes {
            config.max_response_bytes = bytes;
        }
        if let Some(enabled) = self.federation {
            config.federation = enabled;
        }
        config
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableOptions {
    table: String,
}

impl ConnectorFactory for ClickHouseConnector {
    fn validate_connection(&self, value: &Options) -> Result<()> {
        let options: ConnectionOptions = options(value)?;
        if options.password_env.trim().is_empty() {
            return Err(SourceError::configuration(
                "options",
                "/password_env",
                "secret reference must be nonempty",
            ));
        }
        options.config(String::new()).validate()?;
        Ok(())
    }
    fn validate_source(&self, value: &Options) -> Result<()> {
        let options: TableOptions = options(value)?;
        if options.table.trim().is_empty()
            || options.table.chars().any(char::is_control)
            || options.table.contains('\\')
        {
            return Err(SourceError::configuration(
                "options",
                "/table",
                "table must be a nonempty identifier without control characters or backslashes",
            ));
        }
        Ok(())
    }
    fn connect<'a>(
        &'a self,
        value: &'a Options,
        secrets: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(async move {
            let options: ConnectionOptions = options(value)?;
            let password = secrets(&options.password_env).ok_or_else(|| {
                SourceError::configuration("missing_secret", "/password_env", "provide the named password through the secret resolver (CLI: environment or .env)")
            })?;
            Ok(Arc::new(ClickHouseConnection(ClickHouse::new(
                options.config(password),
            )?)) as Arc<dyn SourceConnection>)
        })
    }
}

impl SourceConnection for ClickHouseConnection {
    fn table<'a>(
        &'a self,
        value: &'a Options,
        _: &'a Path,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            let options: TableOptions = options(value)?;
            Ok(self.0.table(&options.table).await?)
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
