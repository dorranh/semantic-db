//! Runnable connector template: simulated API pages, shared CLI, real Ossie import.
//! Replace fetch_page with a bounded HTTP request and keep cursor state per scan.
use datafusion::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    catalog::{TableProvider, streaming::StreamingTable},
    execution::TaskContext,
    physical_plan::{
        SendableRecordBatchStream, stream::RecordBatchStreamAdapter, streaming::PartitionStream,
    },
};
use futures::{future::BoxFuture, stream};
use semantic_sources::{
    ConnectorFactory, Options, Registry, Result, SecretResolver, SourceConnection, SourceError,
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct DemoConnector {
    requests: Arc<AtomicUsize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionOptions {
    page_size: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableOptions {
    rows: usize,
}

fn parse<T: DeserializeOwned>(options: &Options) -> Result<T> {
    serde_json::from_value(serde_json::Value::Object(options.clone()))
        .map_err(|error| SourceError::configuration("options", "/", error.to_string()))
}
impl ConnectorFactory for DemoConnector {
    fn validate_connection(&self, options: &Options) -> Result<()> {
        let config: ConnectionOptions = parse(options)?;
        if !(1..=1000).contains(&config.page_size) {
            return Err(SourceError::configuration(
                "page_size",
                "/page_size",
                "expected 1–1000",
            ));
        }
        Ok(())
    }
    fn validate_source(&self, options: &Options) -> Result<()> {
        let config: TableOptions = parse(options)?;
        if config.rows > 10000 {
            return Err(SourceError::configuration(
                "scope",
                "/rows",
                "demo scope is at most 10000 rows",
            ));
        }
        Ok(())
    }
    fn connect<'a>(
        &'a self,
        options: &'a Options,
        _: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(async move {
            self.validate_connection(options)?;
            let config: ConnectionOptions = parse(options)?;
            Ok(Arc::new(DemoConnection {
                page_size: config.page_size,
                requests: self.requests.clone(),
            }) as Arc<dyn SourceConnection>)
        })
    }
}
struct DemoConnection {
    page_size: usize,
    requests: Arc<AtomicUsize>,
}
impl SourceConnection for DemoConnection {
    fn table<'a>(
        &'a self,
        options: &'a Options,
        _: &'a Path,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            let config: TableOptions = parse(options)?;
            let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
            let partition = Arc::new(DemoPartition {
                schema: schema.clone(),
                rows: config.rows,
                page_size: self.page_size,
                requests: self.requests.clone(),
            });
            Ok(Arc::new(StreamingTable::try_new(schema, vec![partition])?)
                as Arc<dyn TableProvider>)
        })
    }
}
#[derive(Debug)]
struct DemoPartition {
    schema: SchemaRef,
    rows: usize,
    page_size: usize,
    requests: Arc<AtomicUsize>,
}
impl PartitionStream for DemoPartition {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }
    fn execute(&self, _: Arc<TaskContext>) -> SendableRecordBatchStream {
        // A new cursor for every execution. Never put cursors on a shared client.
        let state = PageStream {
            next: 0,
            rows: self.rows,
            page_size: self.page_size,
            schema: self.schema.clone(),
            requests: self.requests.clone(),
        };
        Box::pin(RecordBatchStreamAdapter::new(
            self.schema.clone(),
            stream::try_unfold(state, |mut state| async move {
                Ok(state.fetch_page().await?.map(|batch| (batch, state)))
            }),
        ))
    }
}
struct PageStream {
    next: usize,
    rows: usize,
    page_size: usize,
    schema: SchemaRef,
    requests: Arc<AtomicUsize>,
}
impl PageStream {
    async fn fetch_page(&mut self) -> datafusion::error::Result<Option<RecordBatch>> {
        if self.next >= self.rows {
            return Ok(None);
        }
        // API-specific seam: replace this simulated page with a request. Validate
        // continuation cursors; enforce request/time/byte budgets; return errors
        // for partial data. Do not spawn detached tasks or prefetch by default.
        self.requests.fetch_add(1, Ordering::Relaxed);
        let end = (self.next + self.page_size).min(self.rows);
        let ids = Int64Array::from_iter_values((self.next..end).map(|n| n as i64));
        self.next = end;
        Ok(Some(RecordBatch::try_new(
            self.schema.clone(),
            vec![Arc::new(ids)],
        )?))
    }
}

#[tokio::main]
async fn main() -> semantic_cli::Result<()> {
    let mut registry = Registry::standard();
    registry.register("demo", DemoConnector::default())?;
    semantic_cli::run_with_registry(registry).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use semantic_sources::Project;
    #[tokio::test]
    async fn configured_pages_are_lazy_repeatable_and_residual_limits_are_correct() {
        let requests = Arc::new(AtomicUsize::new(0));
        let mut registry = Registry::new();
        registry
            .register(
                "demo",
                DemoConnector {
                    requests: requests.clone(),
                },
            )
            .unwrap();
        let project = Project::from_path(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/connectors/semantic-db.yaml"
        ))
        .unwrap();
        let engine = project.load(&registry, &|_| None).await.unwrap().engine;
        let frame = engine.plan_sql("SELECT * FROM items").await.unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 0);
        let mut stream = frame.execute_stream().await.unwrap();
        assert_eq!(stream.next().await.unwrap().unwrap().num_rows(), 2);
        drop(stream);
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        let rows = engine
            .query("SELECT id FROM items WHERE id >= 4 LIMIT 1")
            .await
            .unwrap();
        assert_eq!(
            rows[0]
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(0),
            4
        );
        assert_eq!(requests.load(Ordering::Relaxed), 4);
        for _ in 0..2 {
            let rows = engine.query("SELECT COUNT(*) FROM items").await.unwrap();
            assert_eq!(
                rows[0]
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .value(0),
                5
            );
        }
        assert_eq!(requests.load(Ordering::Relaxed), 10);
        let mut reference_sources = semantic_ossie::SourceBindings::new();
        reference_sources
            .bind(
                "demo.items",
                datafusion::prelude::SessionContext::new()
                    .sql("SELECT * FROM (VALUES (0::BIGINT), (1), (2), (3), (4)) AS items(id)")
                    .await
                    .unwrap()
                    .into_view(),
            )
            .unwrap();
        let reference = semantic_ossie::OssieDocument::parse(include_str!(
            "../../../examples/connectors/items.ossie.yaml"
        ))
        .unwrap()
        .load(None, &reference_sources)
        .unwrap();
        semantic_sources::conformance::check_query_equivalence(
            &engine,
            &reference.engine,
            &[
                "SELECT * FROM items ORDER BY id",
                "SELECT COUNT(*) FROM items",
                "SELECT id FROM items WHERE id >= 4 LIMIT 1",
            ],
        )
        .await
        .unwrap();
    }
}
