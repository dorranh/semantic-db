//! Read-only ClickHouse tables and finalized views over Arrow IPC.
//!
//! Core DataFusion federation owns remote subplans. This connector owns the
//! client, schema discovery, bounded streams and a conservative capability policy.
//! AggregateFunction state columns must be finalized in a ClickHouse view first.

mod policy;
mod transport;

use async_trait::async_trait;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::{Session, TableProvider, streaming::StreamingTable},
    common::TableReference,
    error::{DataFusionError, Result},
    execution::TaskContext,
    logical_expr::TableType,
    physical_plan::{ExecutionPlan, SendableRecordBatchStream, streaming::PartitionStream},
};
use datafusion_federation::{
    FederatedTableProviderAdaptor,
    sql::{AstAnalyzer, LogicalOptimizer, SQLExecutor, SQLFederationProvider, SQLTableSource},
};
use reqwest::Url;
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

/// Connection scope is explicit. Secrets are supplied by the embedding host.
#[derive(Clone)]
pub struct ClickHouseConfig {
    pub endpoint: String,
    pub database: String,
    pub user: String,
    pub password: String,
    /// Deadline per metadata request or remote query, including result streaming.
    pub query_timeout: Duration,
    /// Maximum decoded HTTP payload bytes per remote query, not per whole query.
    pub max_response_bytes: usize,
    /// False uses the ordinary scan provider with local filters and aggregates.
    pub federation: bool,
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
            query_timeout: Duration::from_secs(30),
            max_response_bytes: 256 * 1024 * 1024,
            federation: true,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let url = Url::parse(&self.endpoint).map_err(|_| error("invalid endpoint"))?;
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
        validate_identifier(&self.database)?;
        if self.user.trim().is_empty()
            || self.query_timeout.is_zero()
            || self.query_timeout > Duration::from_secs(86400)
            || self.max_response_bytes == 0
        {
            return Err(error(
                "user and response byte budget must be nonempty/positive; timeout must be within (0, 86400] seconds",
            ));
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

#[derive(Default)]
struct Counters {
    queries: AtomicU64,
    rows: AtomicU64,
    bytes: AtomicU64,
}

/// Cumulative execution counters shared by all tables on one connection.
/// Metadata requests are excluded. No SQL or credentials are retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryMetrics {
    pub queries: u64,
    pub rows: u64,
    pub bytes: u64,
}

struct Connection {
    client: clickhouse::Client,
    config: ClickHouseConfig,
    context: String,
    counters: Counters,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

/// Creating a client performs no I/O; `table` obtains metadata only.
#[derive(Clone)]
pub struct ClickHouse(Arc<Connection>);

impl fmt::Debug for ClickHouse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClickHouse")
            .field("context", &self.0.context)
            .finish_non_exhaustive()
    }
}

impl ClickHouse {
    pub fn new(config: ClickHouseConfig) -> Result<Self> {
        config.validate()?;
        let context = format!(
            "semantic-db:clickhouse:{}",
            NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed)
        );
        let client = clickhouse::Client::default()
            .with_url(&config.endpoint)
            .with_database(&config.database)
            .with_user(&config.user)
            .with_password(&config.password)
            .with_setting("readonly", "1")
            .with_setting("cancel_http_readonly_queries_on_client_close", "1")
            .with_setting("join_use_nulls", "1")
            .with_setting("join_default_strictness", "ALL")
            .with_setting("output_format_arrow_string_as_string", "1")
            .with_setting("output_format_arrow_low_cardinality_as_dictionary", "0")
            .with_setting(
                "max_execution_time",
                config.query_timeout.as_secs_f64().to_string(),
            )
            .with_setting("log_comment", &context);
        Ok(Self(Arc::new(Connection {
            client,
            config,
            context,
            counters: Counters::default(),
        })))
    }

    pub fn metrics(&self) -> QueryMetrics {
        let counters = &self.0.counters;
        QueryMetrics {
            queries: counters.queries.load(Ordering::Relaxed),
            rows: counters.rows.load(Ordering::Relaxed),
            bytes: counters.bytes.load(Ordering::Relaxed),
        }
    }

    /// Bind one literal table/view name within the configured database.
    /// Names are identifiers, never SQL expressions. Raw aggregation states are
    /// unsupported; bind a view with GROUP BY and the appropriate *Merge functions.
    pub async fn table(&self, table: &str) -> Result<Arc<dyn TableProvider>> {
        validate_identifier(table)?;
        let reference = TableReference::partial(self.0.config.database.clone(), table.to_owned());
        let sql = format!("SELECT * FROM {} LIMIT 0", remote_name(&reference));
        let schema = transport::schema(&self.0, &sql).await?;
        for field in schema.fields() {
            validate_identifier(field.name())?;
        }
        let fallback = Arc::new(ScanTable {
            connection: self.0.clone(),
            reference: reference.clone(),
            schema: schema.clone(),
        });
        if !self.0.config.federation {
            return Ok(fallback);
        }
        let executor = Arc::new(Executor(self.0.clone()));
        let provider = Arc::new(SQLFederationProvider::new(executor));
        let source = Arc::new(SQLTableSource::new_with_schema(
            provider,
            reference.into(),
            schema,
        ));
        Ok(Arc::new(FederatedTableProviderAdaptor::new_with_provider(
            source, fallback,
        )))
    }
}

