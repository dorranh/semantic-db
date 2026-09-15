use datafusion_postgres::pgwire::tokio::process_socket;
use semantic_db::Engine;
use semantic_server::protocol::Handlers;
use std::sync::Arc;
use tokio_postgres::{NoTls, types::Type};
async fn server() -> (u16, tokio::task::JoinHandle<()>) {
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let handlers = Arc::new(Handlers::new(Arc::new(Engine::new())));
    (
        port,
        tokio::spawn(async move {
            while let Ok((socket, _)) = socket.accept().await {
                let h = handlers.clone();
                tokio::spawn(async move {
                    let _ = process_socket(socket, None, h).await;
                });
            }
        }),
    )
}
#[tokio::test]
async fn protocol_parameters_prepared_statements_and_recovery() {
    let (port, server) = server().await;
    let mut unsupported = tokio_postgres::Config::new();
    unsupported
        .host("127.0.0.1")
        .port(port)
        .user("local")
        .options("-c search_path=private");
    assert!(unsupported.connect(NoTls).await.is_err());
    let (client, conn) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=local dbname=semantic"),
        NoTls,
    )
    .await
    .unwrap();
    assert_eq!(conn.parameter("default_transaction_read_only"), Some("on"));
    tokio::spawn(conn);
    assert!(
        !client
            .simple_query("SELECT 42 AS answer")
            .await
            .unwrap()
            .is_empty()
    );
    for sql in [
        "BEGIN",
        "COMMIT",
        "ROLLBACK",
        "SET x = 1",
        "CREATE TABLE x(a int)",
        "SELECT 1; SELECT 2",
        "EXPLAIN INSERT INTO x VALUES(1)",
    ] {
        assert!(client.simple_query(sql).await.is_err(), "{sql}");
        assert!(client.simple_query("SELECT 1").await.is_ok());
    }
    let statement = client
        .prepare_typed("SELECT $1 AS value", &[Type::INT8])
        .await
        .unwrap();
    assert_eq!(
        client
            .query_one(&statement, &[&42i64])
            .await
            .unwrap()
            .get::<_, i64>(0),
        42
    );
    assert!(
        client
            .query_one(&statement, &[&None::<i64>])
            .await
            .unwrap()
            .get::<_, Option<i64>>(0)
            .is_none()
    );
    let statement = client.prepare("SELECT $1::text AS value").await.unwrap();
    let value = "'; DELETE FROM packages; --";
    assert_eq!(
        client
            .query_one(&statement, &[&value])
            .await
            .unwrap()
            .get::<_, String>(0),
        value
    );
    assert_eq!(
        client
            .query_one(&statement, &[&"second"])
            .await
            .unwrap()
            .get::<_, String>(0),
        "second"
    );
    drop(client);
    server.abort();
}

#[derive(Debug)]
struct FaultStream {
    schema: datafusion::arrow::datatypes::SchemaRef,
    slow: bool,
}
impl datafusion::physical_plan::streaming::PartitionStream for FaultStream {
    fn schema(&self) -> &datafusion::arrow::datatypes::SchemaRef {
        &self.schema
    }
    fn execute(
        &self,
        _: Arc<datafusion::execution::TaskContext>,
    ) -> datafusion::physical_plan::SendableRecordBatchStream {
        let schema = self.schema.clone();
        let output = schema.clone();
        let slow = self.slow;
        Box::pin(
            datafusion::physical_plan::stream::RecordBatchStreamAdapter::new(
                output,
                async_stream::try_stream! {
                    yield datafusion::arrow::record_batch::RecordBatch::try_new(schema,vec![Arc::new(datafusion::arrow::array::Int64Array::from(vec![1]))]).unwrap();
                    if slow {tokio::time::sleep(std::time::Duration::from_secs(30)).await;}
                    Err(datafusion::error::DataFusionError::Execution("injected late error".into()))?;
                },
            ),
        )
    }
}
#[tokio::test]
async fn late_errors_and_cancel_do_not_publish_partial_success() {
    use datafusion::{
        arrow::datatypes::{DataType, Field, Schema},
        catalog::{TableProvider, streaming::StreamingTable},
    };
    let mut engine = Engine::new();
    for (name, slow) in [("late", false), ("slow", true)] {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let table = Arc::new(
            StreamingTable::try_new(schema.clone(), vec![Arc::new(FaultStream { schema, slow })])
                .unwrap(),
        );
        engine
            .register_table(
                semantic_catalog::Relation::base(name, table.schema(), "fixture"),
                table,
            )
            .unwrap();
    }
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let handlers = Arc::new(Handlers::new(Arc::new(engine)));
    let server = tokio::spawn(async move {
        while let Ok((s, _)) = socket.accept().await {
            let h = handlers.clone();
            tokio::spawn(async move {
                let _ = process_socket(s, None, h).await;
            });
        }
    });
    let (client, conn) =
        tokio_postgres::connect(&format!("host=127.0.0.1 port={port} user=local"), NoTls)
            .await
            .unwrap();
    tokio::spawn(conn);
    assert!(client.simple_query("SELECT * FROM late").await.is_err());
    let token = client.cancel_token();
    let pending = client.simple_query("SELECT * FROM slow");
    tokio::pin!(pending);
    tokio::select! {r=&mut pending=>panic!("slow query completed too soon: {r:?}"),_=tokio::time::sleep(std::time::Duration::from_millis(100))=>{}}
    token.cancel_query(NoTls).await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut pending)
            .await
            .unwrap()
            .is_err()
    );
    assert!(client.simple_query("SELECT 1").await.is_ok());
    server.abort();
}

