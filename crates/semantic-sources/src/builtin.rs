use super::*;
use serde::de::DeserializeOwned;
#[cfg(feature = "postgres")]
#[path = "postgres.rs"]
mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PostgresConnector;

#[cfg(feature = "clickhouse")]
#[path = "clickhouse.rs"]
mod clickhouse;
#[cfg(feature = "clickhouse")]
pub use clickhouse::ClickHouseConnector;

pub(super) fn options<T: DeserializeOwned>(value: &Options) -> Result<T> {
    serde_json::from_value(Value::Object(value.clone()))
        .map_err(|e| SourceError::configuration("options", "/", e.to_string()))
}

#[cfg(feature = "github")]
pub use github::GitHubConnector;
#[cfg(feature = "github")]
mod github {
    use super::*;
    use semantic_github::{GitHub, GitHubConfig};
    use std::time::Duration;

    pub struct GitHubConnector;
    struct GitHubConnection(GitHub, String);
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct GitHubOptions {
        token_env: String,
        repositories: Vec<String>,
        endpoint: Option<String>,
        page_size: Option<usize>,
        max_requests_per_scan: Option<usize>,
        request_timeout_seconds: Option<u64>,
        max_response_bytes: Option<usize>,
        filter_pushdown: Option<bool>,
    }
    impl GitHubOptions {
        fn config(&self, token: String) -> GitHubConfig {
            let mut config = GitHubConfig::new(token, self.repositories.clone());
            if let Some(value) = &self.endpoint {
                config.endpoint = value.clone();
            }
            if let Some(value) = self.page_size {
                config.page_size = value;
            }
            if let Some(value) = self.max_requests_per_scan {
                config.max_requests_per_scan = value;
            }
            if let Some(value) = self.request_timeout_seconds {
                config.request_timeout = Duration::from_secs(value);
            }
            if let Some(value) = self.max_response_bytes {
                config.max_response_bytes = value;
            }
            if let Some(value) = self.filter_pushdown {
                config.filter_pushdown = value;
            }
            config
        }
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct GitHubSource {
        collection: Collection,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Collection {
        Issues,
        IssueLabels,
    }

    impl ConnectorFactory for GitHubConnector {
        fn validate_connection(&self, value: &Options) -> Result<()> {
            let options: GitHubOptions = options(value)?;
            if options.token_env.trim().is_empty() {
                return Err(SourceError::configuration(
                    "options",
                    "/token_env",
                    "credential variable name must be nonempty",
                ));
            }
            options.config(String::new()).validate()?;
            Ok(())
        }
        fn validate_source(&self, value: &Options) -> Result<()> {
            options::<GitHubSource>(value).map(|_| ())
        }
        fn connect<'a>(
            &'a self,
            value: &'a Options,
            secrets: &'a SecretResolver<'_>,
        ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
            Box::pin(async move {
                let options: GitHubOptions = options(value)?;
                let token = secrets(&options.token_env)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        SourceError::configuration(
                            "missing_secret",
                            "/token_env",
                            format!(
                                "provide {} through the secret resolver (CLI: environment or .env)",
                                options.token_env
                            ),
                        )
                    })?;
                Ok(Arc::new(GitHubConnection(
                    GitHub::new(options.config(token.clone()))?,
                    semantic_runtime::fingerprint(&[
                        token.as_bytes(),
                        serde_json::to_string(value).unwrap().as_bytes(),
                    ]),
                )) as Arc<dyn SourceConnection>)
            })
        }
    }
    impl SourceConnection for GitHubConnection {
        fn authorization_scope(&self) -> Option<String> {
            Some(self.1.clone())
        }
        fn table<'a>(
            &'a self,
            value: &'a Options,
            _: &'a Path,
        ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
            Box::pin(async move {
                let options: GitHubSource = options(value)?;
                Ok(match options.collection {
                    Collection::Issues => self.0.issues()?,
                    Collection::IssueLabels => self.0.issue_labels()?,
                })
            })
        }
    }
}
