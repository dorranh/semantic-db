use super::*;
use semantic_postgres::Postgres;

pub struct PostgresConnector;
struct Connection(Postgres);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionOptions {
    connection_string_env: String,
    pool_size: Option<usize>,
    batch_size: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceOptions {
    schema: String,
    table: String,
}
impl ConnectorFactory for PostgresConnector {
    fn validate_connection(&self, value: &Options) -> Result<()> {
        let c: ConnectionOptions = options(value)?;
        if c.connection_string_env.trim().is_empty()
            || c.pool_size == Some(0)
            || c.batch_size.is_some_and(|n| !(1..=8192).contains(&n))
        {
            return Err(SourceError::configuration(
                "options",
                "/",
                "Postgres secret reference and positive pool/batch sizes required",
            ));
        }
        Ok(())
    }
    fn validate_source(&self, value: &Options) -> Result<()> {
        let c: SourceOptions = options(value)?;
        if [&c.schema, &c.table]
            .iter()
            .any(|s| s.is_empty() || s.contains('\0'))
        {
            return Err(SourceError::configuration(
                "options",
                "/",
                "explicit schema and table identifiers required",
            ));
        }
        Ok(())
    }
    fn connect<'a>(
        &'a self,
        value: &'a Options,
        secrets: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(async move {
            let c: ConnectionOptions = options(value)?;
            let url = secrets(&c.connection_string_env).ok_or_else(|| {
                SourceError::configuration(
                    "missing_secret",
                    "/connection_string_env",
                    "provide the named Postgres connection secret",
                )
            })?;
            Ok(Arc::new(Connection(Postgres::new(
                &url,
                c.pool_size.unwrap_or(8),
                c.batch_size.unwrap_or(1024),
            )?)) as Arc<dyn SourceConnection>)
        })
    }
}
impl SourceConnection for Connection {
    fn table<'a>(
        &'a self,
        value: &'a Options,
        _: &'a Path,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            let c: SourceOptions = options(value)?;
            Ok(self.0.table(&c.schema, &c.table).await?)
        })
    }
}
