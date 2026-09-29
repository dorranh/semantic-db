use super::*;
use semantic_plan::typed::MissingMatch;

pub(super) fn emit_lookup(
    query: &mut ast::Query,
    lookup: &BoundLookup,
    parameters: &mut Vec<Literal>,
) {
    let ast::SetExpr::Select(input) = query.body.as_mut() else {
        unreachable!()
    };
    input.projection = vec![ast::SelectItem::Wildcard(Default::default())];
    wrap_query(query);
    let mut skeleton = Parser::parse_sql(&GenericDialect {}, "SELECT src.*, rhs.v AS v FROM t AS src LEFT JOIN (SELECT k AS k, min(v) AS v FROM t AS src GROUP BY k HAVING semantic_assert_single_v1(count(1))) AS rhs ON true").expect("static lookup skeleton");
    let ast::Statement::Query(outer) = &mut skeleton[0] else {
        unreachable!()
    };
    let ast::SetExpr::Select(template) = outer.body.as_mut() else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = template.projection.clone();
    select.projection[1] = ast::SelectItem::ExprWithAlias {
        expr: ast::Expr::CompoundIdentifier(vec![ident("rhs"), ident("__semantic_value")]),
        alias: ident(lookup.output.field.name()),
    };
    let mut join = template.from[0].joins.remove(0);
    let ast::TableFactor::Derived { subquery, .. } = &mut join.relation else {
        unreachable!()
    };
    let ast::SetExpr::Select(right) = subquery.body.as_mut() else {
        unreachable!()
    };
    let ast::TableFactor::Table { name, .. } = &mut right.from[0].relation else {
        unreachable!()
    };
    *name = ast::ObjectName::from(vec![ident(&lookup.relationship.right.id)]);
    right.projection = lookup
        .relationship
        .keys
        .iter()
        .enumerate()
        .map(|(i, (_, key))| ast::SelectItem::ExprWithAlias {
            expr: sql_field(key),
            alias: ident(&format!("__semantic_key_{i}")),
        })
        .collect();
    right.projection.push(ast::SelectItem::ExprWithAlias {
        expr: sql_aggregate(&value_aggregate(lookup), parameters),
        alias: ident("__semantic_value"),
    });
    right.group_by = ast::GroupByExpr::Expressions(
        lookup
            .relationship
            .keys
            .iter()
            .map(|(_, key)| sql_field(key))
            .collect(),
        vec![],
    );
    right.selection = lookup
        .relationship
        .predicate
        .as_ref()
        .map(|predicate| sql_predicate(predicate, parameters));
    let on = lookup
        .relationship
        .keys
        .iter()
        .enumerate()
        .map(|(i, (left, _))| {
            let right = ast::Expr::CompoundIdentifier(vec![
                ident("rhs"),
                ident(&format!("__semantic_key_{i}")),
            ]);
            if lookup.relationship.null_keys_match {
                ast::Expr::IsNotDistinctFrom(Box::new(sql_field(left)), Box::new(right))
            } else {
                binary(sql_field(left), ast::BinaryOperator::Eq, right)
            }
        })
        .reduce(|a, b| binary(a, ast::BinaryOperator::And, b))
        .expect("bound keys");
    join.join_operator = match lookup.missing {
        MissingMatch::Null => ast::JoinOperator::LeftOuter(ast::JoinConstraint::On(on)),
        MissingMatch::Exclude => ast::JoinOperator::Inner(ast::JoinConstraint::On(on)),
    };
    select.from[0].joins.push(join);
    wrap_query(query);
}
fn value_aggregate(lookup: &BoundLookup) -> Aggregation {
    Aggregation {
        function: AggregateFunction::Min,
        field: Some(lookup.value.clone()),
        distinct: false,
        output: lookup.output.clone(),
        filter: None,
    }
}
pub(super) async fn plan_lookup(
    input: DataFrame,
    engine: &Engine,
    lookup: &BoundLookup,
) -> datafusion::error::Result<DataFrame> {
    let input = input.alias("src")?;
    let mut columns: Vec<_> = input
        .schema()
        .fields()
        .iter()
        .map(|field| Expr::Column(Column::new(Some("src"), field.name())))
        .collect();
    let mut right = engine
        .plan_generated_sql(&related_scan(&lookup.relationship, Some(&lookup.value)))
        .await
        .map_err(|_| {
            datafusion::error::DataFusionError::Plan("lookup endpoint planning failed".into())
        })?
        .alias("src")?;
    if let Some(predicate) = &lookup.relationship.predicate {
        right = right.filter(df_predicate(predicate))?;
    }
    right = right.aggregate(
        lookup
            .relationship
            .keys
            .iter()
            .enumerate()
            .map(|(i, (_, key))| df_field(key).alias(format!("__semantic_key_{i}")))
            .collect::<Vec<_>>(),
        vec![
            df_aggregate(&value_aggregate(lookup)).alias("__semantic_value"),
            datafusion::functions_aggregate::count::count_udaf()
                .call(vec![lit(1i64)])
                .alias("__semantic_count"),
        ],
    )?;
    right = right
        .filter(
            semantic_engine::semantic_assert_single_v1()
                .call(vec![Expr::Column(Column::from_name("__semantic_count"))]),
        )?
        .alias("rhs")?;
    let on = lookup
        .relationship
        .keys
        .iter()
        .enumerate()
        .map(|(i, (left, _))| {
            let right = Expr::Column(Column::new(Some("rhs"), format!("__semantic_key_{i}")));
            if lookup.relationship.null_keys_match {
                Expr::BinaryExpr(datafusion::logical_expr::expr::BinaryExpr::new(
                    Box::new(df_field(left)),
                    datafusion::logical_expr::Operator::IsNotDistinctFrom,
                    Box::new(right),
                ))
            } else {
                df_field(left).eq(right)
            }
        });
    columns.push(
        Expr::Column(Column::new(Some("rhs"), "__semantic_value"))
            .alias(lookup.output.field.name()),
    );
    input
        .join_on(
            right,
            match lookup.missing {
                MissingMatch::Null => datafusion::logical_expr::JoinType::Left,
                MissingMatch::Exclude => datafusion::logical_expr::JoinType::Inner,
            },
            on,
        )?
        .select(columns)
}