struct FakeWrites {
    schema: datafusion::arrow::datatypes::SchemaRef,
    calls: std::sync::atomic::AtomicUsize,
    stall: std::sync::atomic::AtomicBool,
    started: tokio::sync::Notify,
}
impl semantic_db::WriteConnection for FakeWrites {
    fn inspect_target<'a>(
        &'a self,
        _: &'a str,
    ) -> futures::future::BoxFuture<'a, semantic_db::engine::Result<semantic_db::TargetInspection>>
    {
        Box::pin(async move {
            Ok(semantic_db::TargetInspection {
                physical_namespace: "fake".into(),
                schema: self.schema.clone(),
                revision: "v1".into(),
                resource: "items".into(),
                domain: "fake".into(),
                unique_keys: vec![vec!["id".into()]],
                checked_eligible: true,
                supported_operations: vec!["UPDATE".into()],
                atomic_writes: true,
            })
        })
    }
    fn validate_operation<'a>(
        &'a self,
        _: &'a semantic_db::MutationPlan,
    ) -> futures::future::BoxFuture<'a, semantic_db::engine::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn apply<'a>(
        &'a self,
        _: &'a semantic_db::MutationPlan,
        _: &'a semantic_db::StagedInput,
        _: &'a [datafusion::common::ScalarValue],
        _: Arc<semantic_db::QueryContext>,
    ) -> futures::future::BoxFuture<'a, semantic_db::engine::Result<semantic_db::WriteResult>> {
        Box::pin(async move {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.started.notify_one();
            if self.stall.load(std::sync::atomic::Ordering::SeqCst) {
                futures::future::pending::<()>().await;
            }
            Ok(semantic_db::WriteResult {
                outcome: semantic_db::WriteOutcome::Committed,
                operation_id: "fake".into(),
                boundary: "fake".into(),
                atomic: true,
                idempotent: false,
                receipt: None,
                affected_rows: Some(1),
                code: None,
                message: None,
                external_observations: vec![],
            })
        })
    }
    fn begin<'a>(
        &'a self,
        _: semantic_db::TransactionOptions,
    ) -> futures::future::BoxFuture<
        'a,
        semantic_db::engine::Result<Arc<dyn semantic_db::ConnectorSession>>,
    > {
        Box::pin(async {
            Err(datafusion::error::DataFusionError::Execution("unsupported".into()).into())
        })
    }
}
#[tokio::test]
async fn writes_prepare_without_mutation_and_report_unknown_on_protocol_cancellation() {
    use datafusion::{
        arrow::datatypes::{DataType, Field, Schema},
        datasource::MemTable,
    };
    use std::{collections::BTreeMap, sync::atomic::Ordering};
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let connection = Arc::new(FakeWrites {
        schema: schema.clone(),
        calls: 0.into(),
        stall: false.into(),
        started: tokio::sync::Notify::new(),
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            semantic_db::Relation::base("items", schema.clone(), "fake"),
            Arc::new(MemTable::try_new(schema, vec![vec![]]).unwrap()),
        )
        .unwrap();
    engine
        .attach_write_binding(
            "items",
            semantic_db::WriteBinding {
                connection: connection.clone(),
                target: "items".into(),
                columns: BTreeMap::from([("id".into(), "id".into())]),
            },
        )
        .await
        .unwrap();
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let handlers = Arc::new(Handlers::new(Arc::new(engine)));
    let server = tokio::spawn(async move {
        while let Ok((s, _)) = socket.accept().await {
            let h = handlers.clone();
            tokio::spawn(async move {
                let _ = process_socket(s, None, h).await;
            });
        }
    });
    let (client, conn) =
        tokio_postgres::connect(&format!("host=127.0.0.1 port={port} user=local"), NoTls)
            .await
            .unwrap();
    tokio::spawn(conn);
    let prepared = client
        .prepare("UPDATE items SET id=$1::bigint")
        .await
        .unwrap();
    assert!(prepared.columns().is_empty());
    client
        .simple_query("EXPLAIN UPDATE items SET id=1")
        .await
        .unwrap();
    assert_eq!(connection.calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.execute(&prepared, &[&1i64]).await.unwrap(), 1);
    connection.started.notified().await;
    connection.stall.store(true, Ordering::SeqCst);
    for prepared_mode in [false, true] {
        let token = client.cancel_token();
        let pending = async {
            if prepared_mode {
                client.execute(&prepared, &[&2i64]).await.map(|_| ())
            } else {
                client
                    .simple_query("UPDATE items SET id=2")
                    .await
                    .map(|_| ())
            }
        };
        let cancellation = async {
            connection.started.notified().await;
            token.cancel_query(NoTls).await.unwrap();
        };
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(pending, cancellation)
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        assert_eq!(error.code().unwrap().code(), "08007");
        assert!(
            error
                .as_db_error()
                .unwrap()
                .message()
                .contains("OutcomeUnknown")
        );
        assert!(client.simple_query("SELECT 1").await.is_ok());
    }
    server.abort();
}
