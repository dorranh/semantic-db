use datafusion::functions_aggregate::{
    average::Avg,
    count::Count,
    min_max::{Max, Min},
    sum::Sum,
};
use datafusion::{
    common::tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    error::Result,
    logical_expr::{Expr, Extension, JoinType, LogicalPlan, Operator, SubqueryAlias},
    sql::{sqlparser::ast, unparser::dialect::Dialect},
};
use datafusion_federation::FederatedPlanNode;
use std::{any::Any, sync::Arc};

pub(super) struct ClickHouseDialect;
impl Dialect for ClickHouseDialect {
    fn identifier_quote_style(&self, _: &str) -> Option<char> {
        Some('"')
    }
    fn supports_column_alias_in_table_alias(&self) -> bool {
        false
    }
    fn supports_empty_select_list(&self) -> bool {
        false
    }
}

/// The generic federation rule is optimistic. If a candidate includes an
/// unqualified operation, return its ordinary plan and use correct scan fallback.
/// No arbitrary scalar UDF or cast is forwarded to the remote database.
pub(super) fn restrict_federation(plan: LogicalPlan) -> Result<LogicalPlan> {
    if let LogicalPlan::Extension(extension) = &plan
        && let Some(node) = extension.node.as_any().downcast_ref::<FederatedPlanNode>()
    {
        if !supported(node.plan())? {
            return Ok(node.plan().clone());
        }
        // DataFusion 55's unparser keeps the inner name for consecutive aliases,
        // leaving outer qualified join keys unresolved. Ossie view inlining
        // creates precisely this shape. Only the outer alias is visible here.
        let rewritten = node.plan().clone().transform_up(|plan| {
            if let LogicalPlan::SubqueryAlias(outer) = &plan
                && let LogicalPlan::SubqueryAlias(inner) = outer.input.as_ref()
            {
                return Ok(Transformed::yes(LogicalPlan::SubqueryAlias(
                    SubqueryAlias::try_new(inner.input.clone(), outer.alias.clone())?,
                )));
            }
            Ok(Transformed::no(plan))
        })?;
        return Ok(LogicalPlan::Extension(Extension {
            node: Arc::new(FederatedPlanNode::new(rewritten.data, node.planner.clone())),
        }));
    }
    Ok(plan)
}

fn supported(plan: &LogicalPlan) -> Result<bool> {
    let mut supported = true;
    plan.apply(|node| {
        let operator_ok = match node {
            LogicalPlan::TableScan(_)
            | LogicalPlan::Projection(_)
            | LogicalPlan::Filter(_)
            | LogicalPlan::Sort(_)
            | LogicalPlan::Limit(_)
            | LogicalPlan::Aggregate(_) => true,
            LogicalPlan::SubqueryAlias(alias) => !alias.alias.table().contains('\\'),
            LogicalPlan::Join(join) => {
                matches!(
                    join.join_type,
                    JoinType::Inner | JoinType::Left | JoinType::Right | JoinType::Full
                ) && !join.on.is_empty()
                    && join.null_equality == datafusion::common::NullEquality::NullEqualsNothing
            }
            _ => false,
        };
        if !operator_ok {
            supported = false;
            return Ok(TreeNodeRecursion::Stop);
        }
        for expr in node.expressions() {
            expr.apply(|expr| {
                let expression_ok = match expr {
                    Expr::Literal(value, _) => match value {
                        datafusion::common::ScalarValue::Utf8(Some(value))
                        | datafusion::common::ScalarValue::Utf8View(Some(value))
                        | datafusion::common::ScalarValue::LargeUtf8(Some(value)) => {
                            !value.contains('\\')
                        }
                        _ => true,
                    },
                    Expr::Alias(alias) => !alias.name.contains('\\'),
                    Expr::Column(_) | Expr::IsNull(_) | Expr::IsNotNull(_) | Expr::Not(_) => true,
                    Expr::BinaryExpr(binary) => matches!(
                        binary.op,
                        Operator::Eq
                            | Operator::NotEq
                            | Operator::Lt
                            | Operator::LtEq
                            | Operator::Gt
                            | Operator::GtEq
                            | Operator::And
                            | Operator::Or
                    ),
                    Expr::AggregateFunction(aggregate) => {
                        // Name alone is insufficient: a user UDAF may shadow a builtin.
                        let implementation = aggregate.func.inner().as_ref() as &dyn Any;
                        (implementation.is::<Count>()
                            || implementation.is::<Sum>()
                            || implementation.is::<Avg>()
                            || implementation.is::<Min>()
                            || implementation.is::<Max>())
                            && !aggregate.params.distinct
                            && aggregate.params.filter.is_none()
                            && aggregate.params.order_by.is_empty()
                            && aggregate.params.null_treatment.is_none()
                    }
                    _ => false,
                };
                if !expression_ok {
                    supported = false;
                    return Ok(TreeNodeRecursion::Stop);
                }
                Ok(TreeNodeRecursion::Continue)
            })?;
        }
        Ok(if supported {
            TreeNodeRecursion::Continue
        } else {
            TreeNodeRecursion::Stop
        })
    })?;
    Ok(supported)
}

/// ClickHouse's plain sum/min/max/avg have different empty-set defaults.
/// Change only those generated aggregate calls, retaining count's zero result.
pub(super) fn null_on_empty(mut statement: ast::Statement) -> Result<ast::Statement> {
    let _ = ast::visit_expressions_mut(&mut statement, |expr| {
        if let ast::Expr::Function(function) = expr {
            let replacement = match function.name.to_string().to_lowercase().as_str() {
                "sum" => Some("sumOrNull"),
                "avg" => Some("avgOrNull"),
                "min" => Some("minOrNull"),
                "max" => Some("maxOrNull"),
                _ => None,
            };
            if let Some(name) = replacement {
                function.name =
                    ast::ObjectName(vec![ast::ObjectNamePart::Identifier(ast::Ident::new(name))]);
            }
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    Ok(statement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::{
        arrow::datatypes::{DataType, Field, Schema},
        catalog::empty::EmptyTable,
        prelude::SessionContext,
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn functions_casts_and_distinct_remain_local() {
        let context = SessionContext::new();
        context
            .register_table(
                "samples",
                Arc::new(EmptyTable::new(Arc::new(Schema::new(vec![Field::new(
                    "depth",
                    DataType::Float64,
                    true,
                )])))),
            )
            .unwrap();
        for sql in [
            "SELECT sqrt(depth) FROM samples",
            "SELECT CAST(depth AS VARCHAR) FROM samples",
            "SELECT COUNT(DISTINCT depth) FROM samples",
        ] {
            let frame = context.sql(sql).await.unwrap();
            assert!(!supported(frame.logical_plan()).unwrap(), "{sql}");
        }
        let frame = context
            .sql("SELECT SUM(depth), COUNT(*) FROM samples WHERE depth > 1.0")
            .await
            .unwrap();
        assert!(supported(frame.logical_plan()).unwrap());
    }

    #[test]
    fn null_aggregate_rewrite_preserves_count_and_literals() {
        use datafusion::sql::sqlparser::{dialect::GenericDialect, parser::Parser};
        let statement = Parser::parse_sql(
            &GenericDialect {},
            "SELECT sum(x), avg(x), min(x), max(x), count(*), 'sum(x)' FROM t",
        )
        .unwrap()
        .remove(0);
        let rewritten = null_on_empty(statement).unwrap().to_string();
        assert_eq!(
            rewritten,
            "SELECT sumOrNull(x), avgOrNull(x), minOrNull(x), maxOrNull(x), count(*), 'sum(x)' FROM t"
        );
    }
}
