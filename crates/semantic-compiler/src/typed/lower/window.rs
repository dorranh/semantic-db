use super::*;
use semantic_plan::typed::{WindowFrame, WindowFunction};

pub(super) fn sql_window(window: &BoundWindow) -> ast::Expr {
    let mut statements = Parser::parse_sql(
        &GenericDialect {},
        "SELECT sum(x) OVER (RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)",
    )
    .expect("static window skeleton");
    let ast::Statement::Query(query) = &mut statements[0] else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    let ast::SelectItem::UnnamedExpr(ast::Expr::Function(function)) = &mut select.projection[0]
    else {
        unreachable!()
    };
    function.name = ast::ObjectName::from(vec![ast::Ident::new(match window.function {
        WindowFunction::Rank => "rank",
        WindowFunction::DenseRank => "dense_rank",
        WindowFunction::Count => "count",
        WindowFunction::Sum => "semantic_sum_v1",
        WindowFunction::Min => "min",
        WindowFunction::Max => "max",
    })]);
    let ast::FunctionArguments::List(args) = &mut function.args else {
        unreachable!()
    };
    args.args =
        if matches!(
            window.function,
            WindowFunction::Rank | WindowFunction::DenseRank
        ) {
            vec![]
        } else {
            vec![ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(
                window.input.as_ref().map(sql_field).unwrap_or_else(|| {
                    ast::Expr::Value(ast::Value::Number("1".into(), false).into())
                }),
            ))]
        };
    let Some(ast::WindowType::WindowSpec(spec)) = &mut function.over else {
        unreachable!()
    };
    spec.partition_by = window.partition_by.iter().map(sql_field).collect();
    spec.order_by = window
        .order_by
        .iter()
        .map(|(field, direction, nulls)| ast::OrderByExpr {
            expr: sql_field(field),
            options: ast::OrderByOptions {
                asc: Some(*direction == Direction::Asc),
                nulls_first: Some(*nulls == NullOrder::First),
            },
            with_fill: None,
        })
        .collect();
    if window.frame == WindowFrame::EntirePartition {
        spec.window_frame
            .as_mut()
            .expect("explicit frame")
            .end_bound = Some(ast::WindowFrameBound::Following(None));
    }
    ast::Expr::Function(function.clone())
}

pub(super) fn df_window(window: &BoundWindow) -> Expr {
    use datafusion::{
        functions_aggregate::{count, min_max},
        functions_window::rank,
        logical_expr::{
            WindowFrame as Frame,
            expr::{WindowFunction as Function, WindowFunctionDefinition},
        },
    };
    let function = match window.function {
        WindowFunction::Rank => WindowFunctionDefinition::WindowUDF(rank::rank_udwf()),
        WindowFunction::DenseRank => WindowFunctionDefinition::WindowUDF(rank::dense_rank_udwf()),
        WindowFunction::Count => WindowFunctionDefinition::AggregateUDF(count::count_udaf()),
        WindowFunction::Sum => {
            WindowFunctionDefinition::AggregateUDF(semantic_engine::semantic_sum_v1())
        }
        WindowFunction::Min => WindowFunctionDefinition::AggregateUDF(min_max::min_udaf()),
        WindowFunction::Max => WindowFunctionDefinition::AggregateUDF(min_max::max_udaf()),
    };
    let args = if matches!(
        window.function,
        WindowFunction::Rank | WindowFunction::DenseRank
    ) {
        vec![]
    } else {
        vec![
            window
                .input
                .as_ref()
                .map(df_field)
                .unwrap_or_else(|| lit(1i64)),
        ]
    };
    let mut expr = Function::new(function, args);
    expr.params.partition_by = window.partition_by.iter().map(df_field).collect();
    expr.params.order_by = window
        .order_by
        .iter()
        .map(|(field, direction, nulls)| {
            df_field(field).sort(*direction == Direction::Asc, *nulls == NullOrder::First)
        })
        .collect();
    expr.params.window_frame = Frame::new(if window.frame == WindowFrame::ThroughCurrentPeer {
        Some(false)
    } else {
        None
    });
    Expr::WindowFunction(Box::new(expr))
}
