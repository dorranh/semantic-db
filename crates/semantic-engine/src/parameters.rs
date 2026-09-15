//! Transport-independent parameter binding. SQL values are never interpolated.
use crate::{Engine, QueryExecution, QueryOptions, Result, query::execute_frame};
use datafusion::{
    arrow::datatypes::{DataType, SchemaRef, TimeUnit},
    common::{
        ScalarValue,
        tree_node::{TreeNode, TreeNodeRecursion},
    },
    logical_expr::Expr,
    sql::{
        parser::Statement,
        sqlparser::ast::{self, visit_expressions_mut},
    },
};
use semantic_runtime::{QueryContext, failure};
use std::{ops::ControlFlow, sync::Arc};

#[derive(Debug, Clone)]
pub struct ReadDescription {
    pub parameters: Vec<DataType>,
    pub schema: SchemaRef,
}

impl Engine {
    /// Describe one read without executing rows or acquiring materializations.
    /// Hints are protocol-supplied parameter types; None asks the planner to infer.
    pub async fn describe_read(
        &self,
        sql: &str,
        hints: &[Option<DataType>],
    ) -> Result<ReadDescription> {
        let sql = self.parameter_sql(sql, hints)?;
        let frame = self.plan_sql(&sql).await?;
        let mut types = frame.logical_plan().get_parameter_types()?;
        // DataFusion leaves standalone CAST($n AS type) placeholders untyped.
        // Use the immediately enclosing cast only where inference had no type.
        frame.logical_plan().apply_with_subqueries(|plan| {
            plan.apply_expressions(|expr| {
                expr.apply(|expr| {
                    if let Expr::Cast(cast) = expr
                        && let Expr::Placeholder(parameter) = cast.expr.as_ref()
                    {
                        let entry = types.entry(parameter.id.clone()).or_default();
                        if entry.is_none() {
                            *entry = Some(cast.field.data_type().clone());
                        }
                    }
                    Ok(TreeNodeRecursion::Continue)
                })
            })
        })?;
        let count = types
            .keys()
            .map(|s| position(s))
            .collect::<datafusion::error::Result<Vec<_>>>()?
            .into_iter()
            .max()
            .unwrap_or(0);
        if hints.len() > count {
            return Err(failure("more parameter type hints than SQL parameters").into());
        }
        let parameters = (1..=count)
            .map(|i| {
                types
                    .get(&format!("${i}"))
                    .and_then(Clone::clone)
                    .or_else(|| hints.get(i - 1).cloned().flatten())
                    .ok_or_else(|| {
                        failure(
                            "could not infer parameter type; use an explicit SQL cast or type hint",
                        )
                    })
            })
            .collect::<datafusion::error::Result<Vec<_>>>()?;
        Ok(ReadDescription {
            parameters,
            schema: Arc::new(frame.schema().as_arrow().clone()),
        })
    }

    /// Execute a parameterized read through the same validation, cache and budget
    /// path as execute. Hints are derived from typed values, including typed NULLs.
    pub async fn execute_parameters(
        &self,
        sql: &str,
        parameters: Vec<ScalarValue>,
        options: QueryOptions,
    ) -> Result<QueryExecution> {
        let hints = parameters
            .iter()
            .map(|v| Some(v.data_type()))
            .collect::<Vec<_>>();
        let sql = self.parameter_sql(sql, &hints)?;
        let description = self.describe_read(&sql, &[]).await?;
        if description.parameters.len() != parameters.len() {
            return Err(failure("parameter count does not match SQL").into());
        }
        let context = QueryContext::new(options)?;
        let frame = context
            .run(async {
                self.execution_frame(&sql, &context)
                    .await
                    .map_err(|e| failure(&e.to_string()))
            })
            .await?
            .with_param_values(parameters)?;
        execute_frame(frame, context).await
    }

