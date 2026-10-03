use crate::*;
use datafusion::{
    common::{
        ScalarValue, TableReference,
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    },
    logical_expr::{ExprSchemable, Extension, JoinType, LogicalPlan, Operator},
    sql::unparser::dialect::Dialect,
};
use datafusion_federation::{
    FederatedPlanNode, FederatedTableProviderAdaptor, FederationPlanner,
    sql::{LogicalOptimizer, SQLExecutor, SQLFederationProvider, SQLTableSource},
};
use std::any::Any;
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
struct PgDialect;
impl Dialect for PgDialect {
    fn identifier_quote_style(&self, _: &str) -> Option<char> {
        Some('"')
    }
    fn supports_empty_select_list(&self) -> bool {
        false
    }
}
#[derive(Debug)]
struct BoundTable {
    reference: TableReference,
    target: String,
    schema: SchemaRef,
    revision: String,
}
impl datafusion_federation::sql::SQLTable for BoundTable {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn table_reference(&self) -> TableReference {
        self.reference.clone()
    }
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
}
pub(crate) fn provider(
    pg: Postgres,
    namespace: &str,
    table: &str,
    fallback: Arc<PgTable>,
) -> Result<Arc<dyn TableProvider>> {
    let executor = Arc::new(Executor(pg));
    let provider = Arc::new(SQLFederationProvider::new(executor));
    let source = Arc::new(SQLTableSource::new_with_table(
        provider,
        Arc::new(BoundTable {
            reference: TableReference::partial(namespace.to_owned(), table.to_owned()),
            target: fallback.target.clone(),
            schema: fallback.schema.clone(),
            revision: fallback.metadata.revision.clone(),
        }),
    ));
    Ok(Arc::new(FederatedTableProviderAdaptor::new_with_provider(
        source, fallback,
    )))
}
struct Executor(Postgres);
#[async_trait]
impl SQLExecutor for Executor {
    fn name(&self) -> &str {
        "postgres"
    }
    fn compute_context(&self) -> Option<String> {
        Some(self.0.domain.clone())
    }
    fn dialect(&self) -> Arc<dyn Dialect> {
        Arc::new(PgDialect)
    }
    fn logical_optimizer(&self) -> Option<LogicalOptimizer> {
        let planner = Arc::new(RemotePlanner(self.0.clone()));
        Some(Box::new(move |plan| {
            restrict_federation(plan, planner.clone())
        }))
    }
    fn execute(
        &self,
        _: &str,
        _: SchemaRef,
        _: &[Arc<dyn datafusion::physical_expr::PhysicalExpr>],
    ) -> Result<SendableRecordBatchStream> {
        Err(failure(
            "Postgres federation requires execution TaskContext",
        ))
    }
    async fn table_names(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn get_table_schema(&self, _: &str) -> Result<SchemaRef> {
        Err(failure("Postgres requires an explicitly bound table"))
    }
}
#[derive(Debug)]
struct RemotePlanner(Postgres);
#[async_trait]
impl FederationPlanner for RemotePlanner {
    async fn plan_federation(
        &self,
        node: &FederatedPlanNode,
        _: &dyn Session,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let mut parameters = predicate::Parameters::default();
        let sql = sql_for_plan(node.plan(), &mut parameters)?;
        let mut revisions = std::collections::BTreeMap::new();
        node.plan().apply(|plan| {
            if let LogicalPlan::TableScan(scan) = plan {
                let source = datafusion_federation::get_table_source(&scan.source)?
                    .ok_or_else(|| failure("missing Postgres source"))?;
                let source = (source.as_ref() as &dyn Any)
                    .downcast_ref::<SQLTableSource>()
                    .ok_or_else(|| failure("invalid Postgres source"))?;
                let bound = source
                    .table
                    .as_any()
                    .downcast_ref::<BoundTable>()
                    .ok_or_else(|| failure("invalid Postgres binding"))?;
                revisions.insert(bound.target.clone(), bound.revision.clone());
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        Ok(Arc::new(execution::RemoteExec::new(
            self.0.clone(),
            sql,
            Arc::new(node.plan().schema().as_arrow().clone()),
            parameters,
            revisions.into_iter().collect(),
            true,
        )))
    }
}
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

pub(super) fn sql_for_plan(
    plan: &LogicalPlan,
    parameters: &mut predicate::Parameters,
) -> Result<String> {
    let sql = render(plan, parameters)?;
    let schema = plan.schema();
    let projection = if schema.fields().is_empty() {
        "1 AS __semantic_row".into()
    } else {
        schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, f)| format!("__result.__c{i} AS {}", quote(f.name())))
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
fn expression(
    expr: Expr,
    inputs: &[(&LogicalPlan, &str)],
    parameters: &mut predicate::Parameters,
) -> Result<String> {
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
                return Err(failure("cannot resolve remote expression column"));
            }
            if let Expr::Literal(value, _) = &expr {
                let value = match value {
                    ScalarValue::UInt8(v) => ScalarValue::Int64(v.map(i64::from)),
                    other => other.clone(),
                };
                let id = parameters.push(&value)?;
                return Ok(Transformed::yes(Expr::Placeholder(
                    datafusion::logical_expr::expr::Placeholder::new_with_field(id, None),
                )));
            }
            if let Expr::Alias(alias) = expr {
                return Ok(Transformed::yes(*alias.expr));
            }
            Ok(Transformed::no(expr))
        })?
        .data;
    Ok(datafusion::sql::unparser::Unparser::new(&PgDialect)
        .expr_to_sql(&rewritten)?
        .to_string())
}
// DataFusion 55 integer SUM uses wrapping addition. Sum in PostgreSQL NUMERIC
// first (including int2/int4 inputs), then normalize modulo 2^64. This preserves
// null/empty-set behavior and gives every parent the planned signed-64 value.
fn aggregate_expression(
    expr: &Expr,
    inputs: &[(&LogicalPlan, &str)],
    parameters: &mut predicate::Parameters,
) -> Result<String> {
    if let Expr::Alias(alias) = expr {
        return aggregate_expression(&alias.expr, inputs, parameters);
    }
    if let Expr::AggregateFunction(a) = expr
        && (a.func.inner().as_ref() as &dyn Any).is::<datafusion::functions_aggregate::sum::Sum>()
    {
        let arg = expression(a.params.args[0].clone(), inputs, parameters)?;
        let filter = a
            .params
            .filter
            .as_ref()
            .map(|e| {
                expression(*e.clone(), inputs, parameters).map(|s| format!(" FILTER (WHERE {s})"))
            })
            .transpose()?
            .unwrap_or_default();
        return Ok(format!(
            "(mod(mod(SUM(CAST({arg} AS NUMERIC)){filter} + 9223372036854775808, 18446744073709551616) + 18446744073709551616, 18446744073709551616) - 9223372036854775808)"
        ));
    }
    expression(expr.clone(), inputs, parameters)
}
fn render(plan: &LogicalPlan, parameters: &mut predicate::Parameters) -> Result<String> {
    use datafusion_federation::{get_table_source, sql::SQLTableSource};
    match plan {
        LogicalPlan::TableScan(scan) => {
            let source = get_table_source(&scan.source)?
                .ok_or_else(|| failure("remote scan has no federation source"))?;
            let source = (source.as_ref() as &dyn Any)
                .downcast_ref::<SQLTableSource>()
                .ok_or_else(|| failure("incompatible remote source"))?;
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
                            quote(scan.source.schema().field(*index).name())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let bound = source
                .table
                .as_any()
                .downcast_ref::<BoundTable>()
                .ok_or_else(|| failure("incompatible Postgres table identity"))?;
            let mut sql = format!("SELECT {projection} FROM {}", bound.target);
            if !scan.filters.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(
                    &scan
                        .filters
                        .iter()
                        .map(|e| {
                            predicate::translate(e, &scan.source.schema(), 4096, parameters)
                                .ok_or_else(|| failure("unqualified remote filter"))
                        })
                        .collect::<Result<Vec<_>>>()?
                        .join(" AND "),
                );
            }
            if let Some(fetch) = scan.fetch {
                sql.push_str(&format!(" LIMIT {fetch}"));
            }
            Ok(sql)
        }
        LogicalPlan::SubqueryAlias(alias) => render(&alias.input, parameters),
        LogicalPlan::Projection(projection) => {
            let expressions = projection
                .expr
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    expression(e.clone(), &[(&projection.input, "q")], parameters)
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
                render(&projection.input, parameters)?
            ))
        }
        LogicalPlan::Filter(filter) => Ok(format!(
            "SELECT * FROM ({}) AS q WHERE {}",
            render(&filter.input, parameters)?,
            expression(
                filter.predicate.clone(),
                &[(&filter.input, "q")],
                parameters
            )?
        )),
        LogicalPlan::Aggregate(aggregate) => {
            let input = [(aggregate.input.as_ref(), "q")];
            let groups = aggregate
                .group_expr
                .iter()
                .map(|e| expression(e.clone(), &input, parameters))
                .collect::<Result<Vec<_>>>()?;
            let aggregates = aggregate
                .aggr_expr
                .iter()
                .map(|e| aggregate_expression(e, &input, parameters))
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
                render(&aggregate.input, parameters)?,
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
                    expression(e.expr.clone(), &[(&sort.input, "q")], parameters).map(|s| {
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
                render(&sort.input, parameters)?,
                sort.fetch
                    .map(|n| format!(" LIMIT {n}"))
                    .unwrap_or_default()
            ))
        }
        LogicalPlan::Limit(limit) => {
            let fetch = limit
                .fetch
                .as_ref()
                .map(|e| expression(*e.clone(), &[], parameters))
                .transpose()?
                .unwrap_or_else(|| "ALL".into());
            let skip = limit
                .skip
                .as_ref()
                .map(|e| expression(*e.clone(), &[], parameters))
                .transpose()?
                .unwrap_or_else(|| "0".into());
            Ok(format!(
                "SELECT * FROM ({}) AS q LIMIT {fetch} OFFSET {skip}",
                render(&limit.input, parameters)?
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
                        expression(l.clone(), &inputs, parameters)?,
                        expression(r.clone(), &inputs, parameters)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            if let Some(filter) = &join.filter {
                conditions.push(expression(filter.clone(), &inputs, parameters)?);
            }
            let left = join.left.schema().fields().len();
            let right = join.right.schema().fields().len();
            let (kind, select) = match join.join_type {
                JoinType::Inner => (
                    "INNER",
                    columns(left, "l", 0)
                        .into_iter()
                        .chain(columns(right, "r", left))
                        .collect::<Vec<_>>(),
                ),
                JoinType::Left => (
                    "LEFT",
                    columns(left, "l", 0)
                        .into_iter()
                        .chain(columns(right, "r", left))
                        .collect(),
                ),
                JoinType::Right => (
                    "RIGHT",
                    columns(left, "l", 0)
                        .into_iter()
                        .chain(columns(right, "r", left))
                        .collect(),
                ),
                JoinType::Full => (
                    "FULL",
                    columns(left, "l", 0)
                        .into_iter()
                        .chain(columns(right, "r", left))
                        .collect(),
                ),
                JoinType::LeftSemi => ("LEFT SEMI", columns(left, "l", 0)),
                JoinType::LeftAnti => ("LEFT ANTI", columns(left, "l", 0)),
                JoinType::RightSemi => ("RIGHT SEMI", columns(right, "r", 0)),
                JoinType::RightAnti => ("RIGHT ANTI", columns(right, "r", 0)),
                _ => return Err(failure("unqualified remote join")),
            };
            if matches!(
                join.join_type,
                JoinType::LeftSemi | JoinType::LeftAnti | JoinType::RightSemi | JoinType::RightAnti
            ) {
                let (outer, inner, oa, ia) =
                    if matches!(join.join_type, JoinType::LeftSemi | JoinType::LeftAnti) {
                        (&join.left, &join.right, "l", "r")
                    } else {
                        (&join.right, &join.left, "r", "l")
                    };
                return Ok(format!(
                    "SELECT {} FROM ({}) AS {oa} WHERE {}EXISTS (SELECT 1 FROM ({}) AS {ia} WHERE {})",
                    select.join(", "),
                    render(outer, parameters)?,
                    if matches!(join.join_type, JoinType::LeftAnti | JoinType::RightAnti) {
                        "NOT "
                    } else {
                        ""
                    },
                    render(inner, parameters)?,
                    conditions.join(" AND ")
                ));
            }
            Ok(format!(
                "SELECT {} FROM ({}) AS l {kind} JOIN ({}) AS r ON {}",
                select.join(", "),
                render(&join.left, parameters)?,
                render(&join.right, parameters)?,
                conditions.join(" AND ")
            ))
        }
        LogicalPlan::Union(union) => Ok(union
            .inputs
            .iter()
            .map(|input| render(input, parameters).map(|sql| format!("({sql})")))
            .collect::<Result<Vec<_>>>()?
            .join(" UNION ALL ")),
        LogicalPlan::Window(window) => {
            let mut select = columns(window.input.schema().fields().len(), "q", 0);
            for expr in &window.window_expr {
                select.push(format!(
                    "{} AS __c{}",
                    expression(expr.clone(), &[(&window.input, "q")], parameters)?,
                    select.len()
                ));
            }
            Ok(format!(
                "SELECT {} FROM ({}) AS q",
                select.join(", "),
                render(&window.input, parameters)?
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
        _ => Err(failure("unqualified remote operator")),
    }
}
fn expression_type(expr: &Expr, plan: &LogicalPlan) -> Option<DataType> {
    plan.inputs()
        .into_iter()
        .find_map(|p| expr.get_type(p.schema()).ok())
        .or_else(|| expr.get_type(plan.schema()).ok())
}
fn ordered(expr: &Expr, plan: &LogicalPlan) -> bool {
    expression_type(expr, plan).is_some_and(|t| predicate::comparable(&t))
}
fn expr_supported(expr: &Expr, plan: &LogicalPlan) -> bool {
    use datafusion::functions_aggregate::{
        count::Count,
        min_max::{Max, Min},
        sum::Sum,
    };
    match expr {
        Expr::Column(_) | Expr::Alias(_) => true,
        Expr::Literal(v, _) => {
            predicate::sql_type(&v.data_type()).is_some() || matches!(v, ScalarValue::UInt8(_))
        }
        Expr::IsNull(_) | Expr::IsNotNull(_) | Expr::Not(_) => true,
        Expr::BinaryExpr(b) => {
            matches!(b.op, Operator::And | Operator::Or)
                || matches!(
                    b.op,
                    Operator::Eq
                        | Operator::NotEq
                        | Operator::Lt
                        | Operator::LtEq
                        | Operator::Gt
                        | Operator::GtEq
                ) && ordered(&b.left, plan)
                    && ordered(&b.right, plan)
        }
        Expr::InList(list) => {
            !list.list.is_empty()
                && list.list.len() <= 256
                && ordered(&list.expr, plan)
                && list.list.iter().all(|v| ordered(v, plan))
        }
        Expr::Cast(c) => expression_type(&c.expr, plan).is_some_and(|source| {
            POSTGRES_EXECUTION_PROFILE.safe_cast(&source, c.field.data_type())
        }),
        Expr::AggregateFunction(a) => {
            let implementation = a.func.inner().as_ref() as &dyn Any;
            let args = &a.params.args;
            let qualified = if implementation.is::<Count>() {
                !a.params.distinct || args.len() == 1 && args.iter().all(|e| ordered(e, plan))
            } else if implementation.is::<Sum>() {
                args.len() == 1
                    && matches!(
                        expression_type(&args[0], plan),
                        Some(DataType::Int16 | DataType::Int32 | DataType::Int64)
                    )
                    && !a.params.distinct
            } else if implementation.is::<Min>() || implementation.is::<Max>() {
                args.len() == 1
                    && ordered(&args[0], plan)
                    && expression_type(&args[0], plan) != Some(DataType::Boolean)
            } else {
                false
            };
            qualified && a.params.order_by.is_empty() && a.params.null_treatment.is_none()
        }
        Expr::ScalarFunction(f) => {
            let implementation = f.func.inner().as_ref() as &dyn Any;
            implementation.is::<datafusion::functions::core::coalesce::CoalesceFunc>()
                && f.args.iter().all(|e| ordered(e, plan))
                || implementation.is::<datafusion::functions::core::nullif::NullIfFunc>()
                    && f.args.iter().all(|e| ordered(e, plan))
        }
        Expr::WindowFunction(w) => {
            if let datafusion::logical_expr::WindowFunctionDefinition::WindowUDF(f) = &w.fun {
                let implementation = f.inner().as_ref() as &dyn Any;
                (implementation.is::<datafusion::functions_window::row_number::RowNumber>()
                    || implementation.is::<datafusion::functions_window::rank::Rank>())
                    && w.params.partition_by.iter().all(|e| ordered(e, plan))
                    && w.params.order_by.iter().all(|s| ordered(&s.expr, plan))
            } else {
                false
            }
        }
        _ => false,
    }
}
fn supported(plan: &LogicalPlan) -> Result<bool> {
    let mut yes = true;
    plan.apply(|node| {
        let ok = match node {
            LogicalPlan::TableScan(scan) => scan.filters.iter().all(|e| {
                predicate::translate(
                    e,
                    &scan.source.schema(),
                    256,
                    &mut predicate::Parameters::default(),
                )
                .is_some()
            }),
            LogicalPlan::Projection(_)
            | LogicalPlan::Filter(_)
            | LogicalPlan::Limit(_)
            | LogicalPlan::Window(_)
            | LogicalPlan::SubqueryAlias(_) => true,
            LogicalPlan::Union(u) => u
                .schema
                .fields()
                .iter()
                .all(|f| predicate::comparable(f.data_type())),
            LogicalPlan::Sort(s) => s.expr.iter().all(|s| ordered(&s.expr, node)),
            LogicalPlan::Aggregate(a) => a.group_expr.iter().all(|e| ordered(e, node)),
            LogicalPlan::Join(j) => {
                matches!(
                    j.join_type,
                    JoinType::Inner
                        | JoinType::Left
                        | JoinType::Right
                        | JoinType::Full
                        | JoinType::LeftSemi
                        | JoinType::LeftAnti
                        | JoinType::RightSemi
                        | JoinType::RightAnti
                ) && !j.on.is_empty()
                    && j.on
                        .iter()
                        .all(|(l, r)| ordered(l, node) && ordered(r, node))
                    && j.null_equality == datafusion::common::NullEquality::NullEqualsNothing
            }
            _ => false,
        };
        if !ok {
            yes = false;
            return Ok(TreeNodeRecursion::Stop);
        }
        for e in node.expressions() {
            e.apply(|e| {
                if !expr_supported(e, node) {
                    yes = false;
                    Ok(TreeNodeRecursion::Stop)
                } else {
                    Ok(TreeNodeRecursion::Continue)
                }
            })?;
        }
        Ok(if yes {
            TreeNodeRecursion::Continue
        } else {
            TreeNodeRecursion::Stop
        })
    })?;
    Ok(yes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::{
        arrow::datatypes::{Field, Schema},
        catalog::empty::EmptyTable,
        datasource::provider_as_source,
        logical_expr::LogicalPlanBuilder,
        prelude::{col, lit},
    };

    fn scan() -> LogicalPlanBuilder {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("label", DataType::Utf8, true),
        ]));
        LogicalPlanBuilder::scan(
            "input",
            provider_as_source(Arc::new(EmptyTable::new(schema))),
            None,
        )
        .unwrap()
    }

    #[test]
    fn eligibility_accepts_exact_integer_and_null_semantics() {
        let plan = scan()
            .filter(col("id").gt(lit(1_i64)).and(col("id").is_not_null()))
            .unwrap()
            .sort(vec![col("id").sort(true, false)])
            .unwrap()
            .limit(0, Some(10))
            .unwrap()
            .build()
            .unwrap();
        assert!(supported(&plan).unwrap());
    }

    #[test]
    fn eligibility_keeps_unpinned_text_and_arithmetic_local() {
        let text = scan()
            .filter(col("label").eq(lit("a")))
            .unwrap()
            .build()
            .unwrap();
        assert!(!supported(&text).unwrap());

        let arithmetic = scan()
            .project(vec![col("id") + lit(1_i64)])
            .unwrap()
            .build()
            .unwrap();
        assert!(!supported(&arithmetic).unwrap());
    }
}
