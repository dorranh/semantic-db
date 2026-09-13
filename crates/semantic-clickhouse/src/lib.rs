//! Read-only ClickHouse tables and finalized views over Arrow IPC.
//!
//! Core DataFusion federation owns remote subplans. This connector owns the
//! client, schema discovery, bounded streams and a conservative capability policy.
//! AggregateFunction state columns must be finalized in a ClickHouse view first.

mod execution;
mod http;
mod metadata;
mod options;
mod policy;
mod runtime_filter;
mod transport;
pub use metadata::{ColumnMetadata, ServerCapabilities, TableMetadata};
pub use options::{ArrowCodec, ClickHouseConfig, ServerLimits, TableOptions};

use async_trait::async_trait;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::{Session, TableProvider},
    common::TableReference,
    error::{DataFusionError, Result},
    logical_expr::TableType,
    physical_plan::{ExecutionPlan, SendableRecordBatchStream},
};
use datafusion_federation::{
    FederatedTableProviderAdaptor,
    sql::{AstAnalyzer, LogicalOptimizer, SQLExecutor, SQLFederationProvider, SQLTableSource},
};

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

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
    client: reqwest::Client,
    permits: tokio::sync::Semaphore,
    config: ClickHouseConfig,
    context: String,
    counters: Counters,
    metadata:
        std::sync::Mutex<std::collections::BTreeMap<String, (std::time::Instant, TableMetadata)>>,
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
        let client = http::client(&config)?;
        let permits = tokio::sync::Semaphore::new(config.max_concurrent_requests);
        Ok(Self(Arc::new(Connection {
            client,
            permits,
            config,
            context,
            counters: Counters::default(),
            metadata: std::sync::Mutex::new(std::collections::BTreeMap::new()),
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
        self.table_with_options(table, TableOptions::default())
            .await
    }
    pub async fn table_with_options(
        &self,
        table: &str,
        options: TableOptions,
    ) -> Result<Arc<dyn TableProvider>> {
        validate_identifier(table)?;
        options.validate()?;
        let metadata = self.metadata(table).await?;
        if options.final_read
            && ![
                "ReplacingMergeTree",
                "CollapsingMergeTree",
                "SummingMergeTree",
                "AggregatingMergeTree",
            ]
            .iter()
            .any(|engine| metadata.engine.ends_with(engine))
        {
            return Err(error(
                "FINAL requires a merge engine supporting finalization",
            ));
        }
        for column in &metadata.columns {
            if options
                .columns
                .as_ref()
                .is_none_or(|names| names.contains(&column.name))
                && (column.native_type.starts_with("AggregateFunction(")
                    || column.native_type.contains("(AggregateFunction("))
            {
                return Err(error(
                    "raw AggregateFunction states require a finalized view",
                ));
            }
        }
        let reference = TableReference::partial(self.0.config.database.clone(), table.to_owned());
        let columns = options
            .columns
            .as_ref()
            .map(|names| {
                names
                    .iter()
                    .map(|name| quote_identifier(name))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "*".into());
        let sql = format!(
            "SELECT {columns} FROM {}{} WHERE false LIMIT 0",
            remote_name(&reference),
            if options.final_read { " FINAL" } else { "" }
        );
        let schema = transport::schema(&self.0, &sql).await?;
        for field in schema.fields() {
            validate_identifier(field.name())?;
        }
        let fallback = Arc::new(ScanTable {
            connection: self.0.clone(),
            reference: reference.clone(),
            schema: schema.clone(),
            options: options.clone(),
            estimated_rows: if options.final_read {
                None
            } else {
                metadata.total_rows.and_then(|n| usize::try_from(n).ok())
            },
        });
        if !self.0.config.federation {
            return Ok(fallback);
        }
        let executor = Arc::new(Executor(self.0.clone()));
        let provider = Arc::new(SQLFederationProvider::new(executor));
        let source = Arc::new(SQLTableSource::new_with_table(
            provider,
            Arc::new(BoundTable {
                reference,
                schema,
                options,
            }),
        ));
        Ok(Arc::new(FederatedTableProviderAdaptor::new_with_provider(
            source, fallback,
        )))
    }
}

#[derive(Debug)]
struct BoundTable {
    reference: TableReference,
    schema: SchemaRef,
    options: TableOptions,
}
impl datafusion_federation::sql::SQLTable for BoundTable {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn table_reference(&self) -> TableReference {
        self.reference.clone()
    }
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
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
        let planner = Arc::new(execution::RemotePlanner(self.0.clone()));
        Some(Box::new(move |plan| {
            policy::restrict_federation(plan, planner.clone())
        }))
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
        ClickHouse(self.0.clone()).table_names().await
    }
    async fn get_table_schema(&self, table_name: &str) -> Result<SchemaRef> {
        Ok(ClickHouse(self.0.clone()).table(table_name).await?.schema())
    }
}

#[derive(Debug)]
struct ScanTable {
    estimated_rows: Option<usize>,
    connection: Arc<Connection>,
    reference: TableReference,
    schema: SchemaRef,
    options: TableOptions,
}

#[async_trait]
impl TableProvider for ScanTable {
    fn statistics(&self) -> Option<datafusion::common::Statistics> {
        let mut stats = datafusion::common::Statistics::new_unknown(&self.schema);
        if let Some(rows) = self.estimated_rows {
            stats.num_rows = datafusion::common::stats::Precision::Inexact(rows);
        }
        Some(stats)
    }
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    fn supports_filters_pushdown(
        &self,
        filters: &[&datafusion::logical_expr::Expr],
    ) -> Result<Vec<datafusion::logical_expr::TableProviderFilterPushDown>> {
        use datafusion::logical_expr::TableProviderFilterPushDown::{Exact, Unsupported};
        Ok(filters
            .iter()
            .map(|expr| {
                if self.connection.config.filter_pushdown
                    && policy::supports_filter(expr, &self.schema)
                {
                    Exact
                } else {
                    Unsupported
                }
            })
            .collect())
    }
    async fn scan(
        &self,
        _session: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let indices = projection
            .cloned()
            .unwrap_or_else(|| (0..self.schema.fields().len()).collect());
        let schema = Arc::new(self.schema.project(&indices)?);
        let zero = indices.is_empty();
        let columns = if zero {
            "1 AS __semantic_row".into()
        } else {
            indices
                .iter()
                .map(|i| quote_identifier(self.schema.field(*i).name()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut sql = format!(
            "SELECT {columns} FROM {}{}",
            remote_name(&self.reference),
            if self.options.final_read {
                " FINAL"
            } else {
                ""
            }
        );
        if !filters.is_empty() {
            if filters
                .iter()
                .any(|f| !policy::supports_filter(f, &self.schema))
            {
                return Err(error("unsupported pushed filter"));
            }
            sql.push_str(" WHERE ");
            sql.push_str(
                &filters
                    .iter()
                    .map(|f| policy::filter_sql(f).map(|s| format!("({s})")))
                    .collect::<Result<Vec<_>>>()?
                    .join(" AND "),
            );
        }
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        Ok(Arc::new(execution::RemoteExec::new(
            self.connection.clone(),
            sql,
            schema,
            zero,
        )))
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
    use std::time::Duration;
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
