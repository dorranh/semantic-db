use crate::*;
use datafusion::{
    execution::memory_pool::MemoryConsumer,
    physical_expr::EquivalenceProperties,
    physical_plan::{
        DisplayAs, DisplayFormatType, Partitioning, PlanProperties,
        execution_plan::{Boundedness, EmissionType},
        metrics::{ExecutionPlanMetricsSet, MetricBuilder, MetricsSet},
    },
};
use futures::StreamExt;
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Debug, Default)]
pub(crate) struct Counters {
    pub executions: AtomicU64,
    pub rows: AtomicU64,
    pub decoded_bytes: AtomicU64,
    pub estimated_wire_bytes: AtomicU64,
    pub fetches: AtomicU64,
}
#[derive(Debug, Clone, Copy)]
pub struct QueryMetrics {
    pub executions: u64,
    pub rows: u64,
    pub decoded_bytes: u64,
    pub estimated_wire_bytes: u64,
    pub fetches: u64,
}
impl Postgres {
    pub fn metrics(&self) -> QueryMetrics {
        let c = &self.counters;
        QueryMetrics {
            executions: c.executions.load(Ordering::Relaxed),
            rows: c.rows.load(Ordering::Relaxed),
            decoded_bytes: c.decoded_bytes.load(Ordering::Relaxed),
            estimated_wire_bytes: c.estimated_wire_bytes.load(Ordering::Relaxed),
            fetches: c.fetches.load(Ordering::Relaxed),
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct RemoteExec {
    pg: Postgres,
    sql: String,
    schema: SchemaRef,
    parameters: predicate::Parameters,
    revisions: Vec<(String, String)>,
    delegated: bool,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}
impl RemoteExec {
    pub fn new(
        pg: Postgres,
        sql: String,
        schema: SchemaRef,
        parameters: predicate::Parameters,
        revisions: Vec<(String, String)>,
        delegated: bool,
    ) -> Self {
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(schema.clone()),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Self {
            pg,
            sql,
            schema,
            parameters,
            revisions,
            delegated,
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        }
    }
}
impl DisplayAs for RemoteExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "PostgresExec delegated={} sql={}",
            self.delegated, self.sql
        )
    }
}
impl ExecutionPlan for RemoteExec {
    fn name(&self) -> &str {
        "PostgresExec"
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }
    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !children.is_empty() {
            return Err(failure("Postgres execution cannot have children"));
        }
        Ok(self)
    }
    fn apply_expressions(
        &self,
        _: &mut dyn FnMut(
            &Arc<dyn datafusion::physical_expr::PhysicalExpr>,
        ) -> Result<datafusion::common::tree_node::TreeNodeRecursion>,
    ) -> Result<datafusion::common::tree_node::TreeNodeRecursion> {
        Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
    }
    fn metrics(&self) -> Option<MetricsSet> {
        Some(self.metrics.clone_inner())
    }
    fn execute(
        &self,
        partition: usize,
        task: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        if partition != 0 {
            return Err(failure("invalid Postgres partition"));
        }
        let pg = self.pg.clone();
        let sql = self.sql.clone();
        let schema = self.schema.clone();
        let output = schema.clone();
        let parameters = self.parameters.clone();
        let revisions = self.revisions.clone();
        let metrics = self.metrics.clone();
        let delegated = self.delegated;
        let stream = async_stream::try_stream! {
            let query = QueryContext::from_task(&task).map(Ok).unwrap_or_else(|| {
                QueryContext::new(QueryOptions {
                    timeout_seconds: pg.options.query_timeout_ms.div_ceil(1000),
                    ..Default::default()
                })
            })?;
            let deadline = query.deadline().min(
                tokio::time::Instant::now() + Duration::from_millis(pg.options.query_timeout_ms),
            );
            let started = std::time::Instant::now();
            let fetches = MetricBuilder::new(&metrics).counter("fetches", 0);
            let output_rows = MetricBuilder::new(&metrics).output_rows(0);
            let decoded_bytes = MetricBuilder::new(&metrics).counter("decoded_bytes", 0);
            let estimated_wire_bytes =
                MetricBuilder::new(&metrics).counter("estimated_wire_bytes", 0);
            let elapsed = MetricBuilder::new(&metrics).elapsed_compute(0);
            let _timer = elapsed.timer();
            let acquired = bounded(&query, deadline, pg.acquire()).await;
            MetricBuilder::new(&metrics)
                .counter("queue_ns", 0)
                .add(started.elapsed().as_nanos().min(usize::MAX as u128) as usize);
            let client = acquired?;
            let mut lease = pg.lease(client);
            let client = lease.client.as_mut().unwrap();
            let tx = bounded(&query, deadline, async {
                client
                    .transaction()
                    .await
                    .map_err(|_| failure("Postgres read transaction failed"))
            })
            .await?;
            set_timeouts(tx.client(), &pg, &query, deadline).await?;
            // Locks prevent DDL between validation and execution. Views may change
            // underlying data normally, but their visible column contract is checked.
            for (target, revision) in &revisions {
                bounded(&query, deadline, async {
                    tx.batch_execute(&format!("LOCK TABLE {target} IN ACCESS SHARE MODE"))
                        .await
                        .map_err(|_| failure("Postgres schema lock failed"))
                })
                .await?;
                let actual =
                    bounded(&query, deadline, metadata::inspect(tx.client(), target)).await?;
                if &actual.revision != revision {
                    Err(failure("Postgres schema changed; refresh source binding"))?;
                }
            }
            let statement = bounded(&query, deadline, async {
                tx.prepare(&sql)
                    .await
                    .map_err(|_| failure("Postgres scan planning failed"))
            })
            .await?;
            validate_result(&schema, &statement, delegated)?;
            let values = parameters.bind(statement.params())?;
            let refs = values
                .iter()
                .map(|v| v.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect::<Vec<_>>();
            let portal = bounded(&query, deadline, async {
                tx.bind(&statement, &refs)
                    .await
                    .map_err(|_| failure("Postgres cursor creation failed"))
            })
            .await?;
            pg.counters.executions.fetch_add(1, Ordering::Relaxed);
            MetricBuilder::new(&metrics)
                .counter("remote_executions", 0)
                .add(1);
            let reservation = MemoryConsumer::new("Postgres decode").register(task.memory_pool());
            let mut first = true;
            loop {
                query.check()?;
                if tokio::time::Instant::now() >= deadline {
                    Err(failure("Postgres query timed out"))?;
                }
                query.request_started()?;
                pg.counters.fetches.fetch_add(1, Ordering::Relaxed);
                fetches.add(1);
                let stream = bounded(&query, deadline, async {
                    tx.query_portal_raw(&portal, pg.batch_size as i32)
                        .await
                        .map_err(|_| failure("Postgres fetch failed"))
                })
                .await?;
                tokio::pin!(stream);
                let mut rows = vec![];
                let mut admitted = 1024usize;
                let mut wire = 0usize;
                while let Some(row) = bounded(&query, deadline, async {
                    tokio::time::timeout_at(deadline, stream.next())
                        .await
                        .map_err(|_| failure("Postgres query timed out"))?
                        .transpose()
                        .map_err(|_| failure("Postgres scan failed"))
                })
                .await?
                {
                    let bytes =
                        codec::admitted_size(std::slice::from_ref(&row))?.saturating_sub(1024);
                    admitted = admitted
                        .checked_add(bytes)
                        .ok_or_else(|| failure("Postgres batch size overflow"))?;
                    if admitted > pg.options.max_batch_bytes {
                        Err(failure("Postgres batch memory budget exhausted"))?;
                    }
                    reservation.try_resize(admitted)?;
                    wire = wire.saturating_add(estimated_wire_size(&row)?);
                    rows.push(row);
                }
                if rows.is_empty() {
                    break;
                }
                query.charge_remote_estimated(wire)?;
                // Admission happens before constructing arrays; conservatively includes scratch.
                query.charge_decoded(admitted)?;
                let batch = batch(&schema, &rows)?;
                validate_nulls(&batch)?;
                if batch.get_array_memory_size() > admitted {
                    Err(failure(&format!(
                        "Postgres decode reservation exceeded: {} > {admitted}",
                        batch.get_array_memory_size()
                    )))?;
                }
                pg.counters
                    .rows
                    .fetch_add(rows.len() as u64, Ordering::Relaxed);
                pg.counters
                    .decoded_bytes
                    .fetch_add(batch.get_array_memory_size() as u64, Ordering::Relaxed);
                pg.counters
                    .estimated_wire_bytes
                    .fetch_add(wire as u64, Ordering::Relaxed);
                output_rows.add(rows.len());
                decoded_bytes.add(batch.get_array_memory_size());
                estimated_wire_bytes.add(wire);
                if first {
                    MetricBuilder::new(&metrics)
                        .counter("first_batch_ns", 0)
                        .add(started.elapsed().as_nanos().min(usize::MAX as u128) as usize);
                    first = false;
                }
                drop(rows);
                yield batch;
                reservation.try_resize(0)?;
            }
            bounded(&query, deadline, async {
                tx.rollback()
                    .await
                    .map_err(|_| failure("Postgres cleanup failed"))
            })
            .await?;
            lease.clean = true;
        };
        Ok(Box::pin(RecordBatchStreamAdapter::new(output, stream)))
    }
}
pub(crate) async fn set_timeouts(
    c: &tokio_postgres::Client,
    pg: &Postgres,
    query: &QueryContext,
    deadline: tokio::time::Instant,
) -> Result<()> {
    let ms = deadline
        .saturating_duration_since(tokio::time::Instant::now())
        .as_millis()
        .max(1)
        .min(pg.options.statement_timeout_ms as u128);
    bounded(query,deadline,async {c.batch_execute(&format!("SET LOCAL statement_timeout='{ms}ms'; SET LOCAL idle_in_transaction_session_timeout='{}ms'",pg.options.idle_transaction_timeout_ms)).await.map_err(|_|failure("Postgres timeout configuration failed"))}).await
}
pub(crate) fn estimated_wire_size(row: &Row) -> Result<usize> {
    (0..row.len()).try_fold(7usize, |n, i| {
        Ok(n.saturating_add(4).saturating_add(
            row.try_get::<_, Option<codec::Raw<'_>>>(i)
                .map_err(|_| failure("Postgres value inspection failed"))?
                .map_or(0, |r| r.0.len()),
        ))
    })
}
fn validate_result(
    schema: &Schema,
    statement: &tokio_postgres::Statement,
    delegated: bool,
) -> Result<()> {
    if schema.fields().is_empty() {
        return Ok(());
    }
    if schema.fields().len() != statement.columns().len() {
        return Err(failure("Postgres result column count changed"));
    }
    for (field, column) in schema.fields().iter().zip(statement.columns()) {
        let actual = arrow_type(column.type_())?;
        if &actual != field.data_type()
            && !(matches!(
                (field.data_type(), &actual),
                (DataType::Decimal128(..), DataType::Decimal128(..))
            ) || delegated
                && ((column.type_() == &Type::NUMERIC && field.data_type() == &DataType::Int64)
                    || (column.type_() == &Type::INT8 && field.data_type() == &DataType::UInt64)))
        {
            return Err(failure(
                "Postgres result type changed; refresh source binding",
            ));
        }
    }
    Ok(())
}
pub(crate) fn validate_nulls(batch: &RecordBatch) -> Result<()> {
    if batch
        .schema()
        .fields()
        .iter()
        .zip(batch.columns())
        .any(|(f, a)| !f.is_nullable() && a.null_count() != 0)
    {
        return Err(failure("Postgres returned NULL for a required column"));
    }
    Ok(())
}

