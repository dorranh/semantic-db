use super::*;
use semantic_plan::typed::MissingMatch;

pub(super) fn emit_lookup(
    query: &mut ast::Query,
    lookup: &BoundLookup,
    parameters: &mut Vec<Literal>,
) {
    if lookup.as_of.is_some() {
        emit_as_of_lookup(query, lookup, parameters);
        return;
    }
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
    if let Some(extra) = &lookup.extra_value {
        select.projection.push(ast::SelectItem::ExprWithAlias {
            expr: ast::Expr::CompoundIdentifier(vec![ident("rhs"), ident("__semantic_extra")]),
            alias: ident(extra.output.field.name()),
        });
    }
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
    if let Some(extra) = &lookup.extra_value {
        right.projection.push(ast::SelectItem::ExprWithAlias {
            expr: sql_aggregate(&extra_aggregate(extra), parameters),
            alias: ident("__semantic_extra"),
        });
    }
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

fn emit_as_of_lookup(query: &mut ast::Query, lookup: &BoundLookup, parameters: &mut Vec<Literal>) {
    let as_of = lookup.as_of.as_ref().expect("temporal lookup");
    let row_id = format!("{}_as_of_row", lookup.output.field.name());
    let present = format!("{}_as_of_present", lookup.output.field.name());
    let count = format!("{}_as_of_count", lookup.output.field.name());
    let chosen = format!("{}_as_of_chosen", lookup.output.field.name());
    let qualified =
        |alias: &str, name: &str| ast::Expr::CompoundIdentifier(vec![ident(alias), ident(name)]);

    // Give every source occurrence an identity before the join. Equal source
    // rows remain separate and therefore keep their original multiplicity.
    let ast::SetExpr::Select(input) = query.body.as_mut() else {
        unreachable!()
    };
    input.projection = vec![ast::SelectItem::Wildcard(Default::default())];
    wrap_query(query);
    let mut numbered = Parser::parse_sql(
        &GenericDialect {},
        "SELECT src.*, row_number() OVER () AS n FROM t AS src",
    )
    .expect("static temporal occurrence skeleton");
    let ast::Statement::Query(numbered) = numbered.remove(0) else {
        unreachable!()
    };
    let ast::SetExpr::Select(numbered) = *numbered.body else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = numbered.projection;
    let ast::SelectItem::ExprWithAlias { alias, .. } = &mut select.projection[1] else {
        unreachable!()
    };
    *alias = ident(&row_id);

    wrap_query(query);
    let mut joined = Parser::parse_sql(
        &GenericDialect {},
        "SELECT src.*, rhs.v AS v, rhs.t AS p FROM t AS src LEFT JOIN t AS rhs ON true",
    )
    .expect("static temporal join skeleton");
    let ast::Statement::Query(joined) = joined.remove(0) else {
        unreachable!()
    };
    let ast::SetExpr::Select(mut joined) = *joined.body else {
        unreachable!()
    };
    let mut join = joined.from[0].joins.remove(0);
    let ast::TableFactor::Table { name, .. } = &mut join.relation else {
        unreachable!()
    };
    *name = ast::ObjectName::from(vec![ident(&lookup.relationship.right.id)]);
    let mut comparisons = lookup
        .relationship
        .keys
        .iter()
        .map(|(left, right)| {
            if lookup.relationship.null_keys_match {
                ast::Expr::IsNotDistinctFrom(
                    Box::new(sql_field(left)),
                    Box::new(sql_field_aliased(right, "rhs")),
                )
            } else {
                binary(
                    sql_field(left),
                    ast::BinaryOperator::Eq,
                    sql_field_aliased(right, "rhs"),
                )
            }
        })
        .collect::<Vec<_>>();
    comparisons.push(binary(
        sql_field_aliased(&as_of.valid_from, "rhs"),
        ast::BinaryOperator::LtEq,
        sql_field(&as_of.fact_time),
    ));
    comparisons.push(binary(
        sql_field(&as_of.fact_time),
        ast::BinaryOperator::Lt,
        sql_field_aliased(&as_of.valid_to, "rhs"),
    ));
    if let Some(policy) = &lookup.relationship.predicate {
        comparisons.push(sql_predicate_aliased(policy, parameters, "rhs"));
    }
    let on = comparisons
        .into_iter()
        .reduce(|a, b| binary(a, ast::BinaryOperator::And, b))
        .expect("bound temporal predicates");
    join.join_operator = ast::JoinOperator::LeftOuter(ast::JoinConstraint::On(on));
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = joined.projection;
    select.projection[1] = ast::SelectItem::ExprWithAlias {
        expr: sql_field_aliased(&lookup.value, "rhs"),
        alias: ident(lookup.output.field.name()),
    };
    select.projection[2] = ast::SelectItem::ExprWithAlias {
        expr: sql_field_aliased(&as_of.valid_from, "rhs"),
        alias: ident(&present),
    };
    if let Some(extra) = &lookup.extra_value {
        select.projection.push(ast::SelectItem::ExprWithAlias {
            expr: sql_field_aliased(&extra.value, "rhs"),
            alias: ident(extra.output.field.name()),
        });
    }
    select.from[0].joins.push(join);

    // Compute a per-occurrence match count before selecting one joined row.
    wrap_query(query);
    let mut windowed = Parser::parse_sql(&GenericDialect {},
        "SELECT src.*, count(src.p) OVER (PARTITION BY src.n) AS c, row_number() OVER (PARTITION BY src.n) AS r FROM t AS src")
        .expect("static temporal window skeleton");
    let ast::Statement::Query(windowed) = windowed.remove(0) else {
        unreachable!()
    };
    let ast::SetExpr::Select(windowed) = *windowed.body else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = windowed.projection;
    for (position, name) in [(1, &count), (2, &chosen)] {
        let ast::SelectItem::ExprWithAlias {
            expr: ast::Expr::Function(function),
            alias,
        } = &mut select.projection[position]
        else {
            unreachable!()
        };
        *alias = ident(name);
        let Some(ast::WindowType::WindowSpec(window)) = &mut function.over else {
            unreachable!()
        };
        window.partition_by = vec![qualified("src", &row_id)];
        if position == 1 {
            let ast::FunctionArguments::List(arguments) = &mut function.args else {
                unreachable!()
            };
            arguments.args = vec![ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(
                qualified("src", &present),
            ))];
        }
    }

    wrap_query(query);
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = vec![ast::SelectItem::Wildcard(Default::default())];
    let mut checked = Parser::parse_sql(&GenericDialect {}, "SELECT semantic_assert_single_v1(x)")
        .expect("static temporal check skeleton");
    let ast::Statement::Query(checked) = checked.remove(0) else {
        unreachable!()
    };
    let ast::SetExpr::Select(checked) = *checked.body else {
        unreachable!()
    };
    let ast::SelectItem::UnnamedExpr(ast::Expr::Function(mut check)) =
        checked.projection.into_iter().next().expect("check")
    else {
        unreachable!()
    };
    let ast::FunctionArguments::List(arguments) = &mut check.args else {
        unreachable!()
    };
    arguments.args = vec![ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(
        qualified("src", &count),
    ))];
    let mut predicates = vec![
        ast::Expr::Function(check),
        binary(
            qualified("src", &chosen),
            ast::BinaryOperator::Eq,
            ast::Expr::Value(ast::Value::Number("1".into(), false).into()),
        ),
    ];
    if lookup.missing == MissingMatch::Exclude {
        predicates.push(binary(
            qualified("src", &count),
            ast::BinaryOperator::Gt,
            ast::Expr::Value(ast::Value::Number("0".into(), false).into()),
        ));
    }
    select.selection = predicates
        .into_iter()
        .reduce(|a, b| binary(a, ast::BinaryOperator::And, b));
    wrap_query(query);
}
fn value_aggregate(lookup: &BoundLookup) -> Aggregation {
    Aggregation {
        function: AggregateFunction::Min,
        mean_state: false,
        weighted: None,
        zero_finalizer: None,
        exact_distinct: false,
        snapshot: None,
        field: Some(lookup.value.clone()),
        distinct: false,
        output: lookup.output.clone(),
        filter: None,
    }
}
fn extra_aggregate(extra: &BoundLookupExtra) -> Aggregation {
    Aggregation {
        function: AggregateFunction::Min,
        mean_state: false,
        weighted: None,
        zero_finalizer: None,
        exact_distinct: false,
        snapshot: None,
        field: Some(extra.value.clone()),
        distinct: false,
        output: extra.output.clone(),
        filter: None,
    }
}
pub(super) async fn plan_lookup(
    input: DataFrame,
    engine: &Engine,
    lookup: &BoundLookup,
) -> datafusion::error::Result<DataFrame> {
    if lookup.as_of.is_some() {
        return plan_as_of_lookup(input, engine, lookup).await;
    }
    let input = input.alias("src")?;
    let mut columns: Vec<_> = input
        .schema()
        .fields()
        .iter()
        .map(|field| Expr::Column(Column::new(Some("src"), field.name())))
        .collect();
    let mut right_fields = vec![&lookup.value];
    if let Some(extra) = &lookup.extra_value {
        right_fields.push(&extra.value);
    }
    let mut right = engine
        .plan_generated_sql(&related_scan(&lookup.relationship, &right_fields))
        .await
        .map_err(|_| {
            datafusion::error::DataFusionError::Plan("lookup endpoint planning failed".into())
        })?
        .alias("src")?;
    if let Some(predicate) = &lookup.relationship.predicate {
        right = right.filter(df_predicate(predicate))?;
    }
    let mut aggregates = vec![
        df_aggregate(&value_aggregate(lookup)).alias("__semantic_value"),
        datafusion::functions_aggregate::count::count_udaf()
            .call(vec![lit(1i64)])
            .alias("__semantic_count"),
    ];
    if let Some(extra) = &lookup.extra_value {
        aggregates.push(df_aggregate(&extra_aggregate(extra)).alias("__semantic_extra"));
    }
    right = right.aggregate(
        lookup
            .relationship
            .keys
            .iter()
            .enumerate()
            .map(|(i, (_, key))| df_field(key).alias(format!("__semantic_key_{i}")))
            .collect::<Vec<_>>(),
        aggregates,
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
    if let Some(extra) = &lookup.extra_value {
        columns.push(
            Expr::Column(Column::new(Some("rhs"), "__semantic_extra"))
                .alias(extra.output.field.name()),
        );
    }
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

async fn plan_as_of_lookup(
    input: DataFrame,
    engine: &Engine,
    lookup: &BoundLookup,
) -> datafusion::error::Result<DataFrame> {
    use datafusion::logical_expr::{
        JoinType,
        expr::{WindowFunction, WindowFunctionDefinition},
    };
    let as_of = lookup.as_of.as_ref().expect("temporal lookup");
    let mut row_id = "__semantic_as_of_row".to_owned();
    while input.schema().field_with_unqualified_name(&row_id).is_ok() {
        row_id.push('_');
    }
    let original_columns = input
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect::<Vec<_>>();
    let row_number = Expr::WindowFunction(Box::new(WindowFunction::new(
        WindowFunctionDefinition::WindowUDF(
            datafusion::functions_window::row_number::row_number_udwf(),
        ),
        vec![],
    )));
    let input = input.with_column(&row_id, row_number)?.alias("src")?;
    let mut right_fields = vec![&lookup.value, &as_of.valid_from, &as_of.valid_to];
    if let Some(extra) = &lookup.extra_value {
        right_fields.push(&extra.value);
    }
    let mut right = engine
        .plan_generated_sql(&related_scan(&lookup.relationship, &right_fields))
        .await
        .map_err(|_| {
            datafusion::error::DataFusionError::Plan("as-of endpoint planning failed".into())
        })?;
    if let Some(predicate) = &lookup.relationship.predicate {
        right = right.alias("src")?.filter(df_predicate(predicate))?;
    }
    let mut present = "__semantic_as_of_present".to_owned();
    while right.schema().field_with_unqualified_name(&present).is_ok() {
        present.push('_');
    }
    let right = right.with_column(&present, lit(1i64))?.alias("rhs")?;
    let mut on = lookup
        .relationship
        .keys
        .iter()
        .map(|(left, right)| {
            let rhs = Expr::Column(Column::new(Some("rhs"), right.field.name()));
            if lookup.relationship.null_keys_match {
                Expr::BinaryExpr(datafusion::logical_expr::expr::BinaryExpr::new(
                    Box::new(df_field(left)),
                    datafusion::logical_expr::Operator::IsNotDistinctFrom,
                    Box::new(rhs),
                ))
            } else {
                df_field(left).eq(rhs)
            }
        })
        .collect::<Vec<_>>();
    let fact_time = df_field(&as_of.fact_time);
    on.push(
        Expr::Column(Column::new(Some("rhs"), as_of.valid_from.field.name()))
            .lt_eq(fact_time.clone()),
    );
    on.push(fact_time.lt(Expr::Column(Column::new(
        Some("rhs"),
        as_of.valid_to.field.name(),
    ))));
    let joined = input.join_on(right, JoinType::Left, on)?;
    let group_names = original_columns
        .iter()
        .cloned()
        .chain(std::iter::once(row_id.clone()))
        .collect::<Vec<_>>();
    let mut count_name = "__semantic_as_of_count".to_owned();
    while group_names.iter().any(|name| name == &count_name)
        || lookup.output.field.name() == &count_name
    {
        count_name.push('_');
    }
    let mut aggregates = vec![
        datafusion::functions_aggregate::min_max::min_udaf()
            .call(vec![Expr::Column(Column::new(
                Some("rhs"),
                lookup.value.field.name(),
            ))])
            .alias(lookup.output.field.name()),
        datafusion::functions_aggregate::count::count_udaf()
            .call(vec![Expr::Column(Column::new(Some("rhs"), &present))])
            .alias(&count_name),
    ];
    if let Some(extra) = &lookup.extra_value {
        aggregates.push(
            datafusion::functions_aggregate::min_max::min_udaf()
                .call(vec![Expr::Column(Column::new(
                    Some("rhs"),
                    extra.value.field.name(),
                ))])
                .alias(extra.output.field.name()),
        );
    }
    let grouped = joined.aggregate(
        group_names
            .iter()
            .map(|name| Expr::Column(Column::new(Some("src"), name)))
            .collect::<Vec<_>>(),
        aggregates,
    )?;
    let count = Expr::Column(Column::from_name(&count_name));
    let mut checked =
        grouped.filter(semantic_engine::semantic_assert_single_v1().call(vec![count.clone()]))?;
    if lookup.missing == MissingMatch::Exclude {
        checked = checked.filter(count.gt(lit(0i64)))?;
    }
    let mut output = original_columns
        .iter()
        .map(|name| Expr::Column(Column::from_name(name)))
        .collect::<Vec<_>>();
    output.push(Expr::Column(Column::from_name(lookup.output.field.name())));
    if let Some(extra) = &lookup.extra_value {
        output.push(Expr::Column(Column::from_name(extra.output.field.name())));
    }
    checked.select(output)
}
