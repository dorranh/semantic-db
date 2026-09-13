use crate::{Connection, error, policy, transport};
use async_trait::async_trait;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::Session,
    error::Result,
    execution::TaskContext,
    physical_expr::EquivalenceProperties,
    physical_plan::{
        DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
        SendableRecordBatchStream,
        execution_plan::{Boundedness, EmissionType},
        metrics::{ExecutionPlanMetricsSet, MetricsSet},
    },
};
use datafusion_federation::{FederatedPlanNode, FederationPlanner};
use std::{fmt, sync::Arc};

#[derive(Debug)]
pub(crate) struct RemotePlanner(pub Arc<Connection>);
#[async_trait]
impl FederationPlanner for RemotePlanner {
    async fn plan_federation(
        &self,
        node: &FederatedPlanNode,
        _: &dyn Session,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let sql = policy::sql_for_plan(node.plan())?;
        Ok(Arc::new(RemoteExec::new(
            self.0.clone(),
            sql,
            Arc::new(node.plan().schema().as_arrow().clone()),
            node.plan().schema().fields().is_empty(),
        )))
    }
}
#[derive(Debug, Clone)]
pub(crate) struct RemoteExec {
    connection: Arc<Connection>,
    sql: String,
    schema: SchemaRef,
    zero_columns: bool,
    filters: Vec<Arc<dyn datafusion::physical_expr::PhysicalExpr>>,
    properties: Arc<PlanProperties>,
    metrics: ExecutionPlanMetricsSet,
}
impl RemoteExec {
    pub fn new(
        connection: Arc<Connection>,
        sql: String,
        schema: SchemaRef,
        zero_columns: bool,
    ) -> Self {
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(schema.clone()),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Self {
            connection,
            sql,
            schema,
            zero_columns,
            filters: vec![],
            properties,
            metrics: ExecutionPlanMetricsSet::new(),
        }
    }
}
impl DisplayAs for RemoteExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "VirtualExecutionPlan name=clickhouse sql={}", self.sql)
    }
}
impl ExecutionPlan for RemoteExec {
    fn name(&self) -> &str {
        "ClickHouseExec"
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }
    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !children.is_empty() {
            return Err(error("remote scan cannot have children"));
        }
        Ok(self)
    }
    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        if partition != 0 {
            return Err(error("invalid remote partition"));
        }
        Ok(transport::execute_context(
            self.connection.clone(),
            self.sql.clone(),
            self.schema.clone(),
            context,
            self.zero_columns,
            self.metrics.clone(),
            self.filters.clone(),
        ))
    }
    fn handle_child_pushdown_result(
        &self,
        _phase: datafusion::physical_plan::filter_pushdown::FilterPushdownPhase,
        result: datafusion::physical_plan::filter_pushdown::ChildPushdownResult,
        _config: &datafusion::common::config::ConfigOptions,
    ) -> Result<
        datafusion::physical_plan::filter_pushdown::FilterPushdownPropagation<
            Arc<dyn ExecutionPlan>,
        >,
    > {
        use datafusion::physical_plan::filter_pushdown::{FilterPushdownPropagation, PushedDown};
        let mut node = self.clone();
        node.filters = result
            .parent_filters
            .iter()
            .map(|f| f.filter.clone())
            .collect();
        Ok(FilterPushdownPropagation {
            filters: vec![PushedDown::No; node.filters.len()],
            updated_node: Some(Arc::new(node)),
        })
    }
    fn apply_expressions(
        &self,
        _: &mut dyn FnMut(
            &Arc<dyn datafusion::physical_plan::PhysicalExpr>,
        ) -> Result<datafusion::common::tree_node::TreeNodeRecursion>,
    ) -> Result<datafusion::common::tree_node::TreeNodeRecursion> {
        Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
    }
    fn metrics(&self) -> Option<MetricsSet> {
        Some(self.metrics.clone_inner())
    }
}
