use super::*;
use datafusion::logical_expr::{
    JoinType, Operator,
    expr::{BinaryExpr, WindowFunction, WindowFunctionDefinition},
};

fn occurrence_name(columns: &[SetColumn]) -> String {
    let mut name = "__semantic_occurrence".to_string();
    while columns.iter().any(|c| c.alias == name) {
        name.push('_');
    }
    name
}
fn qualified(alias: &str, name: &str) -> Expr {
    Expr::Column(Column::new(Some(alias), name))
}
/// DataFusion 55's native INTERSECT/EXCEPT ALL use semi/anti membership and do
/// not subtract duplicate counts. Pair occurrences of identical rows instead.
/// Within a partition every projected value is equal, so tie order is immaterial.
pub(super) fn bag(
    left: DataFrame,
    right: DataFrame,
    columns: &[SetColumn],
    operator: SetOperator,
) -> Result<DataFrame, CompileDiagnostic> {
    let occurrence = occurrence_name(columns);
    let mut window = WindowFunction::new(
        WindowFunctionDefinition::WindowUDF(
            datafusion::functions_window::row_number::row_number_udwf(),
        ),
        vec![],
    );
    window.params.partition_by = columns.iter().map(|c| col(&c.alias)).collect();
    let expression = Expr::WindowFunction(Box::new(window));
    let left = left
        .with_column(&occurrence, expression.clone())
        .and_then(|f| f.alias("l"))
        .map_err(lower::backend_error)?;
    let right = right
        .with_column(&occurrence, expression)
        .and_then(|f| f.alias("r"))
        .map_err(lower::backend_error)?;
    let mut on: Vec<_> = columns
        .iter()
        .map(|c| {
            Expr::BinaryExpr(BinaryExpr::new(
                Box::new(qualified("l", &c.alias)),
                Operator::IsNotDistinctFrom,
                Box::new(qualified("r", &c.alias)),
            ))
        })
        .collect();
    on.push(qualified("l", &occurrence).eq(qualified("r", &occurrence)));
    left.join_on(
        right,
        if operator == SetOperator::Intersect {
            JoinType::LeftSemi
        } else {
            JoinType::LeftAnti
        },
        on,
    )
    .and_then(|f| {
        f.select(
            columns
                .iter()
                .map(|c| qualified("l", &c.alias).alias(&c.alias))
                .collect::<Vec<_>>(),
        )
    })
    .map_err(lower::backend_error)
}
fn sql_col(alias: &str, name: &str) -> ast::Expr {
    ast::Expr::CompoundIdentifier(vec![ident(alias), ident(name)])
}
fn numbered(input: Box<ast::Query>, columns: &[SetColumn], occurrence: &str) -> Box<ast::Query> {
    let mut q =
        parsed("SELECT src.*, row_number() OVER (PARTITION BY x) AS n FROM (SELECT 1) AS src");
    let ast::SetExpr::Select(s) = q.body.as_mut() else {
        unreachable!()
    };
    let ast::TableFactor::Derived { subquery, .. } = &mut s.from[0].relation else {
        unreachable!()
    };
    *subquery = input;
    let ast::SelectItem::ExprWithAlias {
        expr: ast::Expr::Function(function),
        alias,
    } = &mut s.projection[1]
    else {
        unreachable!()
    };
    *alias = ident(occurrence);
    let Some(ast::WindowType::WindowSpec(window)) = &mut function.over else {
        unreachable!()
    };
    window.partition_by = columns.iter().map(|c| sql_col("src", &c.alias)).collect();
    q
}
pub(super) fn sql_bag(
    left: Box<ast::Query>,
    right: Box<ast::Query>,
    columns: &[SetColumn],
    operator: SetOperator,
) -> Box<ast::Query> {
    let occurrence = occurrence_name(columns);
    let mut q = parsed("SELECT l.x FROM (SELECT 1) AS l LEFT SEMI JOIN (SELECT 1) AS r ON true");
    let ast::SetExpr::Select(s) = q.body.as_mut() else {
        unreachable!()
    };
    let ast::TableFactor::Derived { subquery, .. } = &mut s.from[0].relation else {
        unreachable!()
    };
    *subquery = numbered(left, columns, &occurrence);
    let join = &mut s.from[0].joins[0];
    let ast::TableFactor::Derived { subquery, .. } = &mut join.relation else {
        unreachable!()
    };
    *subquery = numbered(right, columns, &occurrence);
    let condition = columns
        .iter()
        .map(|c| {
            ast::Expr::IsNotDistinctFrom(
                Box::new(sql_col("l", &c.alias)),
                Box::new(sql_col("r", &c.alias)),
            )
        })
        .chain([ast::Expr::BinaryOp {
            left: Box::new(sql_col("l", &occurrence)),
            op: ast::BinaryOperator::Eq,
            right: Box::new(sql_col("r", &occurrence)),
        }])
        .map(|expr| ast::Expr::Nested(Box::new(expr)))
        .reduce(|left, right| ast::Expr::BinaryOp {
            left: Box::new(left),
            op: ast::BinaryOperator::And,
            right: Box::new(right),
        })
        .expect("occurrence condition");
    join.join_operator = if operator == SetOperator::Intersect {
        ast::JoinOperator::LeftSemi(ast::JoinConstraint::On(condition))
    } else {
        ast::JoinOperator::LeftAnti(ast::JoinConstraint::On(condition))
    };
    s.projection = columns
        .iter()
        .map(|c| ast::SelectItem::ExprWithAlias {
            expr: sql_col("l", &c.alias),
            alias: ident(&c.alias),
        })
        .collect();
    q
}
