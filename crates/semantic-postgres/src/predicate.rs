//! Exact, parameterized predicates. Text ordering/collations, floats and casts
//! deliberately stay local. A partial conjunction is a safe remote superset.
use crate::*;
use datafusion::{
    common::ScalarValue,
    logical_expr::{Operator, TableProviderFilterPushDown},
};
use tokio_postgres::types::ToSql;

#[derive(Clone, Default)]
pub(crate) struct Parameters(pub Vec<ScalarValue>);
impl fmt::Debug for Parameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Parameters({})", self.0.len())
    }
}
impl Parameters {
    pub fn push(&mut self, value: &ScalarValue) -> Result<String> {
        let ty = sql_type(&value.data_type())
            .ok_or_else(|| failure("unsupported Postgres parameter"))?;
        self.0.push(value.clone());
        Ok(format!("${}::{ty}", self.0.len()))
    }
    pub fn bind(&self, types: &[Type]) -> Result<Vec<Box<dyn ToSql + Sync + Send>>> {
        if types.len() != self.0.len() {
            return Err(failure("Postgres parameter count mismatch"));
        }
        self.0
            .iter()
            .zip(types)
            .map(|(v, t)| {
                crate::write::value(v, t)
                    .map_err(|_| failure("Postgres parameter conversion failed"))
            })
            .collect()
    }
}
pub(crate) fn sql_type(ty: &DataType) -> Option<String> {
    POSTGRES_EXECUTION_PROFILE
        .parameter_type(ty)
        .map(str::to_owned)
}
pub(crate) fn comparable(ty: &DataType) -> bool {
    POSTGRES_EXECUTION_PROFILE.comparable(ty)
}
fn column<'a>(e: &'a Expr, schema: &'a Schema) -> Option<(&'a str, &'a DataType)> {
    if let Expr::Column(c) = e {
        schema
            .field_with_name(&c.name)
            .ok()
            .map(|f| (f.name().as_str(), f.data_type()))
    } else {
        None
    }
}
fn literal(e: &Expr, ty: &DataType, params: &mut Parameters) -> Option<String> {
    if let Expr::Literal(v, _) = e {
        // Never narrow or reinterpret literals. Optimizer coercions stay local.
        if &v.data_type() == ty {
            return params.push(v).ok();
        }
    }
    None
}
fn exact(e: &Expr, schema: &Schema, cap: usize, params: &mut Parameters) -> Option<String> {
    match e {
        Expr::BinaryExpr(b) if matches!(b.op, Operator::And | Operator::Or) => Some(format!(
            "({} {} {})",
            exact(&b.left, schema, cap, params)?,
            b.op,
            exact(&b.right, schema, cap, params)?
        )),
        Expr::Not(e) => Some(format!("(NOT {})", exact(e, schema, cap, params)?)),
        Expr::IsNull(v) | Expr::IsNotNull(v) => Some(format!(
            "({} IS {}NULL)",
            identifier(column(v, schema)?.0).ok()?,
            if matches!(e, Expr::IsNotNull(_)) {
                "NOT "
            } else {
                ""
            }
        )),
        Expr::BinaryExpr(b)
            if matches!(
                b.op,
                Operator::Eq
                    | Operator::NotEq
                    | Operator::Lt
                    | Operator::LtEq
                    | Operator::Gt
                    | Operator::GtEq
            ) =>
        {
            if let Some((name, ty)) = column(&b.left, schema) {
                if !comparable(ty) {
                    return None;
                }
                Some(format!(
                    "({} {} {})",
                    identifier(name).ok()?,
                    b.op,
                    literal(&b.right, ty, params)?
                ))
            } else {
                let (name, ty) = column(&b.right, schema)?;
                if !comparable(ty) {
                    return None;
                }
                Some(format!(
                    "({} {} {})",
                    literal(&b.left, ty, params)?,
                    b.op,
                    identifier(name).ok()?
                ))
            }
        }
        Expr::InList(list) => {
            let (name, ty) = column(&list.expr, schema)?;
            if !comparable(ty) || list.list.is_empty() || list.list.len() > cap {
                return None;
            }
            let values = list
                .list
                .iter()
                .map(|v| literal(v, ty, params))
                .collect::<Option<Vec<_>>>()?;
            Some(format!(
                "({} {}IN ({}))",
                identifier(name).ok()?,
                if list.negated { "NOT " } else { "" },
                values.join(", ")
            ))
        }
        _ => None,
    }
}
pub(crate) fn translate(
    e: &Expr,
    schema: &Schema,
    cap: usize,
    params: &mut Parameters,
) -> Option<String> {
    let start = params.0.len();
    // Handle null tests separately to retain the negation.
    let result = match e {
        Expr::IsNull(v) | Expr::IsNotNull(v) => column(v, schema)
            .and_then(|(n, _)| identifier(n).ok())
            .map(|n| {
                format!(
                    "({n} IS {}NULL)",
                    if matches!(e, Expr::IsNotNull(_)) {
                        "NOT "
                    } else {
                        ""
                    }
                )
            }),
        _ => exact(e, schema, cap, params),
    };
    if result.is_none() {
        params.0.truncate(start);
    }
    result
}
pub(crate) fn classify(
    e: &Expr,
    schema: &Schema,
    options: &PostgresOptions,
) -> TableProviderFilterPushDown {
    if !options.filter_pushdown {
        return TableProviderFilterPushDown::Unsupported;
    }
    if translate(e, schema, options.max_in_list, &mut Parameters::default()).is_some() {
        TableProviderFilterPushDown::Exact
    } else if let Expr::BinaryExpr(b) = e
        && b.op == Operator::And
        && [b.left.as_ref(), b.right.as_ref()]
            .iter()
            .any(|e| classify(e, schema, options) != TableProviderFilterPushDown::Unsupported)
    {
        TableProviderFilterPushDown::Inexact
    } else {
        TableProviderFilterPushDown::Unsupported
    }
}
fn terms(e: &Expr, schema: &Schema, cap: usize, params: &mut Parameters) -> Vec<String> {
    if let Some(sql) = translate(e, schema, cap, params) {
        vec![sql]
    } else if let Expr::BinaryExpr(b) = e
        && b.op == Operator::And
    {
        let mut v = terms(&b.left, schema, cap, params);
        v.extend(terms(&b.right, schema, cap, params));
        v
    } else {
        vec![]
    }
}
pub(crate) fn scan_sql(
    target: &str,
    schema: &Schema,
    projection: Option<&Vec<usize>>,
    filters: &[Expr],
    limit: Option<usize>,
    options: &PostgresOptions,
) -> Result<(String, SchemaRef, Parameters)> {
    let indices = projection
        .cloned()
        .unwrap_or_else(|| (0..schema.fields().len()).collect());
    let projected = Arc::new(schema.project(&indices)?);
    let cols = indices
        .iter()
        .map(|i| identifier(schema.field(*i).name()))
        .collect::<Result<Vec<_>>>()?;
    let mut sql = format!(
        "SELECT {} FROM {target}",
        if cols.is_empty() {
            "1 AS __row".into()
        } else {
            cols.join(", ")
        }
    );
    let mut params = Parameters::default();
    if options.filter_pushdown {
        let predicates = filters
            .iter()
            .flat_map(|e| terms(e, schema, options.max_in_list, &mut params))
            .collect::<Vec<_>>();
        if !predicates.is_empty() {
            sql.push_str(&format!(" WHERE {}", predicates.join(" AND ")));
        }
    }
    if filters
        .iter()
        .all(|e| classify(e, schema, options) == TableProviderFilterPushDown::Exact)
        && let Some(limit) = limit
    {
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    Ok((sql, projected, params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::prelude::{col, lit};
    fn schema() -> Schema {
        Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("label", DataType::Utf8, true),
        ])
    }
    #[test]
    fn exact_nulls_parameters_and_atomic_or() {
        let schema = schema();
        let options = PostgresOptions::default();
        let filter = col("id").gt(lit(4i64)).and(col("label").is_not_null());
        let mut p = Parameters::default();
        let sql = translate(&filter, &schema, 256, &mut p).unwrap();
        assert!(sql.contains("$1::bigint") && sql.contains("IS NOT NULL"));
        assert_eq!(p.0, vec![ScalarValue::Int64(Some(4))]);
        assert_eq!(
            classify(&filter, &schema, &options),
            TableProviderFilterPushDown::Exact
        );
        let unsupported = col("label").eq(lit("secret '); DROP TABLE x; --"));
        let or = filter.clone().or(unsupported.clone());
        assert!(translate(&or, &schema, 256, &mut p).is_none());
        assert_eq!(p.0.len(), 1);
        assert_eq!(
            classify(&or, &schema, &options),
            TableProviderFilterPushDown::Unsupported
        );
        let partial = filter.and(unsupported);
        assert_eq!(
            classify(&partial, &schema, &options),
            TableProviderFilterPushDown::Inexact
        );
        let (sql, output, p) = scan_sql(
            "\"public\".\"t\"",
            &schema,
            Some(&vec![]),
            &[partial],
            Some(1),
            &options,
        )
        .unwrap();
        assert!(!sql.contains("LIMIT") && !sql.contains("secret"));
        assert!(sql.contains("IS NOT NULL"));
        assert_eq!(p.0.len(), 1);
        assert!(output.fields().is_empty());
    }
    #[test]
    fn rejects_casts_nonfinite_and_large_lists() {
        let schema = schema();
        let list = col("id").in_list(vec![lit(1i64), lit(2i64)], false);
        assert!(translate(&list, &schema, 1, &mut Parameters::default()).is_none());
        assert!(
            translate(
                &col("id").eq(lit(1i32)),
                &schema,
                256,
                &mut Parameters::default()
            )
            .is_none()
        );
        let options = PostgresOptions {
            filter_pushdown: false,
            ..Default::default()
        };
        assert_eq!(
            classify(&col("id").eq(lit(1i64)), &schema, &options),
            TableProviderFilterPushDown::Unsupported
        );
    }
}
