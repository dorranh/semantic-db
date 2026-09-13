use datafusion::functions_aggregate::{
    average::Avg,
    count::Count,
    min_max::{Max, Min},
    sum::Sum,
};
use datafusion::{
    common::tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    error::Result,
    logical_expr::{Expr, Extension, JoinType, LogicalPlan, Operator},
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
pub(super) fn restrict_federation(
    plan: LogicalPlan,
    planner: Arc<dyn datafusion_federation::FederationPlanner>,
) -> Result<LogicalPlan> {
    if let LogicalPlan::Extension(extension) = &plan
        && let Some(node) = extension.node.as_any().downcast_ref::<FederatedPlanNode>()
    {
        return split(node.plan().clone(), &planner);
    }
    Ok(plan)
}
fn split(
    plan: LogicalPlan,
    planner: &Arc<dyn datafusion_federation::FederationPlanner>,
) -> Result<LogicalPlan> {
    if supported(&plan)? {
        return Ok(LogicalPlan::Extension(Extension {
            node: Arc::new(FederatedPlanNode::new(plan, planner.clone())),
        }));
    }
    let expressions = plan.expressions();
    let inputs = plan
        .inputs()
        .into_iter()
        .map(|p| split(p.clone(), planner))
        .collect::<Result<Vec<_>>>()?;
    plan.with_new_exprs(expressions, inputs)
}

pub(super) fn sql_for_plan(plan: &LogicalPlan) -> Result<String> {
    let sql = render(plan)?;
    let schema = plan.schema();
    let projection = if schema.fields().is_empty() {
        "1 AS __semantic_row".into()
    } else {
        schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, f)| format!("__result.__c{i} AS {}", crate::quote_identifier(f.name())))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Ok(format!("SELECT {projection} FROM ({sql}) AS __result"))
}
fn columns(count: usize, alias: &str, offset: usize) -> Vec<String> {
    (0..count)
        .map(|i| format!("{alias}.__c{i} AS __c{}", i + offset))
        .collect()
}
fn expression(expr: Expr, inputs: &[(&LogicalPlan, &str)]) -> Result<String> {
    let rewritten = expr
        .transform_up(|expr| {
            if let Expr::Column(column) = &expr {
                for (input, alias) in inputs {
                    if let Ok(index) = input.schema().index_of_column(column) {
                        return Ok(Transformed::yes(Expr::Column(
                            datafusion::common::Column::new(Some(*alias), format!("__c{index}")),
                        )));
                    }
                }
                return Err(crate::error("cannot resolve remote expression column"));
            }
            if let Expr::Alias(alias) = expr {
                return Ok(Transformed::yes(*alias.expr));
            }
            Ok(Transformed::no(expr))
        })?
        .data;
    let expr =
        datafusion::sql::unparser::Unparser::new(&ClickHouseDialect).expr_to_sql(&rewritten)?;
    rewrite_expression(expr)
}
fn rewrite_expression(mut expr: ast::Expr) -> Result<String> {
    let _ = ast::visit_expressions_mut(&mut expr, rewrite_function);
    Ok(expr.to_string())
}
fn render(plan: &LogicalPlan) -> Result<String> {
    use datafusion_federation::{get_table_source, sql::SQLTableSource};
    match plan {
        LogicalPlan::TableScan(scan) => {
            let source = get_table_source(&scan.source)?
                .ok_or_else(|| crate::error("remote scan has no federation source"))?;
            let source = (source.as_ref() as &dyn Any)
                .downcast_ref::<SQLTableSource>()
                .ok_or_else(|| crate::error("incompatible remote source"))?;
            let indices = scan
                .projection
                .clone()
                .unwrap_or_else(|| (0..scan.source.schema().fields().len()).collect());
            let projection = if indices.is_empty() {
                "1 AS __semantic_row".into()
            } else {
                indices
                    .iter()
                    .enumerate()
                    .map(|(i, index)| {
                        format!(
                            "{} AS __c{i}",
                            crate::quote_identifier(scan.source.schema().field(*index).name())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let final_read = source
                .table
                .as_any()
                .downcast_ref::<crate::BoundTable>()
                .is_some_and(|bound| bound.options.final_read);
            let mut sql = format!(
                "SELECT {projection} FROM {}{}",
                crate::remote_name(&source.table_reference()),
                if final_read { " FINAL" } else { "" }
            );
            if !scan.filters.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(
                    &scan
                        .filters
                        .iter()
                        .map(|e| filter_sql(e).map(|s| format!("({s})")))
                        .collect::<Result<Vec<_>>>()?
                        .join(" AND "),
                );
            }
            if let Some(fetch) = scan.fetch {
                sql.push_str(&format!(" LIMIT {fetch}"));
            }
            Ok(sql)
        }
        LogicalPlan::SubqueryAlias(alias) => render(&alias.input),
        LogicalPlan::Projection(projection) => {
            let expressions = projection
                .expr
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    expression(e.clone(), &[(&projection.input, "q")])
                        .map(|s| format!("{s} AS __c{i}"))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(format!(
                "SELECT {} FROM ({}) AS q",
                if expressions.is_empty() {
                    "1 AS __semantic_row".into()
                } else {
                    expressions.join(", ")
                },
                render(&projection.input)?
            ))
        }
        LogicalPlan::Filter(filter) => Ok(format!(
            "SELECT * FROM ({}) AS q WHERE {}",
            render(&filter.input)?,
            expression(filter.predicate.clone(), &[(&filter.input, "q")])?
        )),
        LogicalPlan::Aggregate(aggregate) => {
            let input = [(aggregate.input.as_ref(), "q")];
            let groups = aggregate
                .group_expr
                .iter()
                .map(|e| expression(e.clone(), &input))
                .collect::<Result<Vec<_>>>()?;
            let aggregates = aggregate
                .aggr_expr
                .iter()
                .map(|e| expression(e.clone(), &input))
                .collect::<Result<Vec<_>>>()?;
            let select = groups
                .iter()
                .chain(&aggregates)
                .enumerate()
                .map(|(i, s)| format!("{s} AS __c{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "SELECT {select} FROM ({}) AS q{}",
                render(&aggregate.input)?,
                if groups.is_empty() {
                    String::new()
                } else {
                    format!(" GROUP BY {}", groups.join(", "))
                }
            ))
        }
        LogicalPlan::Sort(sort) => {
            let order = sort
                .expr
                .iter()
                .map(|e| {
                    expression(e.expr.clone(), &[(&sort.input, "q")]).map(|s| {
                        format!(
                            "{s} {} NULLS {}",
                            if e.asc { "ASC" } else { "DESC" },
                            if e.nulls_first { "FIRST" } else { "LAST" }
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            Ok(format!(
                "SELECT * FROM ({}) AS q ORDER BY {order}{}",
                render(&sort.input)?,
                sort.fetch
                    .map(|n| format!(" LIMIT {n}"))
                    .unwrap_or_default()
            ))
        }
        LogicalPlan::Limit(limit) => {
            let fetch = limit
                .fetch
                .as_ref()
                .map(|e| expression(*e.clone(), &[]))
                .transpose()?
                .unwrap_or_else(|| "18446744073709551615".into());
            let skip = limit
                .skip
                .as_ref()
                .map(|e| expression(*e.clone(), &[]))
                .transpose()?
                .unwrap_or_else(|| "0".into());
            Ok(format!(
                "SELECT * FROM ({}) AS q LIMIT {fetch} OFFSET {skip}",
                render(&limit.input)?
            ))
        }
        LogicalPlan::Join(join) => {
            let inputs = [(join.left.as_ref(), "l"), (join.right.as_ref(), "r")];
            let mut conditions = join
                .on
                .iter()
                .map(|(l, r)| {
                    Ok(format!(
                        "{} = {}",
                        expression(l.clone(), &inputs)?,
                        expression(r.clone(), &inputs)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            if let Some(filter) = &join.filter {
                conditions.push(expression(filter.clone(), &inputs)?);
            }
            let left = join.left.schema().fields().len();
            let right = join.right.schema().fields().len();
            let (kind, select) = match join.join_type {
                JoinType::Inner => (
                    "ALL INNER",
                    columns(left, "l", 0)
                        .into_iter()
                        .chain(columns(right, "r", left))
                        .collect::<Vec<_>>(),
                ),
                JoinType::Left => (
                    "ALL LEFT",
                    columns(left, "l", 0)
                        .into_iter()
                        .chain(columns(right, "r", left))
                        .collect(),
                ),
                JoinType::Right => (
                    "ALL RIGHT",
                    columns(left, "l", 0)
                        .into_iter()
                        .chain(columns(right, "r", left))
                        .collect(),
                ),
                JoinType::Full => (
                    "ALL FULL",
                    columns(left, "l", 0)
                        .into_iter()
                        .chain(columns(right, "r", left))
                        .collect(),
                ),
                JoinType::LeftSemi => ("LEFT SEMI", columns(left, "l", 0)),
                JoinType::LeftAnti => ("LEFT ANTI", columns(left, "l", 0)),
                JoinType::RightSemi => ("RIGHT SEMI", columns(right, "r", 0)),
                JoinType::RightAnti => ("RIGHT ANTI", columns(right, "r", 0)),
                _ => return Err(crate::error("unqualified remote join")),
            };
            Ok(format!(
                "SELECT {} FROM ({}) AS l {kind} JOIN ({}) AS r ON {}",
                select.join(", "),
                render(&join.left)?,
                render(&join.right)?,
                conditions.join(" AND ")
            ))
        }
        LogicalPlan::Union(union) => Ok(union
            .inputs
            .iter()
            .map(|input| render(input).map(|sql| format!("({sql})")))
            .collect::<Result<Vec<_>>>()?
            .join(" UNION ALL ")),
        LogicalPlan::Window(window) => {
            let mut select = columns(window.input.schema().fields().len(), "q", 0);
            for expr in &window.window_expr {
                select.push(format!(
                    "{} AS __c{}",
                    expression(expr.clone(), &[(&window.input, "q")])?,
                    select.len()
                ));
            }
            Ok(format!(
                "SELECT {} FROM ({}) AS q",
                select.join(", "),
                render(&window.input)?
            ))
        }
        LogicalPlan::EmptyRelation(empty) => Ok(format!(
            "SELECT 1 AS __semantic_row{}",
            if empty.produce_one_row {
                ""
            } else {
                " WHERE false"
            }
        )),
        _ => Err(crate::error("unqualified remote operator")),
    }
}
pub(super) fn unqualify(expr: Expr) -> Result<Expr> {
    Ok(expr
        .transform_up(|expr| match expr {
            Expr::Column(mut c) => {
                c.relation = None;
                Ok(Transformed::yes(Expr::Column(c)))
            }
            other => Ok(Transformed::no(other)),
        })?
        .data)
}
pub(super) fn filter_sql(expr: &Expr) -> Result<String> {
    rewrite_expression(
        datafusion::sql::unparser::Unparser::new(&ClickHouseDialect)
            .expr_to_sql(&unqualify(expr.clone())?)?,
    )
}
pub(super) fn supports_filter(
    expr: &Expr,
    schema: &datafusion::arrow::datatypes::SchemaRef,
) -> bool {
    use datafusion::{
        catalog::empty::EmptyTable, datasource::provider_as_source,
        logical_expr::LogicalPlanBuilder,
    };
    let plan = (|| -> Result<LogicalPlan> {
        LogicalPlanBuilder::scan(
            "filter_input",
            provider_as_source(Arc::new(EmptyTable::new(schema.clone()))),
            None,
        )?
        .filter(unqualify(expr.clone())?)?
        .build()
    })();
    plan.is_ok_and(|plan| supported(&plan).unwrap_or(false)) && filter_sql(expr).is_ok()
}

fn supported(plan: &LogicalPlan) -> Result<bool> {
    let mut supported = true;
    plan.apply(|node| {
        let operator_ok = match node {
            LogicalPlan::TableScan(_)
            | LogicalPlan::EmptyRelation(_)
            | LogicalPlan::Projection(_)
            | LogicalPlan::Filter(_)
            | LogicalPlan::Limit(_)
            | LogicalPlan::Union(_)
            | LogicalPlan::Window(_) => true,
            LogicalPlan::Sort(sort) => sort.expr.iter().all(|sort| !float_expr(&sort.expr, node)),
            LogicalPlan::Aggregate(aggregate) => aggregate
                .group_expr
                .iter()
                .all(|expr| !float_expr(expr, node)),
            LogicalPlan::SubqueryAlias(alias) => !alias.alias.table().contains('\\'),
            LogicalPlan::Join(join) => {
                matches!(
                    join.join_type,
                    JoinType::Inner
                        | JoinType::Left
                        | JoinType::Right
                        | JoinType::Full
                        | JoinType::LeftSemi
                        | JoinType::LeftAnti
                        | JoinType::RightSemi
                        | JoinType::RightAnti
                ) && !join.on.is_empty()
                    && join
                        .on
                        .iter()
                        .all(|(left, right)| !float_expr(left, node) && !float_expr(right, node))
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
                    Expr::Case(case) => case
                        .expr
                        .as_ref()
                        .is_none_or(|expr| !float_expr(expr, node)),
                    Expr::Between(between) => !float_expr(&between.expr, node),
                    Expr::Like(like) => !like.case_insensitive && like.escape_char.is_none(),
                    Expr::InList(list) => {
                        use datafusion::logical_expr::ExprSchemable;
                        !float_expr(&list.expr, node)
                            && node.inputs().first().is_some_and(|input| {
                                list.expr
                                    .nullable(input.schema())
                                    .is_ok_and(|nullable| !nullable)
                            })
                            && list.list.iter().all(
                                |expr| matches!(expr,Expr::Literal(value,_) if !value.is_null()),
                            )
                    }
                    Expr::Cast(cast) => expression_type(&cast.expr, node)
                        .is_some_and(|source| widening_cast(&source, cast.field.data_type())),
                    Expr::ScalarFunction(function) => qualified_scalar(function, node),
                    Expr::WindowFunction(window) => {
                        if let datafusion::logical_expr::WindowFunctionDefinition::WindowUDF(
                            function,
                        ) = &window.fun
                        {
                            let implementation = function.inner().as_ref() as &dyn Any;
                            (implementation
                                .is::<datafusion::functions_window::row_number::RowNumber>()
                                || (implementation
                                    .is::<datafusion::functions_window::rank::Rank>()
                                    && matches!(function.name(), "rank" | "dense_rank")))
                                && window
                                    .params
                                    .partition_by
                                    .iter()
                                    .all(|expr| !float_expr(expr, node))
                                && window
                                    .params
                                    .order_by
                                    .iter()
                                    .all(|sort| !float_expr(&sort.expr, node))
                        } else {
                            false
                        }
                    }
                    Expr::BinaryExpr(binary) => {
                        (matches!(
                            binary.op,
                            Operator::Plus
                                | Operator::Minus
                                | Operator::Multiply
                                | Operator::Divide
                        ) && expression_type(expr, node)
                            == Some(datafusion::arrow::datatypes::DataType::Float64))
                            || matches!(binary.op, Operator::And | Operator::Or)
                            || (!float_expr(&binary.left, node)
                                && !float_expr(&binary.right, node)
                                && matches!(
                                    binary.op,
                                    Operator::Eq
                                        | Operator::NotEq
                                        | Operator::Lt
                                        | Operator::LtEq
                                        | Operator::Gt
                                        | Operator::GtEq
                                ))
                    }
                    Expr::AggregateFunction(aggregate) => {
                        // Name alone is insufficient: a user UDAF may shadow a builtin.
                        let implementation = aggregate.func.inner().as_ref() as &dyn Any;
                        (implementation.is::<Count>()
                            || implementation.is::<Sum>()
                            || implementation.is::<Avg>()
                            || implementation.is::<Min>()
                            || implementation.is::<Max>())
                            && (!aggregate.params.distinct
                                || implementation.is::<Count>() && aggregate.params.args.len() == 1)
                            && (!(implementation.is::<Min>()
                                || implementation.is::<Max>()
                                || aggregate.params.distinct)
                                || aggregate
                                    .params
                                    .args
                                    .iter()
                                    .all(|arg| !float_expr(arg, node)))
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

// DataFusion gives NaNs a total ordering; ClickHouse comparisons and extrema do not.
// Without non-finite statistics, keep operations whose result depends on that ordering local.
fn float_expr(expr: &Expr, plan: &LogicalPlan) -> bool {
    matches!(
        expression_type(expr, plan),
        Some(
            datafusion::arrow::datatypes::DataType::Float16
                | datafusion::arrow::datatypes::DataType::Float32
                | datafusion::arrow::datatypes::DataType::Float64
        )
    )
}

fn expression_type(
    expr: &Expr,
    plan: &LogicalPlan,
) -> Option<datafusion::arrow::datatypes::DataType> {
    use datafusion::logical_expr::ExprSchemable;
    plan.inputs()
        .into_iter()
        .find_map(|input| expr.get_type(input.schema()).ok())
        .or_else(|| expr.get_type(plan.schema()).ok())
}
fn widening_cast(
    source: &datafusion::arrow::datatypes::DataType,
    target: &datafusion::arrow::datatypes::DataType,
) -> bool {
    use datafusion::arrow::datatypes::DataType::*;
    source == target
        || matches!(
            (source, target),
            (Int8, Int16 | Int32 | Int64)
                | (Int16, Int32 | Int64)
                | (Int32, Int64)
                | (UInt8, UInt16 | UInt32 | UInt64 | Int16 | Int32 | Int64)
                | (UInt16, UInt32 | UInt64 | Int32 | Int64)
                | (UInt32, UInt64 | Int64)
                | (Float32, Float64)
        )
}
fn qualified_scalar(
    function: &datafusion::logical_expr::expr::ScalarFunction,
    plan: &LogicalPlan,
) -> bool {
    use datafusion::functions::{
        core::{coalesce::CoalesceFunc, nullif::NullIfFunc},
        string::{lower::LowerFunc, upper::UpperFunc},
    };
    let implementation = function.func.inner().as_ref() as &dyn Any;
    if implementation.is::<datafusion::functions::datetime::date_part::DatePartFunc>() {
        return function.args.len() == 2
            && matches!(&function.args[0],Expr::Literal(datafusion::common::ScalarValue::Utf8(Some(unit)),_) if matches!(unit.to_ascii_lowercase().as_str(),"year"|"month"|"day"|"hour"|"minute"))
            && matches!(&function.args[1], Expr::Column(_))
            && matches!(expression_type(&function.args[1],plan),Some(datafusion::arrow::datatypes::DataType::Timestamp(_,tz)) if tz.as_deref().is_none_or(|tz|tz=="UTC"));
    }
    implementation.is::<CoalesceFunc>()
        || (implementation.is::<NullIfFunc>()
            && function.args.iter().all(|arg| !float_expr(arg, plan)))
        || implementation.is::<LowerFunc>()
        || implementation.is::<UpperFunc>()
}

/// ClickHouse's plain sum/min/max/avg have different empty-set defaults.
/// Change only those generated aggregate calls, retaining count's zero result.
pub(super) fn null_on_empty(mut statement: ast::Statement) -> Result<ast::Statement> {
    let _ = ast::visit_expressions_mut(&mut statement, rewrite_function);
    Ok(statement)
}
fn rewrite_function(expr: &mut ast::Expr) -> std::ops::ControlFlow<()> {
    if let ast::Expr::Function(function) = expr {
        if function.name.to_string().eq_ignore_ascii_case("date_part")
            && let ast::FunctionArguments::List(list) = &mut function.args
        {
            let unit = list
                .args
                .first()
                .map(ToString::to_string)
                .unwrap_or_default()
                .trim_matches('\'')
                .to_ascii_lowercase();
            let name = match unit.as_str() {
                "year" => Some("toYear"),
                "month" => Some("toMonth"),
                "day" => Some("toDayOfMonth"),
                "hour" => Some("toHour"),
                "minute" => Some("toMinute"),
                _ => None,
            };
            if let Some(name) = name {
                list.args.remove(0);
                function.name =
                    ast::ObjectName(vec![ast::ObjectNamePart::Identifier(ast::Ident::new(name))]);
            }
        }
        let distinct = matches!(&function.args,ast::FunctionArguments::List(list) if list.duplicate_treatment == Some(ast::DuplicateTreatment::Distinct));
        let replacement = match function.name.to_string().to_lowercase().as_str() {
            "count" if distinct => Some("uniqExact"),
            "lower" => Some("lowerUTF8"),
            "upper" => Some("upperUTF8"),
            "sum" => Some("sumOrNull"),
            "avg" => Some("avgOrNull"),
            "min" => Some("minOrNull"),
            "max" => Some("maxOrNull"),
            _ => None,
        };
        if let Some(name) = replacement {
            function.name =
                ast::ObjectName(vec![ast::ObjectNamePart::Identifier(ast::Ident::new(name))]);
            if name == "uniqExact"
                && let ast::FunctionArguments::List(list) = &mut function.args
            {
                list.duplicate_treatment = None;
            }
        }
        if let Some(filter) = function.filter.take()
            && let ast::FunctionArguments::List(list) = &mut function.args
        {
            // ANSI FILTER counts only TRUE, excluding NULL just like WHERE.
            let condition = ast::Expr::BinaryOp {
                left: Box::new(ast::Expr::IsNotNull(filter.clone())),
                op: ast::BinaryOperator::And,
                right: filter,
            };
            if function.name.to_string().eq_ignore_ascii_case("count") {
                let condition =
                    if let Some(ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(value))) =
                        list.args.first()
                    {
                        ast::Expr::BinaryOp {
                            left: Box::new(ast::Expr::IsNotNull(Box::new(value.clone()))),
                            op: ast::BinaryOperator::And,
                            right: Box::new(condition),
                        }
                    } else {
                        condition
                    };
                list.args = vec![ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(
                    condition,
                ))];
            } else {
                list.args
                    .push(ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(
                        condition,
                    )));
            }
            let name = format!("{}If", function.name);
            function.name =
                ast::ObjectName(vec![ast::ObjectNamePart::Identifier(ast::Ident::new(name))]);
        }
    }
    std::ops::ControlFlow::<()>::Continue(())
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
            "SELECT depth FROM samples WHERE depth > 1.0",
            "SELECT depth FROM samples ORDER BY depth",
            "SELECT MIN(depth), MAX(depth) FROM samples",
            "SELECT COUNT(DISTINCT depth) FROM samples",
            "SELECT CAST(depth AS VARCHAR) FROM samples",
        ] {
            let frame = context.sql(sql).await.unwrap();
            assert!(!supported(frame.logical_plan()).unwrap(), "{sql}");
        }
        let frame = context
            .sql("SELECT SUM(depth), COUNT(*) FROM samples")
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
