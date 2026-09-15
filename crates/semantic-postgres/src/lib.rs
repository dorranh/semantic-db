//! Read-only PostgreSQL scans with bounded native cursor fetches. A scan owns a
//! connection; separate scans make independent observations, not a shared snapshot.
use async_trait::async_trait;
use datafusion::{
    arrow::{
        array::*,
        datatypes::*,
        record_batch::{RecordBatch, RecordBatchOptions},
    },
    catalog::{Session, TableProvider, streaming::StreamingTable},
    error::Result,
    execution::TaskContext,
    logical_expr::{Expr, TableType},
    physical_plan::{
        ExecutionPlan, SendableRecordBatchStream, stream::RecordBatchStreamAdapter,
        streaming::PartitionStream,
    },
};
use deadpool_postgres::{Manager, Object, Pool};
use semantic_runtime::{QueryContext, QueryOptions, failure};
use std::{fmt, sync::Arc, time::Duration};
use tokio_postgres::{NoTls, Row, types::Type};

#[derive(Clone)]
pub struct Postgres {
    pool: Pool,
    batch_size: usize,
}
impl fmt::Debug for Postgres {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Postgres")
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}
impl Postgres {
    /// Credentials are supplied by the host. This initial connector requires an
    /// explicitly plaintext local/private connection (sslmode=disable).
    pub fn new(url: &str, pool_size: usize, batch_size: usize) -> Result<Self> {
        if pool_size == 0 || !(1..=8192).contains(&batch_size) {
            return Err(failure("invalid Postgres pool or batch size"));
        }
        let mut config: tokio_postgres::Config = url
            .parse()
            .map_err(|_| failure("invalid Postgres connection string"))?;
        if config.get_ssl_mode() != tokio_postgres::config::SslMode::Disable {
            return Err(failure(
                "Postgres connector currently requires sslmode=disable; TLS is not implemented",
            ));
        }
        config.connect_timeout(Duration::from_secs(10));
        // All backend sessions are read-only, including metadata inspection.
        config.options("-c default_transaction_read_only=on -c statement_timeout=30000 -c idle_in_transaction_session_timeout=30000");
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(pool_size)
            .build()
            .map_err(|_| failure("could not configure Postgres pool"))?;
        Ok(Self { pool, batch_size })
    }
    pub async fn table(&self, namespace: &str, table: &str) -> Result<Arc<dyn TableProvider>> {
        let target = format!("{}.{}", identifier(namespace)?, identifier(table)?);
        let client = tokio::time::timeout(Duration::from_secs(10), self.pool.get())
            .await
            .map_err(|_| failure("Postgres metadata connection timed out"))?
            .map_err(|_| failure("Postgres connection failed"))?;
        let statement = client
            .prepare(&format!("SELECT * FROM {target} LIMIT 0"))
            .await
            .map_err(|_| failure("Postgres table inspection failed"))?;
        let schema = Arc::new(Schema::new(
            statement
                .columns()
                .iter()
                .map(|c| Ok(Field::new(c.name(), arrow_type(c.type_())?, true)))
                .collect::<Result<Vec<_>>>()?,
        ));
        Ok(Arc::new(PgTable {
            postgres: self.clone(),
            target,
            schema,
        }))
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
}
#[async_trait]
impl TableProvider for PgTable {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
    fn table_type(&self) -> TableType {
        TableType::Base
    }
    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        _: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let indices = projection
            .cloned()
            .unwrap_or_else(|| (0..self.schema.fields().len()).collect());
        let schema = Arc::new(self.schema.project(&indices)?);
        let columns = indices
            .iter()
            .map(|i| identifier(self.schema.field(*i).name()))
            .collect::<Result<Vec<_>>>()?;
        let selection = if columns.is_empty() {
            "1 AS __row".into()
        } else {
            columns.join(", ")
        };
        let mut sql = format!("SELECT {selection} FROM {}", self.target);
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        let partition = Arc::new(PgPartition {
            postgres: self.postgres.clone(),
            sql,
            schema: schema.clone(),
        });
        StreamingTable::try_new(schema, vec![partition])?
            .scan(state, None, &[], None)
            .await
    }
}
#[derive(Debug)]
struct PgPartition {
    postgres: Postgres,
    sql: String,
    schema: SchemaRef,
}
// A cancelled/incomplete scan removes its connection from the pool. Dropping the
// wrapper closes its driver, terminating any native cursor/transaction.
struct Lease {
    client: Option<Object>,
    clean: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.clean
            && let Some(client) = self.client.take()
        {
            drop(Object::take(client));
        }
    }
}
impl PartitionStream for PgPartition {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }
    fn execute(&self, task: Arc<TaskContext>) -> SendableRecordBatchStream {
        let pool = self.postgres.pool.clone();
        let size = self.postgres.batch_size;
        let sql = self.sql.clone();
        let schema = self.schema.clone();
        let output_schema = schema.clone();
        let query = QueryContext::from_task(&task)
            .unwrap_or_else(|| QueryContext::new(QueryOptions::default()).expect("valid default"));
        let stream = async_stream::try_stream! {
            let client = query.run(async { pool.get().await.map_err(|_| failure("Postgres connection failed")) }).await?;
            let mut lease = Lease { client: Some(client), clean: false };
            let client = lease.client.as_mut().expect("owned client");
            let tx = query.run(async { client.transaction().await.map_err(|_| failure("Postgres read transaction failed")) }).await?;
            let statement = query.run(async { tx.prepare(&sql).await.map_err(|_| failure("Postgres scan planning failed; reload after schema changes")) }).await?;
            if !schema.fields().is_empty() {
                let actual = statement.columns().iter().map(|c| Ok(Field::new(c.name(), arrow_type(c.type_())?, true))).collect::<Result<Vec<_>>>()?;
                if Schema::new(actual) != *schema { Err(failure("Postgres schema changed; reload project"))?; }
            }
            let portal = query.run(async { tx.bind(&statement, &[]).await.map_err(|_| failure("Postgres cursor creation failed")) }).await?;
            loop {
                query.request_started()?;
                let rows = query.run(async { tx.query_portal(&portal, size as i32).await.map_err(|_| failure("Postgres scan failed")) }).await?;
                if rows.is_empty() { break; }
                let batch = batch(&schema, &rows)?;
                query.charge_decoded(batch.get_array_memory_size())?;
                // Native cursor fetches are bounded by rows. Account the decoded
                // representation; wire byte accounting is unavailable in this client.
                query.charge_remote(batch.get_array_memory_size())?;
                yield batch;
            }
            query.run(async { tx.rollback().await.map_err(|_| failure("Postgres read cleanup failed")) }).await?;
            lease.clean = true;
        };
        Box::pin(RecordBatchStreamAdapter::new(output_schema, stream))
    }
}
fn arrow_type(ty: &Type) -> Result<DataType> {
    Ok(match *ty {
        Type::TEXT | Type::VARCHAR => DataType::Utf8,
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
                DataType::Utf8 => Arc::new(StringArray::from(values!(String))),
                DataType::Boolean => Arc::new(BooleanArray::from(values!(bool))),
                DataType::Int16 => Arc::new(Int16Array::from(values!(i16))),
                DataType::Int32 => Arc::new(Int32Array::from(values!(i32))),
                DataType::Int64 => Arc::new(Int64Array::from(values!(i64))),
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
                _ => return Err(failure("unsupported Postgres value type")),
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
