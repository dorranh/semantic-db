use super::*;
use semantic_postgres::{Postgres, PostgresTlsConfig};

pub struct PostgresConnector;
struct Connection(Postgres);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionOptions {
    connection_string_env: String,
    pool_size: Option<usize>,
    batch_size: Option<usize>,
    ca_pem_env: Option<String>,
    client_cert_pem_env: Option<String>,
    client_key_pem_env: Option<String>,
    #[serde(default)]
    write_enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceOptions {
    schema: String,
    table: String,
}
impl ConnectorFactory for PostgresConnector {
    fn resource_namespace(&self) -> Option<&'static str> {
        Some("postgres")
    }
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
        for reference in [&c.ca_pem_env, &c.client_cert_pem_env, &c.client_key_pem_env] {
            if reference
                .as_ref()
                .is_some_and(|name| name.trim().is_empty())
            {
                return Err(SourceError::configuration(
                    "options",
                    "/",
                    "Postgres certificate secret references must be nonempty",
                ));
            }
        }
        if c.client_cert_pem_env.is_some() != c.client_key_pem_env.is_some() {
            return Err(SourceError::configuration(
                "options",
                "/",
                "Postgres client certificate and key must be supplied together",
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
            self.validate_connection(value)?;
            let c: ConnectionOptions = options(value)?;
            let url = secrets(&c.connection_string_env).ok_or_else(|| {
                SourceError::configuration(
                    "missing_secret",
                    "/connection_string_env",
                    "provide the named Postgres connection secret",
                )
            })?;
            let secret =
                |reference: &Option<String>, path: &'static str| -> Result<Option<String>> {
                    reference
                        .as_ref()
                        .map(|name| {
                            secrets(name).ok_or_else(|| {
                                SourceError::configuration(
                                    "missing_secret",
                                    path,
                                    "provide the named Postgres certificate secret",
                                )
                            })
                        })
                        .transpose()
                };
            let tls = PostgresTlsConfig {
                ca_pem: secret(&c.ca_pem_env, "/ca_pem_env")?,
                client_cert_pem: secret(&c.client_cert_pem_env, "/client_cert_pem_env")?,
                client_key_pem: secret(&c.client_key_pem_env, "/client_key_pem_env")?,
            };
            let postgres = Postgres::new_with_tls(
                &url,
                c.pool_size.unwrap_or(8),
                c.batch_size.unwrap_or(1024),
                tls,
            )?;
            Ok(Arc::new(Connection(if c.write_enabled {
                postgres.with_writes()
            } else {
                postgres
            })) as Arc<dyn SourceConnection>)
        })
    }
}
impl SourceConnection for Connection {
    fn read_connection(&self) -> Option<Arc<dyn semantic_engine::ReadConnection>> {
        Some(Arc::new(self.0.clone()))
    }
    fn write_connection(&self) -> Option<Arc<dyn semantic_engine::WriteConnection>> {
        self.0
            .writes_enabled()
            .then(|| Arc::new(self.0.clone()) as Arc<dyn semantic_engine::WriteConnection>)
    }
    fn resource<'a>(
        &'a self,
        value: &'a Options,
        _: &'a Path,
    ) -> BoxFuture<'a, Result<SourceResource>> {
        Box::pin(async move {
            let c: SourceOptions = options(value)?;
            let (provider, read, write) = self.0.bindings(&c.schema, &c.table).await?;
            Ok(SourceResource {
                provider,
                read: Some(read),
                write,
            })
        })
    }
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