    fn parameter_sql(&self, sql: &str, hints: &[Option<DataType>]) -> Result<String> {
        let state = self.context.state();
        // This parser accepts exactly one statement. Do not split on semicolons.
        let mut statement =
            state.sql_to_statement(sql, &state.config_options().sql_parser.dialect)?;
        if let Statement::Statement(inner) = &mut statement {
            let result = visit_expressions_mut(inner.as_mut(), |expr| {
                if let ast::Expr::Value(value) = expr
                    && let ast::Value::Placeholder(id) = &value.value
                {
                    let index = match position(id) {
                        Ok(v) => v,
                        Err(e) => return ControlFlow::Break(e),
                    };
                    if let Some(Some(ty)) = hints.get(index - 1) {
                        let ty = match sql_type(ty) {
                            Ok(v) => v,
                            Err(e) => return ControlFlow::Break(e),
                        };
                        *expr = ast::Expr::Cast {
                            kind: ast::CastKind::Cast,
                            expr: Box::new(expr.clone()),
                            data_type: ty,
                            format: None,
                            array: false,
                        };
                    }
                }
                ControlFlow::Continue(())
            });
            if let ControlFlow::Break(e) = result {
                return Err(e.into());
            }
        }
        Ok(statement.to_string())
    }
}

fn position(id: &str) -> datafusion::error::Result<usize> {
    id.strip_prefix('$')
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= 65535)
        .ok_or_else(|| failure("parameters must be numbered $1 through $65535"))
}

fn sql_type(ty: &DataType) -> datafusion::error::Result<ast::DataType> {
    use ast::DataType as T;
    Ok(match ty {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => T::Text,
        DataType::Boolean => T::Boolean,
        DataType::Int16 => T::SmallInt(None),
        DataType::Int32 => T::Int(None),
        DataType::Int64 => T::BigInt(None),
        DataType::Float32 => T::Real,
        DataType::Float64 => T::Double(ast::ExactNumberInfo::None),
        DataType::Date32 => T::Date,
        DataType::Timestamp(unit, tz) => T::Timestamp(
            Some(match unit {
                TimeUnit::Second => 0,
                TimeUnit::Millisecond => 3,
                TimeUnit::Microsecond => 6,
                TimeUnit::Nanosecond => 9,
            }),
            if tz.is_some() {
                ast::TimezoneInfo::WithTimeZone
            } else {
                ast::TimezoneInfo::None
            },
        ),
        _ => return Err(failure("unsupported parameter type")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn parameters_are_values_and_sorted_numerically() {
        let engine = Engine::new();
        let sql = "SELECT $1::text AS text, $2::bigint + $10::bigint AS total, $3::int, $4::int, $5::int, $6::int, $7::int, $8::int, $9::int";
        let d = engine.describe_read(sql, &[]).await.unwrap();
        assert_eq!(d.parameters.len(), 10);
        let mut values = vec![ScalarValue::Utf8(Some("'; DROP TABLE x; --".into()))];
        values.extend((2..=10).map(|n| ScalarValue::Int64(Some(n))));
        let rows = engine
            .execute_parameters(sql, values, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(rows[0].num_rows(), 1);
        assert_eq!(
            ScalarValue::try_from_array(rows[0].column(1), 0).unwrap(),
            ScalarValue::Int64(Some(12))
        );
        assert!(
            engine
                .describe_read("SELECT 1; SELECT 2", &[])
                .await
                .is_err()
        );
        for sql in ["BEGIN", "DELETE FROM x", "CREATE TABLE x(a int)"] {
            assert!(engine.describe_read(sql, &[]).await.is_err());
        }
        let d = engine
            .describe_read("SELECT $1 AS nullable", &[Some(DataType::Int64)])
            .await
            .unwrap();
        assert_eq!(d.parameters, vec![DataType::Int64]);
        assert!(
            engine
                .execute_parameters("SELECT $1::int", vec![], QueryOptions::default())
                .await
                .is_err()
        );
        engine
            .execute_parameters(
                "SELECT $1",
                vec![ScalarValue::Int64(None)],
                QueryOptions::default(),
            )
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
    }
}

#[cfg(test)]
mod temporal_tests {
    use super::*;
    #[tokio::test]
    async fn typed_timestamps_retain_their_unit_and_microsecond_precision() {
        let engine = Engine::new();
        let value = ScalarValue::TimestampMicrosecond(Some(-30_000_000_000_123_456), None);
        let description = engine
            .describe_read("SELECT $1 AS historical", &[Some(value.data_type())])
            .await
            .unwrap();
        assert_eq!(description.parameters, vec![value.data_type()]);
        let rows = engine
            .execute_parameters(
                "SELECT $1 AS historical",
                vec![value.clone()],
                QueryOptions::default(),
            )
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(
            ScalarValue::try_from_array(rows[0].column(0), 0).unwrap(),
            value
        );
    }
}
