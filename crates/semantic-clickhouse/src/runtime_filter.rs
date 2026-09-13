//! Snapshot optional join filters at execution. The local join remains authoritative.
use crate::{Connection, policy};
use datafusion::{
    arrow::datatypes::SchemaRef,
    common::ScalarValue,
    logical_expr::{Expr, Operator},
    physical_expr::{
        PhysicalExpr,
        expressions::{BinaryExpr, Column, DynamicFilterPhysicalExpr, InListExpr, Literal},
    },
};
use std::sync::Arc;

pub(crate) fn apply(
    sql: String,
    filters: &[Arc<dyn PhysicalExpr>],
    schema: &SchemaRef,
    connection: &Connection,
) -> (String, usize) {
    if !connection.config.runtime_filters {
        return (sql, 0);
    }
    // Synthetic output names can repeat in a join; only unambiguous outputs are eligible.
    let mut names = std::collections::BTreeSet::new();
    if schema.fields().iter().any(|f| !names.insert(f.name())) {
        return (sql, 0);
    }
    let mut predicates = vec![];
    for filter in filters {
        gather(filter, schema, connection, &mut predicates);
    }
    let text = predicates.join(" AND ");
    if text.is_empty() || text.len() > connection.config.runtime_filter_max_bytes {
        return (sql, 0);
    }
    let count = predicates.len();
    (
        format!("SELECT * FROM ({sql}) AS __runtime_filter WHERE {text}"),
        count,
    )
}
fn gather(
    filter: &Arc<dyn PhysicalExpr>,
    schema: &SchemaRef,
    connection: &Connection,
    output: &mut Vec<String>,
) {
    if let Some(dynamic) =
        (filter.as_ref() as &dyn std::any::Any).downcast_ref::<DynamicFilterPhysicalExpr>()
    {
        if let Ok(current) = dynamic.current() {
            gather(&current, schema, connection, output);
        }
        return;
    }
    if let Some(binary) = (filter.as_ref() as &dyn std::any::Any).downcast_ref::<BinaryExpr>() {
        if *binary.op() == Operator::And {
            gather(binary.left(), schema, connection, output);
            gather(binary.right(), schema, connection, output);
        }
        return;
    }
    let Some(list) = (filter.as_ref() as &dyn std::any::Any).downcast_ref::<InListExpr>() else {
        return;
    };
    if list.negated() || list.list().len() > connection.config.runtime_filter_max_keys {
        return;
    }
    let Some(column) = (list.expr().as_ref() as &dyn std::any::Any).downcast_ref::<Column>() else {
        return;
    };
    let Some(field) = schema.fields().get(column.index()) else {
        return;
    };
    let mut values = vec![];
    for value in list.list() {
        let Some(literal) = (value.as_ref() as &dyn std::any::Any).downcast_ref::<Literal>() else {
            return;
        };
        let value = literal.value();
        if value.is_null() {
            continue;
        }
        if !matches!(
            value,
            ScalarValue::Int8(_)
                | ScalarValue::Int16(_)
                | ScalarValue::Int32(_)
                | ScalarValue::Int64(_)
                | ScalarValue::UInt8(_)
                | ScalarValue::UInt16(_)
                | ScalarValue::UInt32(_)
                | ScalarValue::UInt64(_)
                | ScalarValue::Utf8(_)
                | ScalarValue::Utf8View(_)
                | ScalarValue::LargeUtf8(_)
        ) {
            return;
        }
        if matches!(value,ScalarValue::Utf8(Some(s))|ScalarValue::Utf8View(Some(s))|ScalarValue::LargeUtf8(Some(s)) if s.contains('\\'))
        {
            return;
        }
        let Ok(text) = policy::filter_sql(&Expr::Literal(value.clone(), None)) else {
            return;
        };
        values.push(text);
    }
    if values.is_empty() {
        output.push("false".into());
    } else {
        output.push(format!(
            "{} IN ({})",
            crate::quote_identifier(field.name()),
            values.join(", ")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClickHouse, ClickHouseConfig};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    #[test]
    fn complete_key_lists_are_used_without_truncation() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
        let mut config = ClickHouseConfig::new("http://localhost:8123", "test", "reader", "");
        config.runtime_filter_max_keys = 2;
        let connection = ClickHouse::new(config).unwrap();
        let list = |values: Vec<Option<i64>>, negated: bool| -> Arc<dyn PhysicalExpr> {
            Arc::new(
                InListExpr::try_new_from_array(
                    Arc::new(Column::new("id", 0)),
                    Arc::new(datafusion::arrow::array::Int64Array::from(values)),
                    negated,
                    &schema,
                )
                .unwrap(),
            )
        };
        let sql = "SELECT id FROM t".to_owned();
        let (rendered, count) = apply(
            sql.clone(),
            &[list(vec![Some(1), Some(2)], false)],
            &schema,
            &connection.0,
        );
        assert_eq!(count, 1);
        assert!(rendered.contains("IN (1, 2)"));
        for filter in [
            list(vec![Some(1), Some(2), Some(3)], false),
            list(vec![Some(1)], true),
        ] {
            assert_eq!(
                apply(sql.clone(), &[filter], &schema, &connection.0),
                (sql.clone(), 0)
            );
        }
        let (rendered, _) = apply(sql, &[list(vec![None], false)], &schema, &connection.0);
        assert!(rendered.ends_with("WHERE false"));
    }
}
