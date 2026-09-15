use super::*;
use datafusion::common::ScalarValue;
use datafusion::error::Result;
use futures::future::BoxFuture;
use semantic_engine::{self as engine, *};
use semantic_runtime::staging::StagedInput;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;
use tokio_postgres::types::ToSql;

type EResult<T> = engine::Result<T>;
fn err(message: &str) -> EngineError {
    failure(message).into()
}
fn native_error(e: tokio_postgres::Error) -> EngineError {
    // Do not expose server DETAIL, query text, identifiers or parameter values.
    err(match e.code().map(|c| c.code()) {
        Some("23503") => "23503: foreign key constraint failed",
        Some("23505") => "23505: unique constraint failed",
        Some("23502") => "23502: required value missing",
        Some("23514") => "23514: check constraint failed",
        Some("40001") => "40001: serialization conflict",
        Some("40P01") => "40P01: deadlock; operation aborted",
        _ => "Postgres operation failed",
    })
}
fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
impl Postgres {
    /// Explicitly opt this configured connection into writes. Ordinary scans still
    /// use read-only sessions; mutation sessions explicitly begin READ WRITE.
    pub fn writes_enabled(&self) -> bool {
        self.write_enabled
    }
    pub fn with_writes(mut self) -> Self {
        self.write_enabled = true;
        self
    }
    pub async fn bindings(
        &self,
        namespace: &str,
        table: &str,
    ) -> EResult<(Arc<dyn TableProvider>, ReadBinding, Option<WriteBinding>)> {
        let target = format!("{}.{}", identifier(namespace)?, identifier(table)?);
        let client = self
            .pool
            .get()
            .await
            .map_err(|_| err("Postgres connection failed"))?;
        let row = client
            .query_one("SELECT $1::text::regclass::oid", &[&target])
            .await
            .map_err(native_error)?;
        let oid: u32 = row.get(0);
        drop(client);
        let resource = format!("{}:{oid}", self.domain);
        self.resources
            .lock()
            .unwrap()
            .insert(resource.clone(), target);
        let provider = self.table(namespace, table).await?;
        let columns = provider
            .schema()
            .fields()
            .iter()
            .map(|f| (f.name().clone(), f.name().clone()))
            .collect();
        let read = ReadBinding {
            connection: Arc::new(self.clone()),
            resource: resource.clone(),
            columns,
        };
        let write = self.write_enabled.then(|| WriteBinding {
            connection: Arc::new(self.clone()),
            target: resource,
            columns: read.columns.clone(),
        });
        Ok((provider, read, write))
    }
    fn target(&self, resource: &str) -> EResult<String> {
        self.resources
            .lock()
            .unwrap()
            .get(resource)
            .cloned()
            .ok_or_else(|| err("unknown Postgres resource identity"))
    }
    fn validate_receipts(&self, receipts: &[CommitReceipt]) -> EResult<()> {
        let issued = self.receipts.lock().unwrap();
        for r in receipts {
            if r.domain != self.domain || !issued.contains(&r.evidence) {
                return Err(err("unsupported, expired or incompatible commit receipt"));
            }
        }
        Ok(())
    }
    async fn open(
        &self,
        write: bool,
        isolation: Isolation,
        lifetime: Duration,
    ) -> EResult<PgSession> {
        if write && !self.write_enabled {
            return Err(err("Postgres writes require explicit opt-in"));
        }
        if lifetime.is_zero() || lifetime > Duration::from_secs(3600) {
            return Err(err(
                "Postgres session lifetime must be between zero and 3600 seconds",
            ));
        }
        let context = QueryContext::new(QueryOptions::default())?;
        let client = context
            .run(async {
                self.pool
                    .get()
                    .await
                    .map_err(|_| failure("Postgres connection failed"))
            })
            .await?;
        let lease = Lease {
            client: Some(client),
            clean: false,
        };
        let c = lease.client.as_ref().unwrap();
        let isolation = match isolation {
            Isolation::RepeatableRead => "REPEATABLE READ",
            Isolation::Serializable => "SERIALIZABLE",
        };
        c.batch_execute(&format!("BEGIN ISOLATION LEVEL {isolation} READ {}; SET LOCAL idle_in_transaction_session_timeout = '{}ms'",if write{"WRITE"}else{"ONLY"},lifetime.as_millis())).await.map_err(native_error)?;
        let standby: bool = c
            .query_one("SELECT pg_is_in_recovery()", &[])
            .await
            .map_err(native_error)?
            .get(0);
        if standby {
            return Err(err(
                "snapshot/visibility sessions currently require the authoritative Postgres database",
            ));
        }
        // The snapshot is acquired now, including for sessions with no first query yet.
        let snapshot: String = c
            .query_one("SELECT pg_current_snapshot()::text", &[])
            .await
            .map_err(native_error)?
            .get(0);
        let session = PgSession {
            postgres: self.clone(),
            lease: Arc::new(Mutex::new(Some(lease))),
            snapshot,
            expires: tokio::time::Instant::now() + lifetime,
            poisoned: Arc::new(AtomicBool::new(false)),
            write,
            resources: Arc::new(StdMutex::new(BTreeSet::new())),
        };
        let weak = Arc::downgrade(&session.lease);
        let expires = session.expires;
        tokio::spawn(async move {
            tokio::time::sleep_until(expires).await;
            if let Some(lease) = weak.upgrade() {
                lease.lock().await.take();
            }
        });
        Ok(session)
    }
}
struct Metadata {
    info: TargetInspection,
    defaults: BTreeMap<String, bool>,
    types: BTreeMap<String, DataType>,
    collations: BTreeMap<String, String>,
}
async fn inspect(c: &tokio_postgres::Client, pg: &Postgres, resource: &str) -> EResult<Metadata> {
    let target = pg.target(resource)?;
    let oid: u32 = c
        .query_one("SELECT $1::text::regclass::oid", &[&target])
        .await
        .map_err(native_error)?
        .get(0);
    if !resource.ends_with(&format!(":{oid}")) {
        return Err(err("physical resource changed; reload required"));
    }
    let rows=c.query("SELECT a.attname, a.atttypid, a.attnotnull, a.attgenerated::text, a.attidentity::text, pg_get_expr(d.adbin,d.adrelid), a.attnum::int, CASE WHEN a.attcollation=0 THEN NULL ELSE quote_ident(n.nspname)||'.'||quote_ident(co.collname) END, co.collisdeterministic FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum LEFT JOIN pg_collation co ON co.oid=a.attcollation LEFT JOIN pg_namespace n ON n.oid=co.collnamespace WHERE a.attrelid=$1::text::regclass AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",&[&target]).await.map_err(native_error)?;
    if rows.is_empty() {
        return Err(err("Postgres target has no supported columns"));
    }
    let mut fields = vec![];
    let mut defaults = BTreeMap::new();
    let mut numbers = BTreeMap::new();
    let mut nonnull = BTreeSet::new();
    let mut types = BTreeMap::new();
    let mut collations = BTreeMap::new();
    let mut checked = true;
    let mut revision = String::new();
    for row in &rows {
        let n: String = row.get(0);
        let oid: u32 = row.get(1);
        let required: bool = row.get(2);
        let generated: String = row.get(3);
        let identity: String = row.get(4);
        let default: Option<String> = row.get(5);
        let number: i32 = row.get(6);
        let collation: Option<String> = row.get(7);
        let deterministic: Option<bool> = row.get(8);
        let ty = arrow_type(
            &Type::from_oid(oid).ok_or_else(|| err("unsupported Postgres target type"))?,
        )?;
        checked &= generated.is_empty() && identity.is_empty() && deterministic != Some(false);
        // Only literal defaults (optionally cast to a builtin type) are accepted.
        let safe = default.as_ref().is_none_or(|d| safe_default(d));
        defaults.insert(n.clone(), !required || default.is_some() && safe);
        checked &= safe;
        if required {
            nonnull.insert(n.clone());
        }
        numbers.insert(number, n.clone());
        types.insert(n.clone(), ty.clone());
        if let Some(co) = &collation {
            collations.insert(n.clone(), co.clone());
        }
        revision.push_str(&format!(
            "{n:?}:{oid}:{required}:{generated}:{identity}:{default:?}:{collation:?}:{deterministic:?};"
        ));
        // Keep provider schema compatibility: the read connector historically reports all fields nullable.
        fields.push(Field::new(n, ty, true));
    }
    let indices=c.query("SELECT indkey::smallint[], indexrelid::text FROM pg_index WHERE indrelid=$1::text::regclass AND indisunique AND indisvalid AND indisready AND indimmediate AND indpred IS NULL AND indexprs IS NULL AND indnkeyatts=indnatts AND NOT EXISTS(SELECT 1 FROM unnest(indkey::smallint[],indcollation::oid[]) AS parts(attnum,collation_oid) JOIN pg_attribute a ON a.attrelid=indrelid AND a.attnum=parts.attnum WHERE parts.collation_oid<>a.attcollation) AND NOT EXISTS(SELECT 1 FROM unnest(indclass::oid[]) op JOIN pg_opclass oc ON oc.oid=op JOIN pg_namespace ns ON ns.oid=oc.opcnamespace WHERE ns.nspname<>'pg_catalog' OR NOT oc.opcdefault)",&[&target]).await.map_err(native_error)?;
    let mut keys = vec![];
    for row in indices {
        let ns: Vec<i16> = row.get(0);
        let index: String = row.get(1);
        let key = ns
            .iter()
            .filter_map(|n| numbers.get(&(*n as i32)).cloned())
            .collect::<Vec<_>>();
        if key.len() == ns.len() && key.iter().all(|n| nonnull.contains(n)) {
            keys.push(key);
        }
        revision.push_str(&index);
    }
    // Index expressions and predicates may invoke user functions. Even declarations
    // of immutability are not evidence that replay has no external effects.
    let all_indices = c.query(
        "SELECT pg_get_indexdef(indexrelid), indexprs IS NOT NULL OR indpred IS NOT NULL OR EXISTS(SELECT 1 FROM unnest(indclass::oid[]) op JOIN pg_opclass oc ON oc.oid=op JOIN pg_namespace ns ON ns.oid=oc.opcnamespace WHERE ns.nspname<>'pg_catalog' OR NOT oc.opcdefault) FROM pg_index WHERE indrelid=$1::text::regclass ORDER BY indexrelid",
        &[&target],
    ).await.map_err(native_error)?;
    for index in all_indices {
        let definition: String = index.get(0);
        let expressions: bool = index.get(1);
        revision.push_str(&definition);
        checked &= !expressions;
    }
    let effects=c.query_one("SELECT c.relkind::text,c.relrowsecurity,EXISTS(SELECT 1 FROM pg_trigger t WHERE t.tgrelid=c.oid AND NOT t.tgisinternal AND t.tgenabled<>'D'),EXISTS(SELECT 1 FROM pg_rewrite r WHERE r.ev_class=c.oid),EXISTS(SELECT 1 FROM pg_constraint k WHERE k.conrelid=c.oid AND k.contype IN ('f','x','c')),EXISTS(SELECT 1 FROM pg_inherits i WHERE i.inhparent=c.oid OR i.inhrelid=c.oid) FROM pg_class c WHERE c.oid=$1::text::regclass",&[&target]).await.map_err(native_error)?;
    let kind: String = effects.get(0);
    let rls: bool = effects.get(1);
    let triggers: bool = effects.get(2);
    let rules: bool = effects.get(3);
    let foreign: bool = effects.get(4);
    let inheritance: bool = effects.get(5);
    if kind != "r" {
        return Err(err(
            "only ordinary physical Postgres tables support these bindings",
        ));
    }
    checked &= kind == "r" && !rls && !triggers && !rules && !foreign && !inheritance;
    revision.push_str(&format!(
        "{kind}:{rls}:{triggers}:{rules}:{foreign}:{inheritance}:{keys:?}"
    ));
    Ok(Metadata {
        info: TargetInspection {
            physical_namespace: "postgres".into(),
            schema: Arc::new(Schema::new(fields)),
            revision: semantic_runtime::fingerprint(&[revision.as_bytes()]),
            resource: resource.into(),
            domain: pg.domain.clone(),
            unique_keys: keys,
            checked_eligible: checked,
            supported_operations: ["INSERT", "UPDATE", "DELETE", "MERGE"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            atomic_writes: true,
        },
        defaults,
        types,
        collations,
    })
}
fn safe_default(sql: &str) -> bool {
    use datafusion::sql::sqlparser::{ast::*, dialect::PostgreSqlDialect, parser::Parser};
    fn safe(e: &Expr) -> bool {
        match e {
            Expr::Value(v) => matches!(
                v.value,
                Value::SingleQuotedString(_)
                    | Value::Number(_, _)
                    | Value::Boolean(_)
                    | Value::Null
            ),
            Expr::Cast {
                expr, data_type, ..
            } => {
                matches!(
                    data_type,
                    DataType::Text
                        | DataType::Varchar(_)
                        | DataType::Int(_)
                        | DataType::Integer(_)
                        | DataType::BigInt(_)
                        | DataType::Boolean
                ) && safe(expr)
            }
            Expr::Nested(e) => safe(e),
            _ => false,
        }
    }
    Parser::parse_sql(&PostgreSqlDialect {}, &format!("SELECT {sql}"))
        .ok()
        .is_some_and(|ss| match &ss[0] {
            Statement::Query(query) => match query.body.as_ref() {
                SetExpr::Select(s) => {
                    matches!(&s.projection[0],SelectItem::UnnamedExpr(e) if safe(e))
                }
                _ => false,
            },
            _ => false,
        })
}
fn validate(meta: &Metadata, plan: &MutationPlan) -> EResult<()> {
    if meta.info.revision != plan.revision {
        return Err(err(
            "target schema or constraint assumptions changed; reload and prepare again",
        ));
    }
    if plan.checked && !meta.info.checked_eligible {
        return Err(err(
            "checked idempotency cannot establish target side-effect and comparison eligibility",
        ));
    }
    let inserts = match &plan.mutation {
        Mutation::Insert { columns } => Some(columns.clone()),
        Mutation::Merge { keys, inserts, .. } => {
            if !meta.info.unique_keys.iter().any(|k| {
                k.iter().collect::<BTreeSet<_>>()
                    == keys.iter().map(|(k, _)| k).collect::<BTreeSet<_>>()
            }) {
                return Err(err("complete enforced non-null unique key required"));
            }
            if inserts.is_empty() {
                None
            } else {
                Some(inserts.iter().map(|(n, _)| n.clone()).collect())
            }
        }
        _ => None,
    };
    if let Some(columns) = inserts {
        for (n, safe) in &meta.defaults {
            if !columns.contains(n) && !*safe {
                return Err(err("insert omits a required column or unsupported default"));
            }
        }
    }
    Ok(())
}
impl WriteConnection for Postgres {
    fn inspect_target<'a>(&'a self, target: &'a str) -> BoxFuture<'a, EResult<TargetInspection>> {
        Box::pin(async move {
            if !self.write_enabled {
                return Err(err("Postgres writes disabled"));
            }
            let c = self
                .pool
                .get()
                .await
                .map_err(|_| err("Postgres connection failed"))?;
            Ok(inspect(&c, self, target).await?.info)
        })
    }
    fn validate_operation<'a>(&'a self, plan: &'a MutationPlan) -> BoxFuture<'a, EResult<()>> {
        Box::pin(async move {
            let c = self
                .pool
                .get()
                .await
                .map_err(|_| err("Postgres connection failed"))?;
            validate(&inspect(&c, self, &plan.target).await?, plan)
        })
    }
    fn begin<'a>(
        &'a self,
        options: TransactionOptions,
    ) -> BoxFuture<'a, EResult<Arc<dyn ConnectorSession>>> {
        Box::pin(async move {
            Ok(
                Arc::new(self.open(true, options.isolation, options.lifetime).await?)
                    as Arc<dyn ConnectorSession>,
            )
        })
    }
    fn apply<'a>(
        &'a self,
        plan: &'a MutationPlan,
        input: &'a StagedInput,
        parameters: &'a [ScalarValue],
        context: Arc<QueryContext>,
    ) -> BoxFuture<'a, EResult<WriteResult>> {
        Box::pin(async move {
            let s = context
                .run(async {
                    self.open(true, Isolation::Serializable, Duration::from_secs(300))
                        .await
                        .map_err(|e| failure(&e.to_string()))
                })
                .await?;
            let applied = s.apply(plan, input, parameters, context.clone()).await?;
            if !applied.success() {
                let _ = s.finish(false).await;
                return Ok(applied);
            }
            if let Err(e) = context.check() {
                let _ = s.finish(false).await;
                return Ok(s.result(
                    WriteOutcome::Aborted,
                    plan.checked,
                    None,
                    Some(e.to_string()),
                ));
            }
            let mut result = match tokio::time::timeout_at(context.deadline(), s.finish(true)).await
            {
                Ok(result) => result?,
                Err(_) => s.result(
                    WriteOutcome::OutcomeUnknown,
                    plan.checked,
                    None,
                    Some("commit deadline exceeded; acknowledgement unknown".into()),
                ),
            };
            result.idempotent = plan.checked;
            result.affected_rows = applied.affected_rows;
            Ok(result)
        })
    }
    fn create_table<'a>(
        &'a self,
        definition: &'a TableDefinition,
    ) -> BoxFuture<'a, EResult<CreatedTable>> {
        Box::pin(async move {
            if !self.write_enabled {
                return Err(err("Postgres writes disabled"));
            }
            if definition
                .target
                .keys()
                .any(|k| k != "schema" && k != "table")
            {
                return Err(err("Postgres target options require schema and table"));
            }
            let schema = definition
                .target
                .get("schema")
                .ok_or_else(|| err("schema required"))?;
            let table = definition
                .target
                .get("table")
                .ok_or_else(|| err("table required"))?;
            if definition.schema.fields().is_empty()
                || definition
                    .primary_key
                    .iter()
                    .any(|k| definition.schema.field_with_name(k).is_err())
                || definition.primary_key.iter().collect::<BTreeSet<_>>().len()
                    != definition.primary_key.len()
            {
                return Err(err("invalid table definition"));
            }
            let mut columns = definition
                .schema
                .fields()
                .iter()
                .map(|f| {
                    Ok(format!(
                        "{} {} {}",
                        identifier(f.name())?,
                        native_type(f.data_type())?,
                        if !f.is_nullable() || definition.primary_key.contains(f.name()) {
                            "NOT NULL"
                        } else {
                            ""
                        }
                    ))
                })
                .collect::<EResult<Vec<_>>>()?;
            if !definition.primary_key.is_empty() {
                columns.push(format!(
                    "PRIMARY KEY ({})",
                    definition
                        .primary_key
                        .iter()
                        .map(|n| q(n))
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            let mut lease = Lease {
                client: Some(
                    self.pool
                        .get()
                        .await
                        .map_err(|_| err("Postgres connection failed"))?,
                ),
                clean: false,
            };
            let c = lease.client.as_ref().unwrap();
            c.batch_execute(&format!(
                "BEGIN READ WRITE; CREATE TABLE {}.{} ({})",
                identifier(schema)?,
                identifier(table)?,
                columns.join(",")
            ))
            .await
            .map_err(native_error)?;
            c.batch_execute("COMMIT").await.map_err(|_| {
                err("table creation outcome unknown; inspect physical target before retry")
            })?;
            lease.clean = true;
            drop(lease);
            let (provider, read, write) = self.bindings(schema, table).await.map_err(|_| {
                err(&format!(
                    "physical table created; recover {}.{} after registration inspection failure",
                    q(schema),
                    q(table)
                ))
            })?;
            Ok(CreatedTable {
                provider,
                read,
                write: write.unwrap(),
            })
        })
    }
}
impl ReadConnection for Postgres {
    fn physical_namespace(&self) -> Option<&'static str> {
        Some("postgres")
    }
    fn domain(&self) -> String {
        self.domain.clone()
    }
    fn read_provider<'a>(
        &'a self,
        resource: &'a str,
        receipts: &'a [CommitReceipt],
    ) -> BoxFuture<'a, EResult<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            self.validate_receipts(receipts)?;
            if !receipts.is_empty() {
                if receipts
                    .iter()
                    .any(|r| !r.resources.contains(&resource.to_owned()))
                {
                    return Err(err("receipt does not cover resource"));
                }
                // Pin the actual reader: validating one pooled connection and scanning
                // through another would not establish visibility behind a router.
                let session = self
                    .open(false, Isolation::RepeatableRead, Duration::from_secs(300))
                    .await?;
                return session.read_provider(resource).await;
            }
            if receipts
                .iter()
                .any(|r| !r.resources.contains(&resource.to_owned()))
            {
                return Err(err("receipt does not cover resource"));
            }
            let c = self
                .pool
                .get()
                .await
                .map_err(|_| err("Postgres connection failed"))?;
            let meta = inspect(&c, self, resource).await?;
            Ok(Arc::new(PgTable {
                postgres: self.clone(),
                target: self.target(resource)?,
                schema: meta.info.schema,
            }) as Arc<dyn TableProvider>)
        })
    }
    fn open_read_session<'a>(
        &'a self,
        options: ReadSessionOptions,
    ) -> BoxFuture<'a, EResult<Arc<dyn ConnectorSession>>> {
        Box::pin(async move {
            self.validate_receipts(&options.after_commits)?;
            Ok(Arc::new(
                self.open(false, Isolation::RepeatableRead, options.lifetime)
                    .await?,
            ) as Arc<dyn ConnectorSession>)
        })
    }
}
#[derive(Clone)]
struct PgSession {
    postgres: Postgres,
    lease: Arc<Mutex<Option<Lease>>>,
    snapshot: String,
    expires: tokio::time::Instant,
    poisoned: Arc<AtomicBool>,
    write: bool,
    resources: Arc<StdMutex<BTreeSet<String>>>,
}
impl PgSession {
    fn check(&self) -> EResult<()> {
        if self.poisoned.load(Ordering::Acquire) || tokio::time::Instant::now() >= self.expires {
            return Err(err("native session poisoned or expired"));
        }
        Ok(())
    }
    fn result(
        &self,
        outcome: WriteOutcome,
        checked: bool,
        rows: Option<u64>,
        message: Option<String>,
    ) -> WriteResult {
        let code = message.as_ref().and_then(|m| {
            ["23503", "23505", "23502", "23514", "40001", "40P01"]
                .iter()
                .find(|c| m.contains(**c))
                .map(|c| c.to_string())
        });
        WriteResult {
            outcome,
            operation_id: semantic_runtime::unique_id(),
            boundary: self.postgres.domain.clone(),
            atomic: true,
            idempotent: checked,
            receipt: None,
            affected_rows: rows,
            code,
            message,
            external_observations: vec![],
        }
    }
}
impl ConnectorSession for PgSession {
    fn read_your_writes(&self) -> bool {
        self.write
    }
    fn inspect_target<'a>(&'a self, target: &'a str) -> BoxFuture<'a, EResult<TargetInspection>> {
        Box::pin(async move {
            self.check()?;
            let lease = self.lease.lock().await;
            let c = lease
                .as_ref()
                .and_then(|l| l.client.as_ref())
                .ok_or_else(|| err("session closed"))?;
            Ok(inspect(c, &self.postgres, target).await?.info)
        })
    }
    fn validate_operation<'a>(&'a self, plan: &'a MutationPlan) -> BoxFuture<'a, EResult<()>> {
        Box::pin(async move {
            self.check()?;
            let lease = self.lease.lock().await;
            let c = lease
                .as_ref()
                .and_then(|l| l.client.as_ref())
                .ok_or_else(|| err("session closed"))?;
            validate(&inspect(c, &self.postgres, &plan.target).await?, plan)
        })
    }

