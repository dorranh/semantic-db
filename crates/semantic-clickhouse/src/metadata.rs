use crate::{ClickHouse, Connection, TableOptions, error, validate_identifier};
use datafusion::error::Result;
use semantic_runtime::{QueryContext, QueryOptions};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnMetadata {
    pub name: String,
    #[serde(rename = "type")]
    pub native_type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableMetadata {
    pub name: String,
    pub engine: String,
    pub total_rows: Option<u64>,
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub columns: Vec<ColumnMetadata>,
}
pub(crate) fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}
pub(crate) async fn rows<T: serde::de::DeserializeOwned>(
    connection: &Connection,
    sql: &str,
) -> Result<Vec<T>> {
    let query = QueryContext::new(QueryOptions {
        timeout_seconds: connection.config.query_timeout.as_secs().max(1),
        ..Default::default()
    })?;
    query
        .run(async {
            let _permit = connection
                .permits
                .acquire()
                .await
                .map_err(|_| error("connection closed"))?;
            let mut response = crate::http::request(
                connection,
                sql,
                "JSONEachRow",
                &query,
                &format!("{}-metadata", query.id),
            )
            .await?;
            let mut data = vec![];
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| error("metadata request failed"))?
            {
                if data.len().saturating_add(chunk.len()) > 4 * 1024 * 1024 {
                    return Err(error("metadata byte budget exhausted"));
                }
                data.extend_from_slice(&chunk);
            }
            data.split(|b| *b == b'\n')
                .filter(|l| !l.is_empty())
                .map(|line| {
                    serde_json::from_slice(line).map_err(|_| error("invalid metadata response"))
                })
                .collect()
        })
        .await
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCapabilities {
    pub version: String,
    pub timezone: String,
}
impl ClickHouse {
    /// Explicit read-only compatibility probe; construction and planning remain offline.
    pub async fn capabilities(&self) -> Result<ServerCapabilities> {
        rows(
            &self.0,
            "SELECT version() AS version, timezone() AS timezone",
        )
        .await?
        .pop()
        .ok_or_else(|| error("capability probe returned no rows"))
    }
    pub async fn table_names(&self) -> Result<Vec<String>> {
        Ok(self.discover().await?.into_iter().map(|t| t.name).collect())
    }
    pub async fn discover(&self) -> Result<Vec<TableMetadata>> {
        rows(&self.0,&format!("SELECT name,engine,total_rows,total_bytes FROM system.tables WHERE database={} ORDER BY name",literal(&self.0.config.database))).await
    }
    pub async fn metadata(&self, table: &str) -> Result<TableMetadata> {
        validate_identifier(table)?;
        if let Some((at, metadata)) = self
            .0
            .metadata
            .lock()
            .unwrap()
            .get(table)
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
        {
            let _ = at;
            return Ok(metadata.clone());
        }
        let where_sql = format!(
            "database={} AND name={}",
            literal(&self.0.config.database),
            literal(table)
        );
        let mut tables: Vec<TableMetadata> = rows(
            &self.0,
            &format!(
                "SELECT name,engine,total_rows,total_bytes FROM system.tables WHERE {where_sql}"
            ),
        )
        .await?;
        let mut metadata = tables
            .pop()
            .ok_or_else(|| error("table not found or inaccessible"))?;
        metadata.columns = rows(&self.0,&format!("SELECT name,type FROM system.columns WHERE database={} AND table={} ORDER BY position",literal(&self.0.config.database),literal(table))).await?;
        let mut cache = self.0.metadata.lock().unwrap();
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(table.to_owned(), (Instant::now(), metadata.clone()));
        Ok(metadata)
    }
    pub fn invalidate_metadata(&self, table: &str) {
        self.0.metadata.lock().unwrap().remove(table);
    }
    pub async fn refresh_table(
        &self,
        table: &str,
        options: TableOptions,
    ) -> Result<std::sync::Arc<dyn datafusion::catalog::TableProvider>> {
        self.invalidate_metadata(table);
        self.table_with_options(table, options).await
    }
    pub fn authorization_scope(&self) -> String {
        let c = &self.0.config;
        semantic_runtime::fingerprint(&[
            c.endpoint.as_bytes(),
            c.database.as_bytes(),
            c.user.as_bytes(),
            c.password.as_bytes(),
            c.bearer_token.as_deref().unwrap_or_default().as_bytes(),
            c.identity_pem.as_deref().unwrap_or_default().as_bytes(),
            c.roles.join("\0").as_bytes(),
        ])
    }
}
