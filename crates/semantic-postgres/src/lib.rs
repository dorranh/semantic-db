mod codec;
mod execution;
mod federation;
mod metadata;
mod options;
mod predicate;
pub use execution::QueryMetrics;
pub use metadata::{PoolHealth, ServerCapabilities, TableMetadata};
pub use options::PostgresOptions;
mod tls;
mod write;
// PostgreSQL scans with bounded native cursor fetches. Ordinary scans observe independently.
use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::*,
        datatypes::*,
        record_batch::{RecordBatch, RecordBatchOptions},
    },
    catalog::{Session, TableProvider},
    error::Result,
    execution::TaskContext,
    logical_expr::{Expr, TableType},
    physical_plan::{
        ExecutionPlan, SendableRecordBatchStream, stream::RecordBatchStreamAdapter,
        streaming::PartitionStream,
    },
};
use deadpool_postgres::{Object, Pool};
use semantic_runtime::{QueryContext, QueryOptions, failure};
use std::{fmt, sync::Arc, time::Duration};
use tokio_postgres::{Row, types::Type};

pub use tls::PostgresTlsConfig;

#[derive(Clone)]
pub struct Postgres {
    pool: Pool,
    cancel_tls: Option<tokio_postgres_rustls::MakeRustlsConnect>,
    options: PostgresOptions,
    counters: Arc<execution::Counters>,
    batch_size: usize,
    domain: String,
    write_enabled: bool,
    resources: Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>,
    resource_revisions: Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>,
    receipts: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
}
impl fmt::Debug for Postgres {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Postgres")
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}
impl Postgres {
    /// Credentials are supplied by the host. The connection string must specify
    /// an explicit sslmode.
    pub fn new(url: &str, pool_size: usize, batch_size: usize) -> Result<Self> {
        Self::new_with_tls(url, pool_size, batch_size, PostgresTlsConfig::default())
    }

