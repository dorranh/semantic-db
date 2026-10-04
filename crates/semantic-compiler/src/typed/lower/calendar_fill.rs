use super::*;
use datafusion::logical_expr::{JoinType, col};

fn qualified(alias: &str, field: &str) -> ast::Expr {
    ast::Expr::CompoundIdentifier(vec![ident(alias), ident(field)])
}

pub(super) fn emit(
    query: &mut ast::Query,
    month: &BoundField,
    count: &BoundField,
    months: &[i64],
    fill: i64,
    _parameters: &mut Vec<Literal>,
) {
    let mut previous = query.clone();
    // Aggregate emission has wrapped its result with a placeholder SELECT x;
    // ordinary projection would replace that later. The fill join embeds the
    // aggregate now, so expose its generated output slots first.
    let ast::SetExpr::Select(previous_select) = previous.body.as_mut() else {
        unreachable!()
    };
    previous_select.projection = vec![ast::SelectItem::Wildcard(Default::default())];
    let values = super::super::calendar_spine::TypedValues::utc_month_spine(months)
        .expect("bound calendar spine values");
    let mut skeleton = Parser::parse_sql(
        &GenericDialect {},
        "SELECT spine.m AS m, coalesce(agg.n, 0::BIGINT) AS n FROM (VALUES (arrow_cast(0, 'Timestamp(µs, \"UTC\")'))) AS spine(m) LEFT JOIN (SELECT n FROM t) AS agg ON spine.m = agg.m",
    )
    .expect("static calendar fill skeleton");
    let ast::Statement::Query(outer) = skeleton.remove(0) else {
        unreachable!()
    };
    let ast::SetExpr::Select(mut select) = *outer.body else {
        unreachable!()
    };
    let ast::TableFactor::Derived { subquery, .. } = &mut select.from[0].relation else {
        unreachable!()
    };
    *subquery = values::sql_query(&values);
    let ast::SelectItem::ExprWithAlias { alias, .. } = &mut select.projection[0] else {
        unreachable!()
    };
    *alias = ident(month.field.name());
    let ast::SelectItem::ExprWithAlias { expr, alias } = &mut select.projection[1] else {
        unreachable!()
    };
    *alias = ident(count.field.name());
    let ast::Expr::Function(function) = expr else {
        unreachable!()
    };
    let ast::FunctionArguments::List(arguments) = &mut function.args else {
        unreachable!()
    };
    arguments.args = vec![
        ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(qualified(
            "agg",
            count.field.name(),
        ))),
        ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(ast::Expr::Value(
            ast::Value::Number(fill.to_string(), false).into(),
        ))),
    ];
    let join = &mut select.from[0].joins[0];
    let ast::TableFactor::Derived { subquery, .. } = &mut join.relation else {
        unreachable!()
    };
    **subquery = previous;
    join.join_operator = ast::JoinOperator::LeftOuter(ast::JoinConstraint::On(binary(
        qualified("spine", "m"),
        ast::BinaryOperator::Eq,
        qualified("agg", month.field.name()),
    )));
    *query.body = ast::SetExpr::Select(select);
    wrap_query(query);
}

pub(super) fn plan(
    input: DataFrame,
    month: &BoundField,
    count: &BoundField,
    months: &[i64],
    fill: i64,
) -> datafusion::error::Result<DataFrame> {
    let values = super::super::calendar_spine::TypedValues::utc_month_spine(months)
        .expect("bound calendar spine values");
    let spine_field = values.schema.field(0).name().clone();
    let (spine, observed) = values::plan(input, &values)?;
    spine
        .join(
            observed,
            JoinType::Left,
            &[spine_field.as_str()],
            &[month.field.name()],
            None,
        )?
        .select(vec![
            col(spine_field.as_str()).alias(month.field.name()),
            datafusion::functions::core::coalesce()
                .call(vec![col(count.field.name()), lit(fill)])
                .alias(count.field.name()),
        ])
}
