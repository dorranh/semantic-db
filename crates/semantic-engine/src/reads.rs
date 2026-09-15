use crate::*;
use datafusion::{
    arrow::datatypes::SchemaRef,
    common::ScalarValue,
    physical_plan::{RecordBatchStream, SendableRecordBatchStream},
    prelude::SessionContext,
};
use futures::{Stream, StreamExt};
use semantic_runtime::failure;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ReadCompletion {
    Pending,
    Complete,
    Failed,
    Cancelled,
    Abandoned,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReadReport {
    pub requested: ReadConsistency,
    pub established: ReadConsistency,
    pub resources: Vec<String>,
    pub domains: Vec<String>,
    pub external_observations: Vec<String>,
    pub snapshot: Option<String>,
    pub read_your_writes: bool,
    pub cache_bypassed: bool,
    pub caches: Vec<semantic_runtime::CacheObservation>,
    pub satisfied_receipts: Vec<CommitReceipt>,
    pub completion: ReadCompletion,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReadExplanation {
    pub consistency: ReadConsistency,
    pub dependencies: Vec<String>,
    pub domains: Vec<String>,
    pub cache_bypassed: bool,
    pub pending_checks: Vec<String>,
}
pub struct ReadResult {
    pub batches: Vec<RecordBatch>,
    pub report: ReadReport,
}
pub struct ReadExecution {
    pub stream: SendableRecordBatchStream,
    pub context: Arc<QueryContext>,
    pub report: Arc<Mutex<ReadReport>>,
}
impl ReadExecution {
    pub fn cancel(&self) {
        self.context.cancel();
    }
    pub async fn collect(mut self) -> Result<ReadResult> {
        let mut batches = vec![];
        let mut bytes = 0usize;
        while let Some(b) = self.stream.next().await {
            let b = b?;
            bytes = bytes.saturating_add(b.get_array_memory_size());
            if bytes > self.context.options.max_decoded_bytes {
                self.report.lock().unwrap().completion = ReadCompletion::Failed;
                self.context.cancel();
                return Err(
                    failure("collected read byte budget exhausted; no complete result").into(),
                );
            }
            batches.push(b);
        }
        let report = self.report.lock().unwrap().clone();
        if report.completion != ReadCompletion::Complete {
            return Err(failure("read did not complete successfully").into());
        }
        Ok(ReadResult { batches, report })
    }
}
struct GuardedRead {
    inner: Option<SendableRecordBatchStream>,
    schema: SchemaRef,
    report: Arc<Mutex<ReadReport>>,
    context: Arc<QueryContext>,
    session: Option<Arc<dyn ConnectorSession>>,
}
impl Stream for GuardedRead {
    type Item = datafusion::error::Result<RecordBatch>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(inner) = self.inner.as_mut() else {
            return Poll::Ready(None);
        };
        let result = inner.as_mut().poll_next(cx);
        match &result {
            Poll::Ready(None) => {
                let mut report = self.report.lock().unwrap();
                if report.completion == ReadCompletion::Pending {
                    report.completion = ReadCompletion::Complete;
                }
                drop(report);
                self.inner.take();
                self.session.take();
            }
            Poll::Ready(Some(Err(_))) => {
                self.report.lock().unwrap().completion = if self.context.is_cancelled() {
                    ReadCompletion::Cancelled
                } else {
                    ReadCompletion::Failed
                };
                self.inner.take();
                self.session.take();
            }
            _ => (),
        };
        result
    }
}
impl RecordBatchStream for GuardedRead {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
}
impl Drop for GuardedRead {
    fn drop(&mut self) {
        let mut report = self.report.lock().unwrap();
        if report.completion == ReadCompletion::Pending {
            report.completion = if self.context.is_cancelled() {
                ReadCompletion::Cancelled
            } else {
                ReadCompletion::Abandoned
            };
        }
    }
}

impl Engine {
    pub(crate) fn cache_excluded(&self, name: &str) -> bool {
        if self.write_bindings.contains_key(name) {
            return true;
        }
        if self.resource_identities.get(name).is_some_and(|r| {
            self.write_bindings.keys().any(|n| {
                self.resource_identities.get(n).is_some_and(|w| {
                    r.namespace == w.namespace
                        && (r.domain.is_none()
                            || r.domain != w.domain
                            || r.resource.is_none()
                            || r.resource == w.resource)
                })
            })
        }) {
            // Another configured connection may reach the same physical database.
            // Cache only when connector evidence establishes a disjoint resource.
            return true;
        }
        if let Some(RelationKind::View { dependencies, .. }) =
            self.catalog.relation(name).map(|r| &r.kind)
        {
            return dependencies.iter().any(|n| self.cache_excluded(n));
        }
        false
    }
    pub(crate) async fn invalidate_writable_caches(&mut self) -> Result<()> {
        // Removing eligibility invalidates both already published and on-disk generations.
        // No execution can select them after binding attachment, including on reload.
        let excluded = self
            .materialization_policies
            .keys()
            .filter(|n| self.cache_excluded(n))
            .cloned()
            .collect::<Vec<_>>();
        for n in excluded {
            self.materialization_policies.remove(&n);
        }
        Ok(())
    }
    pub async fn explain_read(&self, sql: &str, options: ReadOptions) -> Result<ReadExplanation> {
        self.plan_sql(sql).await?;
        let deps = self.base_dependencies(sql)?;
        let mut domains = deps
            .iter()
            .filter_map(|n| self.read_bindings.get(n).map(|b| b.connection.domain()))
            .collect::<Vec<_>>();
        domains.sort();
        domains.dedup();
        if options.consistency == ReadConsistency::Snapshot
            && !deps.is_empty()
            && (domains.len() != 1 || deps.iter().any(|n| !self.read_bindings.contains_key(n)))
        {
            return Err(
                failure("snapshot requires one verified read domain for all dependencies").into(),
            );
        }
        Ok(ReadExplanation {
            consistency: options.consistency,
            dependencies: deps,
            domains,
            cache_bypassed: options.consistency == ReadConsistency::Snapshot
                || !options.after_commits.is_empty()
                || matches!(options.cache, ReadCache::Bypass),
            pending_checks: vec![
                "snapshot/visibility acquisition and actual cache selection occur at execution"
                    .into(),
            ],
        })
    }
    pub async fn execute_read(
        &self,
        sql: &str,
        parameters: Vec<ScalarValue>,
        options: ReadOptions,
    ) -> Result<ReadExecution> {
        let (options, context) = read_context(options, false)?;
        tokio::time::timeout_at(context.deadline(), async {
            let explanation = self.explain_read(sql, options.clone()).await?;
            let deps = explanation.dependencies;
            let mut session = None;
            if options.consistency == ReadConsistency::Snapshot && !deps.is_empty() {
                let binding = &self.read_bindings[&deps[0]];
                validate_receipt_scope(self, &deps, &options.after_commits)?;
                session = Some(
                    binding
                        .connection
                        .open_read_session(ReadSessionOptions {
                            after_commits: options.after_commits.clone(),
                            ..Default::default()
                        })
                        .await?,
                );
            }
            self.read_execution(
                sql,
                parameters,
                options,
                session,
                true,
                ExternalReads::Reject,
                context,
            )
            .await
        })
        .await
        .map_err(|_| EngineError::from(failure("read deadline exceeded")))
        .and_then(|r| r)
    }
    #[allow(clippy::too_many_arguments)]
    async fn read_execution(
        &self,
        sql: &str,
        parameters: Vec<ScalarValue>,
        mut options: ReadOptions,
        session: Option<Arc<dyn ConnectorSession>>,
        owned_session: bool,
        external: ExternalReads,
        context: Arc<QueryContext>,
    ) -> Result<ReadExecution> {
        let deps = self.base_dependencies(sql)?;
        validate_receipt_scope(self, &deps, &options.after_commits)?;
        if session.is_some() && !owned_session && !options.after_commits.is_empty() {
            return Err(failure(
                "commit visibility against an already pinned snapshot is unsupported",
            )
            .into());
        }
        let mut overrides = BTreeMap::new();
        let mut observations = vec![];
        let mut resources = vec![];
        let mut domains = vec![];
        // Validate participation for the complete query before binding any provider.
        for n in &deps {
            let binding = self.read_bindings.get(n);
            let participating = session
                .as_ref()
                .is_some_and(|s| binding.is_some_and(|b| s.domain() == b.connection.domain()));
            if session.is_some()
                && !participating
                && (external == ExternalReads::Reject
                    || options.consistency == ReadConsistency::Snapshot)
            {
                return Err(
                    failure("query includes a resource outside the pinned read domain").into(),
                );
            }
        }
        for n in &deps {
            if let Some(b) = self.read_bindings.get(n) {
                resources.push(b.resource.clone());
                domains.push(b.connection.domain());
                let p = if let Some(s) = session
                    .as_ref()
                    .filter(|s| s.domain() == b.connection.domain())
                {
                    Some(s.read_provider(&b.resource).await?)
                } else if !options.after_commits.is_empty() {
                    let receipts = options
                        .after_commits
                        .iter()
                        .filter(|r| {
                            r.domain == b.connection.domain() && r.resources.contains(&b.resource)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    Some(b.connection.read_provider(&b.resource, &receipts).await?)
                } else {
                    None
                };
                if let Some(p) = p {
                    overrides.insert(
                        n.clone(),
                        project(p, &b.columns, &self.catalog.relation(n).unwrap().schema)?,
                    );
                } else {
                    observations.push(n.clone());
                }
            } else {
                observations.push(n.clone());
            }
        }
        domains.sort();
        domains.dedup();
        let bypass = options.query.bypass_materialization
            || session.is_some()
            || !options.after_commits.is_empty()
            || matches!(options.cache, ReadCache::Bypass);
        options.query.bypass_materialization |= bypass;
        if let ReadCache::MaxAge(age) = options.cache {
            options.query.max_cache_age_ms = Some(age.as_millis().min(u64::MAX as u128) as u64);
        }
        let execution = if overrides.is_empty() {
            self.execute_parameters_context(sql, parameters, context.clone())
                .await?
        } else {
            self.bound_engine(overrides)
                .await?
                .execute_parameters_context(sql, parameters, context.clone())
                .await?
        };
        let report = Arc::new(Mutex::new(ReadReport {
            requested: options.consistency,
            established: if (session.is_some()
                || deps.is_empty() && options.consistency == ReadConsistency::Snapshot)
                && observations.is_empty()
            {
                ReadConsistency::Snapshot
            } else {
                ReadConsistency::Observed
            },
            resources,
            domains,
            external_observations: observations,
            snapshot: session.as_ref().map(|s| s.snapshot()),
            read_your_writes: session.as_ref().is_some_and(|s| s.read_your_writes()),
            cache_bypassed: bypass,
            caches: execution.context.cache_observations(),
            satisfied_receipts: options.after_commits,
            completion: ReadCompletion::Pending,
        }));
        let stream = Box::pin(GuardedRead {
            schema: execution.stream.schema(),
            inner: Some(execution.stream),
            report: report.clone(),
            context: execution.context.clone(),
            session: if owned_session { session } else { None },
        });
        Ok(ReadExecution {
            stream,
            context: execution.context,
            report,
        })
    }
    pub(crate) async fn bound_engine(
        &self,
        overrides: BTreeMap<String, Arc<dyn TableProvider>>,
    ) -> Result<Engine> {
        let mut e = Engine::new();
        // Share the configured runtime budget rather than creating an independent execution pool.

        // A fresh catalog is essential: cloning a SessionState retains shared providers.
        e.context = SessionContext::new_with_config_rt(
            datafusion::prelude::SessionConfig::new(),
            self.context.runtime_env(),
        );
        for n in self.registration_order(&self.catalog, false)? {
            let r = self.catalog.relation(&n).unwrap();
            match &r.kind {
                RelationKind::Base { .. } => e.register_table(
                    r.clone(),
                    overrides.get(&n).unwrap_or(&self.providers[&n]).clone(),
                )?,
                RelationKind::View { sql, .. } => {
                    e.create_view_with_description(&n, sql, r.description.as_deref())
                        .await?
                }
            }
        }
        e.read_bindings = self.read_bindings.clone();
        e.write_bindings = self.write_bindings.clone();
        e.query_options = self.query_options.clone();
        e.binding_generation = self.binding_generation;
        e.write_domains = self.write_domains.clone();
        e.resource_identities = self.resource_identities.clone();
        Ok(e)
    }
    pub async fn begin_read_session(
        &self,
        relations: &[String],
        options: ReadSessionOptions,
    ) -> Result<ReadSession> {
        if relations.is_empty() || options.lifetime.is_zero() {
            return Err(
                failure("read sessions require relations and a finite positive lifetime").into(),
            );
        }
        let sql = format!(
            "SELECT 1 FROM {}",
            relations
                .iter()
                .map(|r| crate::writes::quote(r))
                .collect::<Vec<_>>()
                .join(",")
        );
        let explanation = self
            .explain_read(
                &sql,
                ReadOptions {
                    consistency: ReadConsistency::Snapshot,
                    ..Default::default()
                },
            )
            .await?;
        validate_receipt_scope(self, &explanation.dependencies, &options.after_commits)?;
        let first = explanation
            .dependencies
            .first()
            .ok_or_else(|| failure("a read session requires at least one physical resource"))?;
        let b = &self.read_bindings[first];
        let session = b.connection.open_read_session(options.clone()).await?;
        Ok(ReadSession {
            engine: self.bound_engine(BTreeMap::new()).await?,
            session,
            expires: Instant::now() + options.lifetime,
            external: options.external_reads,
            closed: false,
        })
    }
    pub async fn begin_transaction(
        &self,
        connection: Arc<dyn WriteConnection>,
        options: TransactionOptions,
    ) -> Result<Transaction> {
        if options.lifetime.is_zero() {
            return Err(failure("transaction lifetime must be positive").into());
        }
        let session = connection.begin(options.clone()).await?;
        let mut engine = self.bound_engine(BTreeMap::new()).await?;
        for (name, binding) in &mut engine.write_bindings {
            if engine.write_domains.get(name) == Some(&session.domain()) {
                binding.connection = Arc::new(SessionConnection(session.clone()));
            }
        }
        Ok(Transaction {
            engine,
            session,
            expires: Instant::now() + options.lifetime,
            external: options.external_reads,
            poisoned: false,
        })
    }
}
fn validate_receipt_scope(
    engine: &Engine,
    deps: &[String],
    receipts: &[CommitReceipt],
) -> Result<()> {
    for r in receipts {
        if !deps.iter().any(|n| {
            engine.read_bindings.get(n).is_some_and(|b| {
                b.connection.domain() == r.domain && r.resources.contains(&b.resource)
            })
        }) {
            return Err(failure("commit receipt does not apply to a read dependency").into());
        }
    }
    Ok(())
}
fn project(
    provider: Arc<dyn TableProvider>,
    columns: &BTreeMap<String, String>,
    schema: &SchemaRef,
) -> Result<Arc<dyn TableProvider>> {
    let expr = schema
        .fields()
        .iter()
        .map(|f| {
            datafusion::logical_expr::Expr::Column(datafusion::common::Column::new_unqualified(
                &columns[f.name()],
            ))
            .alias(f.name())
        })
        .collect::<Vec<_>>();
    Ok(SessionContext::new()
        .read_table(provider)?
        .select(expr)?
        .into_view())
}
pub struct ReadSession {
    engine: Engine,
    session: Arc<dyn ConnectorSession>,
    expires: Instant,
    external: ExternalReads,
    closed: bool,
}
// Dropping an in-flight handle operation must also release its native resources,
// even if the caller retains the now-poisoned handle without using it again.
struct SessionOperation(Option<Arc<dyn ConnectorSession>>);
impl Drop for SessionOperation {
    fn drop(&mut self) {
        if let Some(session) = self.0.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                let _ = session.finish(false).await;
            });
        }
    }
}
impl ReadSession {
    pub async fn query(
        &mut self,
        sql: &str,
        parameters: Vec<ScalarValue>,
        options: ReadOptions,
    ) -> Result<ReadResult> {
        if self.closed || Instant::now() >= self.expires {
            self.closed = true;
            let _ = self.session.finish(false).await;
            return Err(failure("read session closed or expired").into());
        }
        self.closed = true;
        let mut operation = SessionOperation(Some(self.session.clone()));
        let (options, context) = read_context(options, true)?;
        let result = tokio::time::timeout_at(context.deadline().min(self.expires.into()), async {
            self.engine
                .read_execution(
                    sql,
                    parameters,
                    options,
                    Some(self.session.clone()),
                    false,
                    self.external,
                    context,
                )
                .await?
                .collect()
                .await
        })
        .await
        .map_err(|_| EngineError::from(failure("read session expired")))
        .and_then(|r| r);
        self.closed = result.is_err();
        if result.is_err() {
            let _ = self.session.finish(false).await;
        }
        operation.0.take();
        result
    }
    pub async fn close(mut self) -> Result<()> {
        self.closed = true;
        self.session.finish(false).await?;
        Ok(())
    }
}
pub struct Transaction {
    engine: Engine,
    session: Arc<dyn ConnectorSession>,
    expires: Instant,
    external: ExternalReads,
    poisoned: bool,
}
impl Transaction {
    fn check(&mut self) -> Result<()> {
        if self.poisoned || Instant::now() >= self.expires {
            self.poisoned = true;
            return Err(failure("transaction poisoned or expired; rollback required").into());
        }
        Ok(())
    }
    pub async fn query(
        &mut self,
        sql: &str,
        parameters: Vec<ScalarValue>,
        options: ReadOptions,
    ) -> Result<ReadResult> {
        self.check()?;
        self.poisoned = true;
        let mut operation = SessionOperation(Some(self.session.clone()));
        let (options, context) = read_context(options, true)?;
        let result = tokio::time::timeout_at(context.deadline().min(self.expires.into()), async {
            self.engine
                .read_execution(
                    sql,
                    parameters,
                    options,
                    Some(self.session.clone()),
                    false,
                    self.external,
                    context,
                )
                .await?
                .collect()
                .await
        })
        .await
        .map_err(|_| EngineError::from(failure("transaction expired")))
        .and_then(|r| r);
        self.poisoned = result.is_err();
        operation.0.take();
        result
    }
    pub async fn execute_write(
        &mut self,
        sql: &str,
        parameters: Vec<ScalarValue>,
    ) -> Result<WriteResult> {
        self.check()?;
        self.poisoned = true;
        let mut operation = SessionOperation(Some(self.session.clone()));
        let result = tokio::time::timeout_at(self.expires.into(), async {
            let p = self.engine.prepare_write(sql).await?;
            if p.explain().await?.boundary != self.session.domain() {
                return Err(failure("target is outside the transaction commit domain").into());
            }
            // Every source scan must use the native session, even through dependent views.
            let mut overrides = BTreeMap::new();
            for (n, b) in &self.engine.read_bindings {
                if b.connection.domain() == self.session.domain() {
                    overrides.insert(
                        n.clone(),
                        project(
                            self.session.read_provider(&b.resource).await?,
                            &b.columns,
                            &self.engine.catalog.relation(n).unwrap().schema,
                        )?,
                    );
                }
            }
            if self.external == ExternalReads::Reject
                && p.explain()
                    .await?
                    .source_observations
                    .iter()
                    .any(|n| !overrides.contains_key(n))
            {
                return Err(failure("external write inputs require AllowObserved").into());
            }
            let bound = self.engine.bound_engine(overrides).await?;
            let p = bound.prepare_write(sql).await?;
            p.execute_on(
                parameters,
                WriteOptions::default(),
                Some(self.session.clone()),
            )
            .await
        })
        .await
        .map_err(|_| EngineError::from(failure("transaction expired")))
        .and_then(|r| r);
        self.poisoned = result.as_ref().map_or(true, |r| !r.success());
        operation.0.take();
        result
    }
    pub async fn commit(mut self) -> Result<WriteResult> {
        self.check()?;
        self.session.finish(true).await
    }
    pub async fn rollback(self) -> Result<WriteResult> {
        self.session.finish(false).await
    }
}

