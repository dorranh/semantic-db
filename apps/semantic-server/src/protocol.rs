use async_trait::async_trait;
use datafusion::{
    arrow::datatypes::{DataType, TimeUnit},
    common::ScalarValue,
};
use datafusion_postgres::{
    arrow_pg::datatypes::{
        arrow_schema_to_pg_fields, df::deserialize_parameters, encode_recordbatch, into_pg_type,
    },
    pgwire,
};
use futures::{Sink, StreamExt, stream};
use pgwire::{
    api::{
        ClientInfo, ClientPortalStore, ConnectionGuard, ConnectionHandle, ConnectionManager,
        PgWireServerHandlers, PidSecretKeyGenerator, RandomPidSecretKeyGenerator, Type,
        auth::{self, DefaultServerParameterProvider, StartupHandler},
        cancel::{CancelHandler, DefaultCancelHandler},
        portal::{Format, Portal},
        query::{ExtendedQueryHandler, SimpleQueryHandler},
        results::{FieldInfo, QueryResponse, Response},
        stmt::QueryParser,
        store::PortalStore,
    },
    error::{ErrorInfo, PgWireError, PgWireResult},
    messages::{PgWireBackendMessage, PgWireFrontendMessage},
};
use semantic_db::{Engine, ReadDescription};
use std::sync::Arc;

