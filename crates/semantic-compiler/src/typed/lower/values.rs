//! A bounded, typed relation operand shared by SQL and direct lowering.

use datafusion::{
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{LogicalPlanBuilder, col, lit},
    sql::sqlparser::{ast, dialect::GenericDialect, parser::Parser},
};

use super::super::calendar_spine::TypedValues;

fn cast_template(timestamp: bool) -> ast::Expr {
    let sql = if timestamp {
        "SELECT arrow_cast(0, 'Timestamp(µs, \"UTC\")')"
    } else {
        "SELECT arrow_cast(0, 'Int64')"
    };
    let mut parsed = Parser::parse_sql(&GenericDialect {}, sql).expect("static Values cast");
    let ast::Statement::Query(query) = parsed.remove(0) else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = *query.body else {
        unreachable!()
    };
    let ast::SelectItem::UnnamedExpr(expression) = select.projection.into_iter().next().unwrap()
    else {
        unreachable!()
    };
    expression
}

/// Construct a SQL VALUES relation from previously checked cells. Type casts
/// come from fixed compiler templates; no cell or identifier is parsed as SQL.
pub(super) fn sql_query(values: &TypedValues) -> Box<ast::Query> {
    let templates = values
        .schema
        .fields()
        .iter()
        .map(|field| {
            cast_template(!matches!(
                field.data_type(),
                datafusion::arrow::datatypes::DataType::Int64
            ))
        })
        .collect::<Vec<_>>();
    let mut parsed =
        Parser::parse_sql(&GenericDialect {}, "VALUES (0)").expect("static Values relation");
    let ast::Statement::Query(mut query) = parsed.remove(0) else {
        unreachable!()
    };
    let ast::SetExpr::Values(body) = query.body.as_mut() else {
        unreachable!()
    };
    body.rows = values
        .rows
        .iter()
        .map(|row| {
            ast::Parens::with_empty_span(
                row.iter()
                    .zip(&templates)
                    .map(|(cell, template)| {
                        let mut expression = template.clone();
                        let ast::Expr::Function(function) = &mut expression else {
                            unreachable!()
                        };
                        let ast::FunctionArguments::List(arguments) = &mut function.args else {
                            unreachable!()
                        };
                        let literal = match cell {
                            ScalarValue::Int64(Some(value))
                            | ScalarValue::TimestampMicrosecond(Some(value), _) => {
                                ast::Value::Number(value.to_string(), false)
                            }
                            ScalarValue::Int64(None)
                            | ScalarValue::TimestampMicrosecond(None, _) => ast::Value::Null,
                            _ => unreachable!("TypedValues rejected unsupported cell"),
                        };
                        arguments.args[0] = ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(
                            ast::Expr::Value(literal.into()),
                        ));
                        expression
                    })
                    .collect(),
            )
        })
        .collect();
    query
}

/// Plan the same typed cells against the current read-only execution state and
/// return both the Values relation and the unchanged input relation.
pub(super) fn plan(
    input: DataFrame,
    values: &TypedValues,
) -> datafusion::error::Result<(DataFrame, DataFrame)> {
    let (state, input_plan) = input.into_parts();
    let rows = values
        .rows
        .iter()
        .map(|row| row.iter().cloned().map(lit).collect())
        .collect();
    let projections = values
        .schema
        .fields()
        .iter()
        .enumerate()
        .map(|(index, field)| col(&format!("column{}", index + 1)).alias(field.name()))
        .collect::<Vec<_>>();
    let relation = LogicalPlanBuilder::values(rows)?
        .project(projections)?
        .build()?;
    Ok((
        DataFrame::new(state.clone(), relation),
        DataFrame::new(state, input_plan),
    ))
}

#[cfg(test)]
#[path = "values/tests.rs"]
mod tests;