struct Executor(Arc<Connection>);
#[async_trait]
impl SQLExecutor for Executor {
    fn name(&self) -> &str {
        "clickhouse"
    }
    fn compute_context(&self) -> Option<String> {
        Some(self.0.context.clone())
    }
    fn dialect(&self) -> Arc<dyn datafusion::sql::unparser::dialect::Dialect> {
        Arc::new(policy::ClickHouseDialect)
    }
    fn logical_optimizer(&self) -> Option<LogicalOptimizer> {
        Some(Box::new(policy::restrict_federation))
    }
    fn ast_analyzer(&self) -> Option<AstAnalyzer> {
        Some(Box::new(policy::null_on_empty))
    }
    fn execute(
        &self,
        query: &str,
        schema: SchemaRef,
        _filters: &[Arc<dyn datafusion::physical_plan::PhysicalExpr>],
    ) -> Result<SendableRecordBatchStream> {
        // Runtime filters remain local; never claim that a join-key lookup occurred.
        Ok(transport::execute(self.0.clone(), query.to_owned(), schema))
    }
    async fn table_names(&self) -> Result<Vec<String>> {
        Err(error("bind explicit table names; discovery is not enabled"))
    }
    async fn get_table_schema(&self, _table_name: &str) -> Result<SchemaRef> {
        Err(error("schemas are acquired through ClickHouse::table"))
    }
}

#[derive(Debug)]
struct ScanTable {
    connection: Arc<Connection>,
    reference: TableReference,
    schema: SchemaRef,
}

#[async_trait]
impl TableProvider for ScanTable {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    async fn scan(
        &self,
        session: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !filters.is_empty() {
            return Err(error("scan fallback does not accept pushed filters"));
        }
        // Keep the bound column order even if the remote table later adds or
        // reorders columns. SELECT * could silently relabel same-typed values.
        let columns = self
            .schema
            .fields()
            .iter()
            .map(|field| quote_identifier(field.name()))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT {columns} FROM {}", remote_name(&self.reference));
        let partition = Arc::new(ScanPartition {
            connection: self.connection.clone(),
            sql,
            schema: self.schema.clone(),
        });
        // Projection and limits are delegated locally so residual predicates can
        // consume later batches. Correct fallback precedes scan SQL optimization.
        StreamingTable::try_new(self.schema.clone(), vec![partition])?
            .scan(session, projection, filters, limit)
            .await
    }
}

#[derive(Debug)]
struct ScanPartition {
    connection: Arc<Connection>,
    sql: String,
    schema: SchemaRef,
}
impl PartitionStream for ScanPartition {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }
    fn execute(&self, _context: Arc<TaskContext>) -> SendableRecordBatchStream {
        transport::execute(
            self.connection.clone(),
            self.sql.clone(),
            self.schema.clone(),
        )
    }
}

fn validate_identifier(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) || value.contains('\\') {
        return Err(error(
            "identifiers must be nonempty and contain no control characters or backslashes",
        ));
    }
    Ok(())
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn remote_name(reference: &TableReference) -> String {
    format!(
        "{}.{}",
        quote_identifier(reference.schema().expect("database is always supplied")),
        quote_identifier(reference.table())
    )
}

fn error(message: &str) -> DataFusionError {
    DataFusionError::Execution(format!("ClickHouse: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_scope_and_debug_are_safe() {
        let config = ClickHouseConfig::new(
            "http://localhost:8123",
            "drilling",
            "reader",
            "secret-marker",
        );
        config.validate().unwrap();
        assert!(!format!("{config:?}").contains("secret-marker"));
        let first = ClickHouse::new(config.clone()).unwrap();
        let second = ClickHouse::new(config.clone()).unwrap();
        assert_ne!(first.0.context, second.0.context);
        assert_eq!(first.0.context, first.clone().0.context);
        for endpoint in [
            "http://remote.example",
            "https://reader:secret@remote.example",
            "https://remote.example?password=x",
            "https://remote.example/path",
        ] {
            let mut invalid = config.clone();
            invalid.endpoint = endpoint.into();
            assert!(invalid.validate().is_err());
        }
    }

    #[tokio::test]
    async fn execution_is_lazy_and_timeout_closes_pending_io() {
        use datafusion::arrow::datatypes::Schema;
        use futures::StreamExt;
        use tokio::{io::AsyncReadExt, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = ClickHouseConfig::new(
            format!("http://{}", listener.local_addr().unwrap()),
            "drilling",
            "reader",
            "secret",
        );
        config.query_timeout = Duration::from_millis(100);
        let connection = ClickHouse::new(config).unwrap();
        let stream = transport::execute(
            connection.0.clone(),
            "SELECT 1".into(),
            Arc::new(Schema::empty()),
        );
        drop(stream);
        assert_eq!(connection.metrics().queries, 0);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            // Receive the request but intentionally never send a response.
            while socket.read(&mut buffer).await.unwrap() != 0 {}
        });
        let mut stream = transport::execute(
            connection.0.clone(),
            "SELECT 1".into(),
            Arc::new(Schema::empty()),
        );
        let error = stream.next().await.unwrap().unwrap_err();
        assert!(error.to_string().contains("timed out"));
        drop(stream);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(connection.metrics().queries, 1);
    }
}