/// Pinned sessions retain their own execution path and transaction lease.
#[derive(Debug)]
pub(crate) struct SessionExec {
    partition: Arc<dyn PartitionStream>,
    sql: String,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}
impl SessionExec {
    pub(crate) fn new(
        partition: Arc<dyn PartitionStream>,
        sql: String,
        metrics: ExecutionPlanMetricsSet,
    ) -> Self {
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(partition.schema().clone()),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Final,
            Boundedness::Bounded,
        ));
        Self {
            partition,
            sql,
            properties,
            metrics,
        }
    }
}
impl DisplayAs for SessionExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PostgresSessionExec pinned=true sql={}", self.sql)
    }
}
impl ExecutionPlan for SessionExec {
    fn name(&self) -> &str {
        "PostgresSessionExec"
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }
    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !children.is_empty() {
            return Err(failure("Postgres session cannot have children"));
        }
        Ok(self)
    }
    fn execute(
        &self,
        partition: usize,
        task: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        if partition != 0 {
            return Err(failure("invalid Postgres session partition"));
        }
        Ok(self.partition.execute(task))
    }
    fn metrics(&self) -> Option<MetricsSet> {
        Some(self.metrics.clone_inner())
    }
    fn apply_expressions(
        &self,
        _: &mut dyn FnMut(
            &Arc<dyn datafusion::physical_expr::PhysicalExpr>,
        ) -> Result<datafusion::common::tree_node::TreeNodeRecursion>,
    ) -> Result<datafusion::common::tree_node::TreeNodeRecursion> {
        Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
    }
}

pub(crate) async fn bounded<T>(
    query: &QueryContext,
    deadline: tokio::time::Instant,
    operation: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    query
        .run(async {
            tokio::time::timeout_at(deadline, operation)
                .await
                .map_err(|_| failure("Postgres query timed out"))?
        })
        .await
}