struct SessionConnection(Arc<dyn ConnectorSession>);
impl WriteConnection for SessionConnection {
    fn inspect_target<'a>(
        &'a self,
        target: &'a str,
    ) -> futures::future::BoxFuture<'a, Result<TargetInspection>> {
        self.0.inspect_target(target)
    }
    fn validate_operation<'a>(
        &'a self,
        plan: &'a MutationPlan,
    ) -> futures::future::BoxFuture<'a, Result<()>> {
        self.0.validate_operation(plan)
    }
    fn apply<'a>(
        &'a self,
        plan: &'a MutationPlan,
        input: &'a semantic_runtime::staging::StagedInput,
        parameters: &'a [ScalarValue],
        context: Arc<QueryContext>,
    ) -> futures::future::BoxFuture<'a, Result<WriteResult>> {
        self.0.apply(plan, input, parameters, context)
    }
    fn begin<'a>(
        &'a self,
        _: TransactionOptions,
    ) -> futures::future::BoxFuture<'a, Result<Arc<dyn ConnectorSession>>> {
        Box::pin(async { Err(failure("nested transactions unsupported").into()) })
    }
}

fn read_context(
    mut options: ReadOptions,
    session: bool,
) -> Result<(ReadOptions, Arc<QueryContext>)> {
    options.query.bypass_materialization |= session
        || options.consistency == ReadConsistency::Snapshot
        || !options.after_commits.is_empty()
        || matches!(options.cache, ReadCache::Bypass);
    if let ReadCache::MaxAge(age) = options.cache {
        options.query.max_cache_age_ms = Some(age.as_millis().min(u64::MAX as u128) as u64);
    }
    let context = QueryContext::new(options.query.clone())?;
    Ok((options, context))
}
