//! Execution APIs that carry budgets through every remote subplan.
use crate::{Engine, RecordBatch, Result};
use datafusion::{
    physical_plan::{SendableRecordBatchStream, stream::RecordBatchStreamAdapter},
    prelude::SessionConfig,
};
use futures::{StreamExt, TryStreamExt};
use semantic_runtime::{QueryContext, QueryOptions};
use std::sync::Arc;

pub struct PreparedQuery<'a> {
    engine: &'a Engine,
    sql: String,
}
pub struct QueryExecution {
    pub context: Arc<QueryContext>,
    pub stream: SendableRecordBatchStream,
}
impl QueryExecution {
    pub fn cancel(&self) {
        self.context.cancel();
    }
    pub async fn collect(self) -> Result<Vec<RecordBatch>> {
        Ok(self.stream.try_collect().await?)
    }
}
impl PreparedQuery<'_> {
    pub async fn execute(&self, options: QueryOptions) -> Result<QueryExecution> {
        self.engine.execute(&self.sql, options).await
    }
}
impl Engine {
    pub fn set_query_options(&mut self, options: QueryOptions) -> Result<()> {
        options.validate()?;
        self.query_options = options;
        Ok(())
    }
    pub fn query_options(&self) -> &QueryOptions {
        &self.query_options
    }
    pub async fn prepare(&self, sql: &str) -> Result<PreparedQuery<'_>> {
        self.plan_sql(sql).await?;
        Ok(PreparedQuery {
            engine: self,
            sql: sql.to_owned(),
        })
    }
    pub async fn execute(&self, sql: &str, options: QueryOptions) -> Result<QueryExecution> {
        let context = QueryContext::new(options)?;
        let frame = self.execution_frame(sql, &context).await?;
        execute_frame(frame, context).await
    }
}
pub(crate) async fn execute_frame(
    frame: datafusion::dataframe::DataFrame,
    context: Arc<QueryContext>,
) -> Result<QueryExecution> {
    let physical = context.run(frame.create_physical_plan()).await?;
    let task = frame.task_ctx();
    let config: SessionConfig = task
        .session_config()
        .clone()
        .with_extension(context.clone());
    let mut stream = datafusion::physical_plan::execute_stream(
        physical,
        Arc::new(task.with_session_config(config)),
    )?;
    let schema = stream.schema();
    let state = context.clone();
    let guarded = async_stream::try_stream! {
        loop {
            let batch = state.run(async { stream.next().await.transpose() }).await?;
            let Some(batch) = batch else { break; };
            yield batch;
        }
    };
    Ok(QueryExecution {
        context,
        stream: Box::pin(RecordBatchStreamAdapter::new(schema, guarded)),
    })
}