    /// Connect with optional custom trust roots and client identity PEMs.
    pub fn new_with_tls(
        url: &str,
        pool_size: usize,
        batch_size: usize,
        tls: PostgresTlsConfig,
    ) -> Result<Self> {
        // The legacy constructor's mandatory sslmode is an explicit transport exception.
        Self::new_with_options(
            url,
            pool_size,
            batch_size,
            tls,
            PostgresOptions {
                allow_insecure_transport: true,
                ..Default::default()
            },
        )
    }
    pub fn new_with_options(
        url: &str,
        pool_size: usize,
        batch_size: usize,
        tls: PostgresTlsConfig,
        options: PostgresOptions,
    ) -> Result<Self> {
        options.validate()?;
        if pool_size == 0 || !(1..=8192).contains(&batch_size) {
            return Err(failure("invalid Postgres pool or batch size"));
        }
        let (normalized, mode) = tls::normalize_connection_string(url)?;
        if !options.allow_insecure_transport && mode != tls::SslMode::VerifyFull {
            return Err(failure(
                "Postgres requires sslmode=verify-full; weaker modes require allow_insecure_transport",
            ));
        }
        let mut config: tokio_postgres::Config = normalized
            .parse()
            .map_err(|_| failure("invalid Postgres connection string"))?;
        config.connect_timeout(Duration::from_millis(options.connect_timeout_ms));
        // All backend sessions are read-only, including metadata inspection.
        config.options(format!("-c default_transaction_read_only=on -c search_path=pg_catalog -c timezone=UTC -c statement_timeout={} -c idle_in_transaction_session_timeout={}",options.statement_timeout_ms.min(options.query_timeout_ms),options.idle_transaction_timeout_ms));
        let (manager, cancel_tls) = tls::manager(config, mode, &tls)?;
        let pool = Pool::builder(manager)
            .max_size(pool_size)
            .build()
            .map_err(|_| failure("could not configure Postgres pool"))?;
        Ok(Self {
            pool,
            cancel_tls,
            options,
            counters: Default::default(),
            batch_size,
            domain: semantic_runtime::unique_id(),
            write_enabled: false,
            resources: Default::default(),
            resource_revisions: Default::default(),
            receipts: Default::default(),
        })
    }
    pub async fn table(&self, namespace: &str, table: &str) -> Result<Arc<dyn TableProvider>> {
        let metadata = self.metadata(namespace, table).await?;
        self.bind_provider(namespace, table, metadata)
    }
    fn bind_provider(
        &self,
        namespace: &str,
        table: &str,
        metadata: TableMetadata,
    ) -> Result<Arc<dyn TableProvider>> {
        let target = format!("{}.{}", identifier(namespace)?, identifier(table)?);
        let fallback = Arc::new(PgTable {
            postgres: self.clone(),
            target: target.clone(),
            schema: metadata.schema.clone(),
            metadata,
        });
        if self.options.federation && self.options.filter_pushdown {
            federation::provider(self.clone(), namespace, table, fallback)
        } else {
            Ok(fallback)
        }
    }
}
fn identifier(s: &str) -> Result<String> {
    if s.is_empty() || s.contains('\0') {
        return Err(failure(
            "Postgres schema/table must be a nonempty identifier",
        ));
    }
    Ok(format!("\"{}\"", s.replace('"', "\"\"")))
}
#[derive(Debug)]
struct PgTable {
    postgres: Postgres,
    target: String,
    schema: SchemaRef,
    metadata: TableMetadata,
}
#[async_trait]
impl TableProvider for PgTable {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    fn statistics(&self) -> Option<datafusion::common::Statistics> {
        let mut stats = datafusion::common::Statistics::new_unknown(&self.schema);
        if let Some(n) = self.metadata.estimated_rows {
            stats.num_rows = datafusion::common::stats::Precision::Inexact(n);
        }
        if let Some(n) = self.metadata.estimated_bytes {
            stats.total_byte_size = datafusion::common::stats::Precision::Inexact(n);
        }
        Some(stats)
    }
    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<datafusion::logical_expr::TableProviderFilterPushDown>> {
        Ok(filters
            .iter()
            .map(|e| predicate::classify(e, &self.schema, &self.postgres.options))
            .collect())
    }
    async fn scan(
        &self,
        _: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let (sql, schema, parameters) = predicate::scan_sql(
            &self.target,
            &self.schema,
            projection,
            filters,
            limit,
            &self.postgres.options,
        )?;
        Ok(Arc::new(execution::RemoteExec::new(
            self.postgres.clone(),
            sql,
            schema,
            parameters,
            vec![(self.target.clone(), self.metadata.revision.clone())],
            false,
        )))
    }
}
// Incomplete work is never recycled. Closing the driver ends server cursors and transactions.
struct Lease {
    client: Option<Object>,
    clean: bool,
    cancel_tls: Option<tokio_postgres_rustls::MakeRustlsConnect>,
}
impl Postgres {
    fn lease(&self, client: Object) -> Lease {
        Lease {
            client: Some(client),
            clean: false,
            cancel_tls: self.cancel_tls.clone(),
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.clean
            && let Some(client) = self.client.take()
        {
            let token = client.cancel_token();
            // Release the pool slot now, but retain the driver until cancellation
            // finishes so this backend identity cannot be recycled in the meantime.
            let driver = Object::take(client);
            let tls = self.cancel_tls.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = tokio::time::timeout(Duration::from_secs(2), async {
                        match tls {
                            Some(tls) => token.cancel_query(tls).await,
                            None => token.cancel_query(tokio_postgres::NoTls).await,
                        }
                    })
                    .await;
                    drop(driver);
                });
            } else {
                drop(driver);
            }
        }
    }
}