    fn domain(&self) -> String {
        self.postgres.domain.clone()
    }
    fn snapshot(&self) -> String {
        self.snapshot.clone()
    }
    fn read_provider<'a>(
        &'a self,
        resource: &'a str,
    ) -> BoxFuture<'a, EResult<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            self.check()?;
            let lease = self.lease.lock().await;
            let c = lease
                .as_ref()
                .and_then(|l| l.client.as_ref())
                .ok_or_else(|| err("session closed"))?;
            c.batch_execute(&format!(
                "LOCK TABLE {} IN ACCESS SHARE MODE",
                self.postgres.target(resource)?
            ))
            .await
            .map_err(native_error)?;
            let meta = inspect(c, &self.postgres, resource).await?;
            Ok(Arc::new(SessionTable {
                session: self.clone(),
                target: self.postgres.target(resource)?,
                schema: meta.info.schema,
            }) as Arc<dyn TableProvider>)
        })
    }
    fn apply<'a>(
        &'a self,
        plan: &'a MutationPlan,
        input: &'a StagedInput,
        parameters: &'a [ScalarValue],
        context: Arc<QueryContext>,
    ) -> BoxFuture<'a, EResult<WriteResult>> {
        Box::pin(async move {
            self.check()?;
            if !self.write {
                return Err(err("read session cannot write"));
            }
            let result = tokio::time::timeout_at(self.expires.min(context.deadline()), async {
                let lease = self.lease.lock().await;
                let c = lease
                    .as_ref()
                    .and_then(|l| l.client.as_ref())
                    .ok_or_else(|| err("session closed"))?;
                let target = self.postgres.target(&plan.target)?;
                context
                    .run(async {
                        c.batch_execute(&format!("LOCK TABLE {target} IN SHARE ROW EXCLUSIVE MODE"))
                            .await
                            .map_err(|e| failure(&native_error(e).to_string()))
                    })
                    .await?;
                let meta = inspect(c, &self.postgres, &plan.target).await?;
                validate(&meta, plan)?;
                context.check()?;
                let rows =
                    apply_native(c, &target, &meta, plan, input, parameters, &context).await?;
                self.resources.lock().unwrap().insert(plan.target.clone());
                Ok::<_, EngineError>(rows)
            })
            .await;
            match result {
                Ok(Ok(rows)) => Ok(self.result(
                    WriteOutcome::AppliedInTransaction,
                    plan.checked,
                    Some(rows),
                    None,
                )),
                other => {
                    self.poisoned.store(true, Ordering::Release);
                    // Discarding the lease terminates an uncommitted native transaction.
                    self.lease.lock().await.take();
                    let message = match other {
                        Ok(Err(e)) => e.to_string(),
                        Err(_) => "session expired".into(),
                        _ => unreachable!(),
                    };
                    Ok(self.result(WriteOutcome::Aborted, plan.checked, None, Some(message)))
                }
            }
        })
    }
    fn finish<'a>(&'a self, commit: bool) -> BoxFuture<'a, EResult<WriteResult>> {
        Box::pin(async move {
            if commit {
                self.check()?;
            }
            let mut guard = self.lease.lock().await;
            let Some(mut lease) = guard.take() else {
                return if commit {
                    Err(err("session closed"))
                } else {
                    Ok(self.result(WriteOutcome::Aborted, false, None, None))
                };
            };
            let c = lease.client.as_ref().unwrap();
            let reply = tokio::time::timeout(
                Duration::from_secs(30),
                c.batch_execute(if commit { "COMMIT" } else { "ROLLBACK" }),
            )
            .await;
            match reply {
                Ok(Ok(())) => {
                    lease.clean = true;
                    let mut result = self.result(
                        if commit {
                            WriteOutcome::Committed
                        } else {
                            WriteOutcome::Aborted
                        },
                        false,
                        None,
                        None,
                    );
                    if commit {
                        let evidence = semantic_runtime::unique_id();
                        self.postgres
                            .receipts
                            .lock()
                            .unwrap()
                            .insert(evidence.clone());
                        result.receipt = Some(CommitReceipt {
                            domain: self.domain(),
                            evidence,
                            resources: self.resources.lock().unwrap().iter().cloned().collect(),
                        });
                    }
                    Ok(result)
                }
                Ok(Err(e)) if e.as_db_error().is_some() => Ok(self.result(
                    WriteOutcome::Aborted,
                    false,
                    None,
                    Some(native_error(e).to_string()),
                )),
                _ => Ok(self.result(
                    if commit {
                        WriteOutcome::OutcomeUnknown
                    } else {
                        WriteOutcome::Aborted
                    },
                    false,
                    None,
                    Some(
                        if commit {
                            "commit acknowledgement unavailable; do not retry blindly"
                        } else {
                            "session discarded during rollback"
                        }
                        .into(),
                    ),
                )),
            }
        })
    }
}
fn native_type(ty: &DataType) -> EResult<&'static str> {
    Ok(match ty {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => "text",
        DataType::Boolean => "boolean",
        DataType::Int16 => "smallint",
        DataType::Int32 => "integer",
        DataType::Int64 => "bigint",
        DataType::Float32 => "real",
        DataType::Float64 => "double precision",
        DataType::Date32 => "date",
        DataType::Timestamp(TimeUnit::Microsecond, None) => "timestamp",
        DataType::Timestamp(TimeUnit::Microsecond, Some(_)) => "timestamptz",
        _ => return Err(err("unsupported Postgres mutation type")),
    })
}
fn value(v: &ScalarValue, ty: &Type) -> EResult<Box<dyn ToSql + Sync + Send>> {
    let t = arrow_type(ty)?;
    let v = if v.is_null() {
        ScalarValue::try_from(&t)?
    } else {
        v.cast_to(&t)
            .map_err(|_| err("Postgres value conversion failed"))?
    };
    Ok(match v {
        ScalarValue::Utf8(v) => Box::new(v),
        ScalarValue::Boolean(v) => Box::new(v),
        ScalarValue::Int16(v) => Box::new(v),
        ScalarValue::Int32(v) => Box::new(v),
        ScalarValue::Int64(v) => Box::new(v),
        ScalarValue::Float32(v) => Box::new(v),
        ScalarValue::Float64(v) => Box::new(v),
        ScalarValue::Date32(v) => Box::new(v.map(|n| {
            chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap() + chrono::Duration::days(n as i64)
        })),
        ScalarValue::TimestampMicrosecond(v, None) => Box::new(
            v.map(|n| chrono::DateTime::from_timestamp_micros(n).map(|d| d.naive_utc()))
                .transpose_option()?,
        ),
        ScalarValue::TimestampMicrosecond(v, Some(_)) => Box::new(
            v.map(chrono::DateTime::from_timestamp_micros)
                .transpose_option()?,
        ),
        _ => return Err(err("unsupported mutation value")),
    })
}
trait TransposeOption<T> {
    fn transpose_option(self) -> EResult<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> EResult<Option<T>> {
        match self {
            None => Ok(None),
            Some(Some(v)) => Ok(Some(v)),
            Some(None) => Err(err("timestamp outside supported range")),
        }
    }
}
async fn execute(
    c: &tokio_postgres::Client,
    sql: &str,
    values: &[ScalarValue],
    context: &Arc<QueryContext>,
) -> EResult<u64> {
    let stmt = c.prepare(sql).await.map_err(native_error)?;
    if stmt.params().len() != values.len() {
        return Err(err("native parameter count mismatch"));
    }
    let values = values
        .iter()
        .zip(stmt.params())
        .map(|(v, t)| value(v, t))
        .collect::<EResult<Vec<_>>>()?;
    let refs = values
        .iter()
        .map(|v| v.as_ref() as &(dyn ToSql + Sync))
        .collect::<Vec<_>>();
    context
        .run(async {
            c.execute(&stmt, &refs)
                .await
                .map_err(|e| failure(&native_error(e).to_string()))
        })
        .await
        .map_err(Into::into)
}
fn lower(
    e: &ValueExpr,
    parameters: &[ScalarValue],
    values: &mut Vec<ScalarValue>,
) -> EResult<String> {
    Ok(match e {
        ValueExpr::Column(c) => format!("t.{}", q(c)),
        ValueExpr::Parameter(i) => {
            values.push(
                parameters
                    .get(*i)
                    .ok_or_else(|| err("missing parameter"))?
                    .clone(),
            );
            format!("${}", values.len())
        }
        ValueExpr::Literal(v) => {
            values.push(v.clone());
            format!("${}", values.len())
        }
        ValueExpr::Binary(l, op, r) => format!(
            "({} {op} {})",
            lower(l, parameters, values)?,
            lower(r, parameters, values)?
        ),
        ValueExpr::Unary(op, e) => format!("({op} {})", lower(e, parameters, values)?),
        ValueExpr::Cast(e, t) => format!(
            "CAST({} AS {})",
            lower(e, parameters, values)?,
            native_type(t)?
        ),
        ValueExpr::IsNull(e, not) => format!(
            "({} IS {}NULL)",
            lower(e, parameters, values)?,
            if *not { "NOT " } else { "" }
        ),
    })
}
async fn apply_native(
    c: &tokio_postgres::Client,
    target: &str,
    meta: &Metadata,
    plan: &MutationPlan,
    input: &StagedInput,
    parameters: &[ScalarValue],
    context: &Arc<QueryContext>,
) -> EResult<u64> {
    let mut values = vec![];
    match &plan.mutation {
        Mutation::Update {
            assignments,
            predicate,
        } => {
            let set = assignments
                .iter()
                .map(|(n, e)| Ok(format!("{}={}", q(n), lower(e, parameters, &mut values)?)))
                .collect::<EResult<Vec<_>>>()?
                .join(",");
            let predicate = predicate
                .as_ref()
                .map(|e| lower(e, parameters, &mut values))
                .transpose()?
                .map(|p| format!(" WHERE {p}"))
                .unwrap_or_default();
            execute(
                c,
                &format!("UPDATE {target} AS t SET {set}{predicate}"),
                &values,
                context,
            )
            .await
        }
        Mutation::Delete { predicate } => {
            let predicate = predicate
                .as_ref()
                .map(|e| lower(e, parameters, &mut values))
                .transpose()?
                .map(|p| format!(" WHERE {p}"))
                .unwrap_or_default();
            execute(
                c,
                &format!("DELETE FROM {target} AS t{predicate}"),
                &values,
                context,
            )
            .await
        }
        Mutation::Insert { columns } => {
            if input.schema.fields().len() != columns.len() {
                return Err(err("INSERT source column count mismatch"));
            }
            let names = columns.iter().map(|c| q(c)).collect::<Vec<_>>().join(",");
            let params = (1..=columns.len())
                .map(|i| format!("${i}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("INSERT INTO {target} ({names}) VALUES ({params})");
            let mut rows = 0;
            for batch in input.batches() {
                let batch = batch?;
                for row in 0..batch.num_rows() {
                    let values = batch
                        .columns()
                        .iter()
                        .map(|a| ScalarValue::try_from_array(a, row))
                        .collect::<Result<Vec<_>>>()?;
                    rows += execute(c, &sql, &values, context).await?;
                }
            }
            Ok(rows)
        }
        Mutation::Merge {
            keys,
            updates,
            inserts,
        } => {
            let stage = q(&format!(
                "semantic_stage_{}",
                semantic_runtime::unique_id().replace('-', "_")
            ));
            let mapping = keys
                .iter()
                .chain(updates)
                .chain(inserts)
                .map(|(dest, source)| (source.clone(), dest.clone()))
                .collect::<BTreeMap<_, _>>();
            let cols = mapping
                .iter()
                .map(|(source, dest)| {
                    Ok(format!(
                        "{} {}{}",
                        q(source),
                        native_type(&meta.types[dest])?,
                        meta.collations
                            .get(dest)
                            .map(|c| format!(" COLLATE {c}"))
                            .unwrap_or_default()
                    ))
                })
                .collect::<EResult<Vec<_>>>()?;
            c.batch_execute(&format!(
                "CREATE TEMP TABLE {stage} ({}) ON COMMIT DROP",
                cols.join(",")
            ))
            .await
            .map_err(native_error)?;
            let names = mapping.keys().map(|c| q(c)).collect::<Vec<_>>().join(",");
            let params = (1..=mapping.len())
                .map(|i| format!("${i}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("INSERT INTO {stage} ({names}) VALUES ({params})");
            for batch in input.batches() {
                let batch = batch?;
                for row in 0..batch.num_rows() {
                    let values = mapping
                        .keys()
                        .map(|n| {
                            ScalarValue::try_from_array(
                                batch
                                    .column_by_name(n)
                                    .ok_or_else(|| failure("missing staged column"))?,
                                row,
                            )
                        })
                        .collect::<Result<Vec<_>>>()?;
                    execute(c, &sql, &values, context).await?;
                }
            }
            let keycols = keys.iter().map(|(_, s)| q(s)).collect::<Vec<_>>();
            let null = keycols
                .iter()
                .map(|k| format!("{k} IS NULL"))
                .collect::<Vec<_>>()
                .join(" OR ");
            let invalid:bool=c.query_one(&format!("SELECT EXISTS(SELECT 1 FROM {stage} WHERE {null}) OR EXISTS(SELECT 1 FROM {stage} GROUP BY {} HAVING count(*)>1)",keycols.join(",")),&[]).await.map_err(native_error)?.get(0);
            if invalid {
                return Err(err(
                    "null or duplicate source keys under destination equality",
                ));
            }
            context.check()?;
            let matched = keys
                .iter()
                .map(|(d, s)| format!("t.{}=s.{}", q(d), q(s)))
                .collect::<Vec<_>>()
                .join(" AND ");
            let mut rows = 0;
            if !updates.is_empty() {
                let assignments = updates
                    .iter()
                    .map(|(d, s)| format!("{}=s.{}", q(d), q(s)))
                    .collect::<Vec<_>>()
                    .join(",");
                let changed = updates
                    .iter()
                    .map(|(d, s)| format!("t.{} IS DISTINCT FROM s.{}", q(d), q(s)))
                    .collect::<Vec<_>>()
                    .join(" OR ");
                rows+=execute(c,&format!("UPDATE {target} AS t SET {assignments} FROM {stage} AS s WHERE {matched} AND ({changed})"),&[],context).await?;
            }
            if !inserts.is_empty() {
                let dest = inserts
                    .iter()
                    .map(|(d, _)| q(d))
                    .collect::<Vec<_>>()
                    .join(",");
                let src = inserts
                    .iter()
                    .map(|(_, s)| format!("s.{}", q(s)))
                    .collect::<Vec<_>>()
                    .join(",");
                let conflict = keys.iter().map(|(d, _)| q(d)).collect::<Vec<_>>().join(",");
                rows+=execute(c,&format!("INSERT INTO {target} AS t ({dest}) SELECT {src} FROM {stage} AS s WHERE NOT EXISTS(SELECT 1 FROM {target} AS t WHERE {matched}) ON CONFLICT ({conflict}) DO NOTHING"),&[],context).await?;
            }
            c.batch_execute(&format!("DROP TABLE {stage}"))
                .await
                .map_err(native_error)?;
            Ok(rows)
        }
    }
}
#[derive(Clone)]
struct SessionTable {
    session: PgSession,
    target: String,
    schema: SchemaRef,
}
impl fmt::Debug for SessionTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostgresSessionTable")
    }
}
#[async_trait]
impl TableProvider for SessionTable {
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
            .map(|i| q(self.schema.field(*i).name()))
            .collect::<Vec<_>>();
        let select = if columns.is_empty() {
            "1 AS __row".into()
        } else {
            columns.join(",")
        };
        let sql = format!(
            "SELECT {select} FROM {}{}",
            self.target,
            limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default()
        );
        let partition = Arc::new(SessionPartition {
            session: self.session.clone(),
            schema: schema.clone(),
            sql,
        });
        StreamingTable::try_new(schema, vec![partition])?
            .scan(state, None, &[], None)
            .await
    }
}
struct SessionPartition {
    session: PgSession,
    schema: SchemaRef,
    sql: String,
}
impl fmt::Debug for SessionPartition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostgresSessionPartition")
    }
}
impl PartitionStream for SessionPartition {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }
    fn execute(&self, task: Arc<TaskContext>) -> SendableRecordBatchStream {
        let session = self.session.clone();
        let schema = self.schema.clone();
        let sql = self.sql.clone();
        let query = QueryContext::from_task(&task)
            .unwrap_or_else(|| QueryContext::new(QueryOptions::default()).unwrap());
        let output = schema.clone();
        let stream = async_stream::try_stream! {
            session.check().map_err(|e|failure(&e.to_string()))?;
            let collected=tokio::time::timeout_at(session.expires,async{
                let lease=session.lease.lock().await;let c=lease.as_ref().and_then(|l|l.client.as_ref()).ok_or_else(||failure("session closed"))?;
                let cursor=q(&format!("semantic_cursor_{}",semantic_runtime::unique_id().replace('-',"_")));
                query.run(async{c.batch_execute(&format!("DECLARE {cursor} NO SCROLL CURSOR FOR {sql}")).await.map_err(|_|failure("session scan failed"))}).await?;
                let mut batches=vec![];
                loop{query.request_started()?;let rows=query.run(async{c.query(&format!("FETCH FORWARD {} FROM {cursor}",session.postgres.batch_size),&[]).await.map_err(|_|failure("session fetch failed"))}).await?;if rows.is_empty(){break;}let b=batch(&schema,&rows)?;query.charge_decoded(b.get_array_memory_size())?;query.charge_remote(b.get_array_memory_size())?;batches.push(b);}
                c.batch_execute(&format!("CLOSE {cursor}")).await.map_err(|_|failure("session cursor cleanup failed"))?;Ok::<_,datafusion::error::DataFusionError>(batches)
            }).await.map_err(|_|failure("session expired")).and_then(|r|r);
            if collected.is_err(){session.poisoned.store(true,Ordering::Release);session.lease.lock().await.take();}
            for b in collected?{yield b;}
        };
        Box::pin(RecordBatchStreamAdapter::new(output, stream))
    }
}