pub struct Handlers {
    service: Arc<Service>,
    startup: Arc<Startup>,
    cancel: Arc<DefaultCancelHandler>,
}
impl Handlers {
    pub fn new(engine: Arc<Engine>) -> Self {
        let manager = Arc::new(ConnectionManager::new());
        Self {
            service: Arc::new(Service { engine }),
            startup: Arc::new(Startup(manager.clone())),
            cancel: Arc::new(DefaultCancelHandler::new(manager)),
        }
    }
}
impl PgWireServerHandlers for Handlers {
    fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
        self.service.clone()
    }
    fn extended_query_handler(&self) -> Arc<impl ExtendedQueryHandler> {
        self.service.clone()
    }
    fn startup_handler(&self) -> Arc<impl StartupHandler> {
        self.startup.clone()
    }
    fn cancel_handler(&self) -> Arc<impl CancelHandler> {
        self.cancel.clone()
    }
}
pub struct Startup(Arc<ConnectionManager>);
#[async_trait]
impl StartupHandler for Startup {
    async fn on_startup<C>(
        &self,
        client: &mut C,
        message: PgWireFrontendMessage,
    ) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: std::fmt::Debug,
        PgWireError: From<C::Error>,
    {
        if let PgWireFrontendMessage::Startup(startup) = message {
            for (name, value) in &startup.parameters {
                let supported = match name.as_str() {
                    "user" | "database" | "application_name" => true,
                    "client_encoding" => {
                        matches!(value.to_ascii_uppercase().as_str(), "UTF8" | "UTF-8")
                    }
                    "options" => value.is_empty(),
                    _ => name.starts_with("_pq_."),
                };
                if !supported {
                    return Err(error(format!("unsupported startup parameter: {name}")));
                }
            }
            auth::protocol_negotiation(client, &startup).await?;
            auth::save_startup_parameters_to_metadata(client, &startup);
            let (pid, key) = RandomPidSecretKeyGenerator::default().generate(client);
            client.set_pid_and_secret_key(pid, key);
            let (pid, key) = client.pid_and_secret_key();
            let (handle, guard) = self.0.register(pid, key);
            client
                .session_extensions()
                .insert::<Arc<ConnectionHandle>>(handle);
            client.session_extensions().insert::<ConnectionGuard>(guard);
            let mut parameters = DefaultServerParameterProvider::default();
            parameters.default_transaction_read_only = true;
            parameters.is_superuser = false;
            auth::finish_authentication(client, &parameters).await?;
        }
        Ok(())
    }
}
pub struct Service {
    engine: Arc<Engine>,
}
#[derive(Clone)]
pub struct Statement {
    sql: String,
    description: ReadDescription,
}
fn error(message: impl Into<String>) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".into(),
        "0A000".into(),
        message.into(),
    )))
}
fn engine_error(e: semantic_db::EngineError) -> PgWireError {
    error(e.to_string())
}
impl Service {
    async fn read(
        &self,
        sql: &str,
        values: Vec<ScalarValue>,
        format: &Format,
    ) -> PgWireResult<Response> {
        let mut execution = self
            .engine
            .execute_parameters(sql, values, self.engine.query_options().clone())
            .await
            .map_err(engine_error)?;
        let fields = Arc::new(arrow_schema_to_pg_fields(
            execution.stream.schema().as_ref(),
            format,
            None,
        )?);
        // Collected, bounded responses keep native resources and cancellation
        // within do_query. No rows escape when a later source page fails.
        let mut batches = Vec::new();
        let mut bytes = 0usize;
        let mut rows = 0usize;
        while let Some(batch) = execution.stream.next().await {
            let batch = batch.map_err(|_| error("source query failed; no complete result"))?;
            bytes = bytes.saturating_add(batch.get_array_memory_size());
            rows = rows.saturating_add(batch.num_rows());
            if bytes > 64 * 1024 * 1024 || rows > 100_000 {
                execution.cancel();
                return Err(error("server result limit exceeded (64 MiB / 100000 rows)"));
            }
            batches.push(batch);
        }
        let output_fields = fields.clone();
        let rows = batches
            .into_iter()
            .flat_map(move |batch| encode_recordbatch(output_fields.clone(), batch));
        Ok(Response::Query(QueryResponse::new(
            fields,
            stream::iter(rows),
        )))
    }
}
#[async_trait]
impl SimpleQueryHandler for Service {
    async fn do_query<C>(&self, _: &mut C, query: &str) -> PgWireResult<Vec<Response>>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore,
        C::Error: std::fmt::Debug,
        PgWireError: From<C::Error>,
    {
        Ok(vec![self.read(query, vec![], &Format::UnifiedText).await?])
    }
}
#[async_trait]
impl QueryParser for Service {
    type Statement = Statement;
    async fn parse_sql<C>(
        &self,
        _: &C,
        sql: &str,
        types: &[Option<Type>],
    ) -> PgWireResult<Option<Statement>>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        let hints = types
            .iter()
            .map(|t| {
                t.as_ref()
                    .filter(|t| **t != Type::UNKNOWN)
                    .map(arrow_type)
                    .transpose()
            })
            .collect::<PgWireResult<Vec<_>>>()?;
        let description = self
            .engine
            .describe_read(sql, &hints)
            .await
            .map_err(engine_error)?;
        Ok(Some(Statement {
            sql: sql.to_owned(),
            description,
        }))
    }
    fn get_parameter_types(&self, stmt: &Statement) -> PgWireResult<Vec<Type>> {
        stmt.description
            .parameters
            .iter()
            .map(into_pg_type)
            .collect()
    }
    fn get_result_schema(
        &self,
        stmt: &Statement,
        format: Option<&Format>,
    ) -> PgWireResult<Vec<FieldInfo>> {
        arrow_schema_to_pg_fields(
            &stmt.description.schema,
            format.unwrap_or(&Format::UnifiedText),
            None,
        )
    }
}
#[async_trait]
impl ExtendedQueryHandler for Service {
    type Statement = Statement;
    type QueryParser = Service;
    fn query_parser(&self) -> Arc<Service> {
        Arc::new(Service {
            engine: self.engine.clone(),
        })
    }
    async fn do_query<C>(
        &self,
        _: &mut C,
        portal: &Portal<Statement>,
        _: usize,
    ) -> PgWireResult<Response>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore,
        C::Error: std::fmt::Debug,
        PgWireError: From<C::Error>,
    {
        let stmt = &portal.statement.statement;
        let types = stmt
            .description
            .parameters
            .iter()
            .map(Some)
            .collect::<Vec<_>>();
        let values = deserialize_parameters(portal, &types)?;
        let datafusion::common::ParamValues::List(values) = values else {
            return Err(error("positional parameters required"));
        };
        self.read(
            &stmt.sql,
            values.into_iter().map(|v| v.value).collect(),
            &portal.result_column_format,
        )
        .await
    }
}
fn arrow_type(ty: &Type) -> PgWireResult<DataType> {
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
        _ => return Err(error("unsupported parameter OID")),
    })
}
