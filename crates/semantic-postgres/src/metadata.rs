use crate::*;
#[derive(Debug, Clone)]
pub struct ServerCapabilities {
    pub version_number: i32,
    pub read_only: bool,
}
#[derive(Debug, Clone)]
pub struct TableMetadata {
    pub oid: u32,
    pub schema: SchemaRef,
    pub estimated_rows: Option<usize>,
    pub estimated_bytes: Option<usize>,
    pub revision: String,
}
pub(crate) async fn inspect(
    client: &tokio_postgres::Client,
    target: &str,
) -> Result<TableMetadata> {
    let statement = client
        .prepare(&format!("SELECT * FROM {target} LIMIT 0"))
        .await
        .map_err(|_| failure("Postgres table inspection failed"))?;
    // Parse alone does not enforce SELECT privileges. Execute the empty result
    // before advertising a usable binding.
    client
        .query(&statement, &[])
        .await
        .map_err(|_| failure("Postgres table inspection denied or failed"))?;
    let rows=client.query("SELECT attname, atttypmod, attnotnull, atttypid, attnum, attcollation FROM pg_catalog.pg_attribute WHERE attrelid=$1::text::regclass AND attnum>0 AND NOT attisdropped ORDER BY attnum",&[&target]).await.map_err(|_|failure("Postgres column inspection failed"))?;
    let mut fields = vec![];
    let mut revision = String::new();
    for c in statement.columns() {
        let row = rows
            .iter()
            .find(|r| r.get::<_, String>(0) == c.name())
            .ok_or_else(|| failure("Postgres column metadata changed"))?;
        let modifier: i32 = row.get(1);
        let required: bool = row.get(2);
        let oid: u32 = row.get(3);
        let number: i16 = row.get(4);
        let collation: u32 = row.get(5);
        if Type::from_oid(oid).is_none() {
            return Err(failure(
                "Postgres domains, enums and custom types require an explicit supported view cast",
            ));
        }
        let ty = if c.type_() == &Type::NUMERIC {
            codec::decimal_type(modifier)?
        } else {
            arrow_type(c.type_())?
        };
        fields.push(Field::new(c.name(), ty, !required));
        revision.push_str(&format!(
            "{:?}:{oid}:{modifier}:{required}:{number}:{collation};",
            c.name()
        ));
    }
    let row=client.query_one("SELECT oid, reltuples::double precision, relpages::bigint, current_setting('block_size')::bigint, CASE WHEN relkind IN ('v','m') THEN pg_get_viewdef(oid,true) ELSE relkind::text END FROM pg_catalog.pg_class WHERE oid=$1::text::regclass",&[&target]).await.map_err(|_|failure("Postgres relation inspection failed"))?;
    let oid: u32 = row.get(0);
    let count: f64 = row.get(1);
    let pages: i64 = row.get(2);
    revision.push_str(&oid.to_string());
    revision.push_str(&row.get::<_, String>(4));
    let block_size = usize::try_from(row.get::<_, i64>(3)).ok();
    Ok(TableMetadata {
        oid,
        schema: Arc::new(Schema::new(fields)),
        estimated_rows: (count >= 0.0 && count.is_finite()).then_some(count.max(0.0) as usize),
        estimated_bytes: usize::try_from(pages)
            .ok()
            .and_then(|p| block_size.and_then(|b| p.checked_mul(b))),
        revision: semantic_runtime::fingerprint(&[revision.as_bytes()]),
    })
}
impl Postgres {
    pub(crate) async fn acquire(&self) -> Result<Object> {
        tokio::time::timeout(
            Duration::from_millis(self.options.acquire_timeout_ms),
            self.pool.get(),
        )
        .await
        .map_err(|_| failure("Postgres connection acquisition timed out"))?
        .map_err(|_| failure("Postgres connection failed"))
    }
    pub async fn capabilities(&self) -> Result<ServerCapabilities> {
        let mut lease = self.lease(self.acquire().await?);
        let client = lease.client.as_ref().unwrap();
        let row=tokio::time::timeout(Duration::from_millis(self.options.query_timeout_ms),client.query_one("SELECT current_setting('server_version_num')::int, current_setting('default_transaction_read_only')::boolean",&[])).await.map_err(|_|failure("Postgres capability inspection timed out"))?.map_err(|_|failure("Postgres capability inspection failed"))?;
        lease.clean = true;
        Ok(ServerCapabilities {
            version_number: row.get(0),
            read_only: row.get(1),
        })
    }
    pub async fn metadata(&self, namespace: &str, table: &str) -> Result<TableMetadata> {
        let target = format!("{}.{}", identifier(namespace)?, identifier(table)?);
        self.metadata_target(&target).await
    }
    pub(crate) async fn metadata_target(&self, target: &str) -> Result<TableMetadata> {
        let mut lease = self.lease(self.acquire().await?);
        let client = lease.client.as_ref().unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(self.options.query_timeout_ms),
            inspect(client, target),
        )
        .await
        .map_err(|_| failure("Postgres metadata timed out"))??;
        lease.clean = true;
        Ok(result)
    }
    /// Return a newly inspected provider. Existing providers retain their revision
    /// and fail closed when the physical relation or column contract changes.
    pub async fn refresh_table(
        &self,
        namespace: &str,
        table: &str,
    ) -> Result<Arc<dyn TableProvider>> {
        self.table(namespace, table).await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PoolHealth {
    pub maximum: usize,
    pub connections: usize,
    pub available: usize,
    pub waiting: usize,
    pub closed: bool,
}
impl Postgres {
    pub fn pool_health(&self) -> PoolHealth {
        let s = self.pool.status();
        PoolHealth {
            maximum: s.max_size,
            connections: s.size,
            available: s.available,
            waiting: s.waiting,
            closed: self.pool.is_closed(),
        }
    }
    /// Stop new acquisitions (including queued work). Existing leases finish on
    /// their original connection. Re-resolve secrets and construct a new Postgres
    /// instance to rotate credentials; it receives a new federation identity.
    pub fn close(&self) {
        self.pool.close();
    }
}
