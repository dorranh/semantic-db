use crate::{error, validate_identifier};
use datafusion::error::Result;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{fmt, time::Duration};

#[derive(Clone)]
pub struct ClickHouseConfig {
    pub endpoint: String,
    pub database: String,
    pub user: String,
    pub password: String,
    pub bearer_token: Option<String>,
    pub ca_pem: Option<String>,
    pub identity_pem: Option<String>,
    pub proxy: Option<String>,
    pub roles: Vec<String>,
    pub failover_endpoints: Vec<String>,
    /// Total attempts, including the first. No retry once a batch has been delivered.
    pub max_attempts: usize,
    pub query_timeout: Duration,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub pool_idle_timeout: Duration,
    pub max_response_bytes: usize,
    pub max_decoded_bytes: usize,
    pub max_concurrent_requests: usize,
    pub federation: bool,
    pub filter_pushdown: bool,
    pub runtime_filters: bool,
    pub runtime_filter_max_keys: usize,
    pub runtime_filter_max_bytes: usize,
    pub codec: ArrowCodec,
    pub dictionary_output: bool,
    pub string_as_binary: bool,
    /// Override the server's legacy Date-as-UInt16 Arrow representation.
    /// None preserves compatibility with servers predating this setting.
    pub date_as_uint16: Option<bool>,
    pub max_block_size: usize,
    pub server: ServerLimits,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowCodec {
    #[default]
    Lz4,
    Zstd,
    None,
}
impl ArrowCodec {
    pub(crate) fn setting(self) -> &'static str {
        match self {
            Self::Lz4 => "lz4_frame",
            Self::Zstd => "zstd",
            Self::None => "none",
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerLimits {
    pub max_memory_usage: Option<usize>,
    pub max_rows_to_read: Option<usize>,
    pub max_bytes_to_read: Option<usize>,
    pub max_result_rows: Option<usize>,
    pub max_result_bytes: Option<usize>,
    pub max_threads: Option<usize>,
    pub max_bytes_before_external_group_by: Option<usize>,
    pub max_bytes_before_external_sort: Option<usize>,
}
impl ServerLimits {
    pub(crate) fn settings(&self) -> Vec<(&'static str, usize)> {
        [
            ("max_memory_usage", self.max_memory_usage),
            ("max_rows_to_read", self.max_rows_to_read),
            ("max_bytes_to_read", self.max_bytes_to_read),
            ("max_result_rows", self.max_result_rows),
            ("max_result_bytes", self.max_result_bytes),
            ("max_threads", self.max_threads),
            (
                "max_bytes_before_external_group_by",
                self.max_bytes_before_external_group_by,
            ),
            (
                "max_bytes_before_external_sort",
                self.max_bytes_before_external_sort,
            ),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect()
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TableOptions {
    pub columns: Option<Vec<String>>,
    pub final_read: bool,
}
impl TableOptions {
    pub fn validate(&self) -> Result<()> {
        if let Some(columns) = &self.columns {
            if columns.is_empty() {
                return Err(error("source columns must not be empty"));
            }
            let mut seen = std::collections::BTreeSet::new();
            for name in columns {
                validate_identifier(name)?;
                if !seen.insert(name) {
                    return Err(error("duplicate source column"));
                }
            }
        }
        Ok(())
    }
}
impl ClickHouseConfig {
    pub fn new(
        endpoint: impl Into<String>,
        database: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            database: database.into(),
            user: user.into(),
            password: password.into(),
            bearer_token: None,
            ca_pem: None,
            identity_pem: None,
            proxy: None,
            roles: vec![],
            failover_endpoints: vec![],
            max_attempts: 1,
            query_timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            pool_idle_timeout: Duration::from_secs(2),
            max_response_bytes: 256 * 1024 * 1024,
            max_decoded_bytes: 1024 * 1024 * 1024,
            max_concurrent_requests: 8,
            federation: true,
            filter_pushdown: true,
            runtime_filters: true,
            runtime_filter_max_keys: 1000,
            runtime_filter_max_bytes: 65536,
            codec: ArrowCodec::Lz4,
            dictionary_output: false,
            string_as_binary: false,
            date_as_uint16: None,
            max_block_size: 65536,
            server: ServerLimits::default(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        for endpoint in std::iter::once(&self.endpoint).chain(&self.failover_endpoints) {
            let url = Url::parse(endpoint).map_err(|_| error("invalid endpoint"))?;
            let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
            if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
            {
                return Err(error(
                    "endpoint must be HTTPS (or loopback HTTP), without credentials, path, query or fragment",
                ));
            }
        }
        validate_identifier(&self.database)?;
        if self.user.trim().is_empty() && self.bearer_token.is_none() {
            return Err(error("user is required"));
        }
        if self.bearer_token.is_some() && (!self.password.is_empty() || self.identity_pem.is_some())
        {
            return Err(error(
                "bearer authentication cannot be combined with password or mTLS",
            ));
        }
        if self
            .bearer_token
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
        {
            return Err(error("empty bearer token"));
        }
        if self.runtime_filter_max_keys == 0
            || self.runtime_filter_max_bytes == 0
            || self.max_attempts == 0
            || self.max_attempts > 8
            || self.max_concurrent_requests == 0
            || self.max_block_size == 0
            || self.max_response_bytes == 0
            || self.max_decoded_bytes == 0
            || [
                self.query_timeout,
                self.connect_timeout,
                self.read_timeout,
                self.pool_idle_timeout,
            ]
            .iter()
            .any(|d| d.is_zero() || *d > Duration::from_secs(86400))
        {
            return Err(error("invalid timeout or resource budget"));
        }
        if self.server.settings().iter().any(|(_, v)| *v == 0) {
            return Err(error("server resource limits must be positive"));
        }
        for role in &self.roles {
            validate_identifier(role)?;
        }
        Ok(())
    }
}
impl fmt::Debug for ClickHouseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClickHouseConfig")
            .field("federation", &self.federation)
            .finish_non_exhaustive()
    }
}
