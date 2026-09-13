use datafusion::{
    common::tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    error::Result,
    logical_expr::{Expr, LogicalPlan},
    optimizer::{OptimizerConfig, OptimizerRule},
};
use std::sync::Arc;

pub(super) fn optimizer_rules() -> Vec<Arc<dyn OptimizerRule + Send + Sync>> {
    let mut rules = datafusion_federation::default_optimizer_rules();
    let index = rules
        .iter()
        .position(|r| r.name() == "federation_optimizer_rule")
        .expect("federation rule");
    let rule = rules.remove(index);
    rules.push(Arc::new(GuardedFederation(rule)));
    rules
}

#[derive(Debug)]
struct GuardedFederation(Arc<dyn OptimizerRule + Send + Sync>);

impl OptimizerRule for GuardedFederation {
    fn name(&self) -> &str {
        "federation_optimizer_rule"
    }
    fn supports_rewrite(&self) -> bool {
        true
    }
    fn rewrite(
        &self,
        plan: LogicalPlan,
        config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let mut has_remote = false;
        let mut unsupported_subquery = false;
        plan.apply_with_subqueries(|node| {
            if let LogicalPlan::TableScan(scan) = node {
                has_remote |= datafusion_federation::get_table_source(&scan.source)?.is_some();
            }
            for expression in node.expressions() {
                expression.apply(|expr| {
                    unsupported_subquery |= matches!(expr, Expr::InSubquery(_));
                    Ok(TreeNodeRecursion::Continue)
                })?;
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        // Core 0.5.6 explicitly errors on a remaining InSubquery. Retain normal
        // scans for these plans, and do not involve federation in local queries.
        if !has_remote {
            return Ok(Transformed::no(plan));
        }
        if unsupported_subquery {
            let expressions = plan.expressions();
            let inputs = plan
                .inputs()
                .into_iter()
                .map(|input| self.rewrite(input.clone(), config).map(|value| value.data))
                .collect::<Result<Vec<_>>>()?;
            return Ok(Transformed::yes(plan.with_new_exprs(expressions, inputs)?));
        }
        self.0.rewrite(plan, config)
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn local_subqueries_and_information_schema_still_work() {
        let engine = crate::Engine::new();
        for sql in [
            "SELECT 1 AS found WHERE 1 IN (SELECT 1)",
            "SELECT (SELECT 42) AS answer",
            "SELECT table_name FROM information_schema.tables LIMIT 1",
        ] {
            engine.query(sql).await.unwrap();
        }
    }
}