fn arrow_type(ty: &Type) -> Result<DataType> {
    Ok(match *ty {
        Type::TEXT | Type::VARCHAR | Type::JSONB => DataType::Utf8,
        Type::NUMERIC => DataType::Decimal128(38, 10),
        Type::UUID => DataType::FixedSizeBinary(16),
        Type::BYTEA => DataType::Binary,
        Type::TIME => DataType::Time64(TimeUnit::Microsecond),
        Type::INTERVAL => DataType::Interval(IntervalUnit::MonthDayNano),
        Type::BOOL_ARRAY
        | Type::INT2_ARRAY
        | Type::INT4_ARRAY
        | Type::INT8_ARRAY
        | Type::TEXT_ARRAY
        | Type::VARCHAR_ARRAY => {
            let tokio_postgres::types::Kind::Array(element) = ty.kind() else {
                unreachable!()
            };
            DataType::List(Arc::new(Field::new("item", arrow_type(element)?, true)))
        }
        Type::BOOL => DataType::Boolean,
        Type::INT2 => DataType::Int16,
        Type::INT4 => DataType::Int32,
        Type::INT8 => DataType::Int64,
        Type::FLOAT4 => DataType::Float32,
        Type::FLOAT8 => DataType::Float64,
        Type::DATE => DataType::Date32,
        Type::TIMESTAMP => DataType::Timestamp(TimeUnit::Microsecond, None),
        Type::TIMESTAMPTZ => DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        _ => {
            return Err(failure(&format!(
                "unsupported Postgres type: {}",
                ty.name()
            )));
        }
    })
}
fn batch(schema: &SchemaRef, rows: &[Row]) -> Result<RecordBatch> {
    let arrays = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(i, field)| -> Result<ArrayRef> {
            macro_rules! values {
                ($t:ty) => {
                    rows.iter()
                        .map(|r| {
                            r.try_get::<_, Option<$t>>(i)
                                .map_err(|_| failure("Postgres value conversion failed"))
                        })
                        .collect::<Result<Vec<_>>>()?
                };
            }
            Ok(match field.data_type() {
                DataType::Utf8
                    if rows
                        .first()
                        .is_some_and(|r| r.columns()[i].type_() == &Type::JSONB) =>
                {
                    let values = rows
                        .iter()
                        .map(|r| {
                            r.try_get::<_, Option<codec::Raw<'_>>>(i)
                                .map_err(|_| failure("Postgres JSONB decoding failed"))?
                                .map(|v| {
                                    if v.0.first() != Some(&1) {
                                        return Err(failure("unsupported Postgres JSONB version"));
                                    }
                                    std::str::from_utf8(&v.0[1..])
                                        .map(str::to_owned)
                                        .map_err(|_| failure("invalid Postgres JSONB encoding"))
                                })
                                .transpose()
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Arc::new(StringArray::from(values))
                }
                DataType::Utf8 => Arc::new(StringArray::from(values!(String))),
                DataType::Boolean => Arc::new(BooleanArray::from(values!(bool))),
                DataType::Int16 => Arc::new(Int16Array::from(values!(i16))),
                DataType::Int32 => Arc::new(Int32Array::from(values!(i32))),
                DataType::Int64
                    if rows
                        .first()
                        .is_some_and(|r| r.columns()[i].type_() == &Type::NUMERIC) =>
                {
                    Arc::new(Int64Array::from(
                        rows.iter()
                            .map(|r| {
                                codec::numeric_at(r, i, 38, 0)?
                                    .map(|v| {
                                        i64::try_from(v).map_err(|_| {
                                            failure("Postgres aggregate integer overflow")
                                        })
                                    })
                                    .transpose()
                            })
                            .collect::<Result<Vec<_>>>()?,
                    ))
                }
                DataType::Int64 => Arc::new(Int64Array::from(values!(i64))),
                DataType::UInt64 => Arc::new(UInt64Array::from(
                    values!(i64)
                        .into_iter()
                        .map(|v| {
                            v.map(|n| {
                                u64::try_from(n)
                                    .map_err(|_| failure("negative Postgres window result"))
                            })
                            .transpose()
                        })
                        .collect::<Result<Vec<_>>>()?,
                )),
                DataType::Float32 => Arc::new(Float32Array::from(values!(f32))),
                DataType::Float64 => Arc::new(Float64Array::from(values!(f64))),
                DataType::Date32 => {
                    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
                    Arc::new(Date32Array::from(
                        values!(chrono::NaiveDate)
                            .into_iter()
                            .map(|v| v.map(|d| (d - epoch).num_days() as i32))
                            .collect::<Vec<_>>(),
                    ))
                }
                DataType::Timestamp(_, None) => Arc::new(TimestampMicrosecondArray::from(
                    values!(chrono::NaiveDateTime)
                        .into_iter()
                        .map(|v| v.map(|d| d.and_utc().timestamp_micros()))
                        .collect::<Vec<_>>(),
                )),
                DataType::Timestamp(_, Some(_)) => Arc::new(
                    TimestampMicrosecondArray::from(
                        values!(chrono::DateTime<chrono::Utc>)
                            .into_iter()
                            .map(|v| v.map(|d| d.timestamp_micros()))
                            .collect::<Vec<_>>(),
                    )
                    .with_timezone("UTC"),
                ),
                _ => codec::extra_array(field, rows, i)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    RecordBatch::try_new_with_options(
        schema.clone(),
        arrays,
        &RecordBatchOptions::new().with_row_count(Some(rows.len())),
    )
    .map_err(Into::into)
}
