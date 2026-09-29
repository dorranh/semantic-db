use std::collections::{BTreeMap, BTreeSet};

use datafusion::{
    common::{Column, ScalarValue},
    dataframe::DataFrame,
    logical_expr::{Expr, lit},
    sql::sqlparser::{ast, dialect::GenericDialect, parser::Parser},
};
use semantic_catalog::ObjectRef;
use semantic_engine::Engine;
use semantic_plan::typed::{
    AggregateFunction, Comparison, Direction, ExistenceMode, Literal, NullOrder, OutputFilterStage,
    ZeroDivision,
};
use serde::Serialize;

use super::{CompileDiagnostic, bind::*, diagnostic};

/// Internal, version-tagged relational artifact. No public deserialization or
/// mutation: replay starts from the untrusted proposal and validates again.
#[derive(Debug, Clone, Serialize)]
pub struct RelationalPlan {
    version: u32,
    comparison_profile: &'static str,
    nodes: Vec<Node>,
}
#[derive(Debug, Clone, Serialize)]
struct Node {
    id: usize,
    input: Option<usize>,
    requirements: Vec<String>,
    operator: Operator,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Operator {
    Lookup {
        lookup: Box<BoundLookup>,
    },
    Window {
        windows: Vec<WindowExpression>,
    },
    Derive {
        ratios: Vec<RatioExpression>,
    },
    Related {
        relationship: BoundRelationship,
    },
    Aggregate {
        groups: Vec<Grouping>,
        aggregates: Vec<Aggregation>,
    },
    Scan {
        relation: ObjectRef,
        instance: String,
    },
    Filter {
        predicate: BoundPredicate,
    },
    Sort {
        keys: Vec<SortKey>,
    },
    Project {
        columns: Vec<Projection>,
    },
    Fetch {
        count: u32,
    },
}
#[derive(Debug, Clone, Serialize)]
struct SortKey {
    field: BoundField,
    direction: Direction,
    nulls: NullOrder,
}
#[derive(Debug, Clone, Serialize)]
struct Projection {
    field: BoundField,
    alias: String,
}

#[derive(Debug, Clone, Serialize)]
struct Grouping {
    field: BoundField,
    output: BoundField,
}
#[derive(Debug, Clone, Serialize)]
struct Aggregation {
    function: AggregateFunction,
    field: Option<BoundField>,
    distinct: bool,
    output: BoundField,
    filter: Option<BoundPredicate>,
}

#[derive(Debug, Clone, Serialize)]
struct RatioExpression {
    numerator: BoundField,
    denominator: BoundField,
    zero: ZeroDivision,
    output: BoundField,
}

#[derive(Debug, Clone, Serialize)]
struct WindowExpression {
    window: BoundWindow,
    output: BoundField,
}
mod lookup;
mod window;

#[tracing::instrument(name = "semantic.lower", skip_all)]
pub(super) fn lower(bound: &BoundQuery) -> Result<RelationalPlan, CompileDiagnostic> {
    let mut plan = RelationalPlan {
        version: 1,
        comparison_profile: "datafusion-55/sql-null-logic/utf8-binary",
        nodes: Vec::new(),
    };
    plan.push(
        Vec::new(),
        Operator::Scan {
            relation: bound.input.clone(),
            instance: bound.instance.clone(),
        },
    );
    let mut windows = Vec::new();
    let mut window_ids = Vec::new();
    let mut ratios = Vec::new();
    let mut ratio_ids = Vec::new();
    let mut groups = Vec::new();
    let mut aggregates = Vec::new();
    let mut aggregate_ids = Vec::new();
    let mut projections = Vec::new();
    let mut projection_ids = Vec::new();
    let mut ordering = Vec::new();
    let mut ordering_ids = Vec::new();
    let mut limit = None;
    let mut aggregate_filters = Vec::new();
    let mut window_filters = Vec::new();
    for requirement in &bound.requirements {
        match &requirement.operation {
            BoundOperation::FilterOutput { stage, predicate } => {
                let filters = match stage {
                    OutputFilterStage::AfterAggregate => &mut aggregate_filters,
                    OutputFilterStage::AfterWindow => &mut window_filters,
                };
                filters.push((requirement.id.clone(), predicate.clone()));
            }
            BoundOperation::Lookup {
                lookup,
                alias,
                group_output,
            } => {
                plan.push(
                    vec![requirement.id.clone()],
                    Operator::Lookup {
                        lookup: lookup.clone(),
                    },
                );
                projections.push(Projection {
                    field: group_output.as_ref().unwrap_or(&lookup.output).clone(),
                    alias: alias.clone(),
                });
                if let Some(output) = group_output {
                    groups.push(Grouping {
                        field: lookup.output.clone(),
                        output: output.clone(),
                    });
                }
            }
            BoundOperation::Window {
                window,
                alias,
                output,
            } => {
                window_ids.push(requirement.id.clone());
                windows.push(WindowExpression {
                    window: window.clone(),
                    output: output.clone(),
                });
                projections.push(Projection {
                    field: output.clone(),
                    alias: alias.clone(),
                });
            }
            BoundOperation::Ratio {
                numerator,
                denominator,
                zero,
                alias,
                output,
            } => {
                let mut component = |input: &BoundRatioInput, suffix: &str| {
                    let field = BoundField {
                        instance: "$output".into(),
                        field: semantic_catalog::Field::new(
                            format!("{}_{suffix}", output.field.name()),
                            semantic_catalog::DataType::Int64,
                            true,
                        ),
                    };
                    aggregates.push(Aggregation {
                        function: input.function,
                        field: input.field.clone(),
                        distinct: input.distinct,
                        filter: input.filter.clone(),
                        output: field.clone(),
                    });
                    field
                };
                let numerator = component(numerator, "numerator");
                let denominator = component(denominator, "denominator");
                ratio_ids.push(requirement.id.clone());
                ratios.push(RatioExpression {
                    numerator,
                    denominator,
                    zero: *zero,
                    output: output.clone(),
                });
                projections.push(Projection {
                    field: output.clone(),
                    alias: alias.clone(),
                });
            }
            BoundOperation::Related { relationship } => plan.push(
                vec![requirement.id.clone()],
                Operator::Related {
                    relationship: relationship.clone(),
                },
            ),
            BoundOperation::Group {
                field,
                alias,
                output,
            } => {
                aggregate_ids.push(requirement.id.clone());
                groups.push(Grouping {
                    field: field.clone(),
                    output: output.clone(),
                });
                projections.push(Projection {
                    field: output.clone(),
                    alias: alias.clone(),
                });
            }
            BoundOperation::Aggregate {
                function,
                field,
                distinct,
                alias,
                output,
                filter,
            } => {
                aggregate_ids.push(requirement.id.clone());
                aggregates.push(Aggregation {
                    function: *function,
                    field: field.clone(),
                    distinct: *distinct,
                    output: output.clone(),
                    filter: filter.clone(),
                });
                projections.push(Projection {
                    field: output.clone(),
                    alias: alias.clone(),
                });
            }
            BoundOperation::Filter { predicate }
            | BoundOperation::CalendarFilter { predicate, .. } => plan.push(
                vec![requirement.id.clone()],
                Operator::Filter {
                    predicate: predicate.clone(),
                },
            ),
            BoundOperation::Project { field, alias } => {
                projection_ids.push(requirement.id.clone());
                projections.push(Projection {
                    field: field.clone(),
                    alias: alias.clone(),
                });
            }
            BoundOperation::Order {
                field,
                direction,
                nulls,
            } => {
                ordering_ids.push(requirement.id.clone());
                ordering.push(SortKey {
                    field: field.clone(),
                    direction: *direction,
                    nulls: *nulls,
                });
            }
            BoundOperation::Limit { count } => limit = Some((requirement.id.clone(), *count)),
        }
    }
    if !groups.is_empty() || !aggregates.is_empty() {
        plan.push(aggregate_ids, Operator::Aggregate { groups, aggregates });
    }
    if !ratios.is_empty() {
        plan.push(ratio_ids, Operator::Derive { ratios });
    }
    for (id, predicate) in aggregate_filters {
        plan.push(vec![id], Operator::Filter { predicate });
    }
    if !windows.is_empty() {
        plan.push(window_ids, Operator::Window { windows });
    }
    for (id, predicate) in window_filters {
        plan.push(vec![id], Operator::Filter { predicate });
    }
    if !ordering.is_empty() {
        plan.push(ordering_ids, Operator::Sort { keys: ordering });
    }
    plan.push(
        projection_ids,
        Operator::Project {
            columns: projections,
        },
    );
    if let Some((id, count)) = limit {
        plan.push(vec![id], Operator::Fetch { count });
    }
    // Each accepted requirement must survive exactly once; a missing predicate
    // cannot be excused by a field appearing in the output/evidence.
    let expected: BTreeSet<_> = bound.requirements.iter().map(|r| r.id.as_str()).collect();
    let mut actual = BTreeSet::new();
    for (index, node) in plan.nodes.iter().enumerate() {
        if node.id != index || node.input != index.checked_sub(1) {
            return Err(diagnostic(
                "invalid_relational_plan",
                "Invalid relational node linkage",
            ));
        }
        for id in &node.requirements {
            if !actual.insert(id.as_str()) {
                return Err(diagnostic(
                    "requirement_coverage",
                    "A requirement was lowered more than once",
                ));
            }
        }
    }
    if actual != expected {
        return Err(diagnostic(
            "requirement_coverage",
            "Lowering did not preserve all requirements",
        ));
    }
    Ok(plan)
}

#[derive(Debug, Clone, Serialize)]
pub struct SqlArtifact {
    dialect: &'static str,
    snapshot_id: String,
    statement: String,
    parameters: Vec<Literal>,
}
impl SqlArtifact {
    pub(super) fn generated(
        snapshot_id: &str,
        statement: String,
        parameters: Vec<Literal>,
    ) -> Self {
        Self {
            dialect: "datafusion-55",
            snapshot_id: snapshot_id.into(),
            statement,
            parameters,
        }
    }
    pub fn statement(&self) -> &str {
        &self.statement
    }
    pub fn parameters(&self) -> &[Literal] {
        &self.parameters
    }
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }
    pub(super) fn values(&self) -> Vec<ScalarValue> {
        self.parameters.iter().map(scalar).collect()
    }
}

impl RelationalPlan {
    fn push(&mut self, requirements: Vec<String>, operator: Operator) {
        let id = self.nodes.len();
        self.nodes.push(Node {
            id,
            input: id.checked_sub(1),
            requirements,
            operator,
        });
    }
    pub(super) fn emit(&self, snapshot_id: &str) -> SqlArtifact {
        // Parse only a compiler-owned skeleton; every dynamic identifier and
        // expression is then constructed as an AST node, never parsed as SQL.
        let mut statements = Parser::parse_sql(&GenericDialect {}, "SELECT x FROM t AS src")
            .expect("static SQL skeleton");
        let ast::Statement::Query(query) = &mut statements[0] else {
            unreachable!()
        };
        let mut parameters = Vec::new();
        for node in &self.nodes {
            let ast::SetExpr::Select(select) = query.body.as_mut() else {
                unreachable!()
            };
            match &node.operator {
                Operator::Lookup { lookup } => lookup::emit_lookup(query, lookup, &mut parameters),
                Operator::Window { windows } => {
                    select.projection = vec![ast::SelectItem::Wildcard(Default::default())];
                    select.projection.extend(windows.iter().map(|window| {
                        ast::SelectItem::ExprWithAlias {
                            expr: window::sql_window(&window.window),
                            alias: ident(window.output.field.name()),
                        }
                    }));
                    wrap_query(query);
                }
                Operator::Derive { ratios } => {
                    select.projection = vec![ast::SelectItem::Wildcard(Default::default())];
                    select.projection.extend(ratios.iter().map(|ratio| {
                        ast::SelectItem::ExprWithAlias {
                            expr: sql_ratio(ratio),
                            alias: ident(ratio.output.field.name()),
                        }
                    }));
                    wrap_query(query);
                }
                Operator::Related { relationship } => {
                    let expr = sql_relationship(relationship, &mut parameters);
                    select.selection = Some(match select.selection.take() {
                        None => expr,
                        Some(left) => binary(left, ast::BinaryOperator::And, expr),
                    });
                }
                Operator::Aggregate { groups, aggregates } => {
                    select.group_by = ast::GroupByExpr::Expressions(
                        groups.iter().map(|g| sql_field(&g.field)).collect(),
                        vec![],
                    );
                    select.projection = groups
                        .iter()
                        .map(|g| ast::SelectItem::ExprWithAlias {
                            expr: sql_field(&g.field),
                            alias: ident(g.output.field.name()),
                        })
                        .chain(aggregates.iter().map(|a| ast::SelectItem::ExprWithAlias {
                            expr: sql_aggregate(a, &mut parameters),
                            alias: ident(a.output.field.name()),
                        }))
                        .collect();
                    wrap_query(query);
                }
                Operator::Scan { relation, .. } => {
                    let ast::TableFactor::Table { name, .. } = &mut select.from[0].relation else {
                        unreachable!()
                    };
                    *name = ast::ObjectName::from(vec![ident(&relation.id)]);
                }
                Operator::Filter { predicate } => {
                    let expr = sql_predicate(predicate, &mut parameters);
                    select.selection = Some(match select.selection.take() {
                        None => expr,
                        Some(left) => binary(left, ast::BinaryOperator::And, expr),
                    });
                }
                Operator::Project { columns } => {
                    select.projection = columns
                        .iter()
                        .map(|p| ast::SelectItem::ExprWithAlias {
                            expr: sql_field(&p.field),
                            alias: ident(&p.alias),
                        })
                        .collect()
                }
                Operator::Sort { keys } => {
                    query.order_by = Some(ast::OrderBy {
                        kind: ast::OrderByKind::Expressions(
                            keys.iter()
                                .map(|key| ast::OrderByExpr {
                                    expr: sql_field(&key.field),
                                    options: ast::OrderByOptions {
                                        asc: Some(key.direction == Direction::Asc),
                                        nulls_first: Some(key.nulls == NullOrder::First),
                                    },
                                    with_fill: None,
                                })
                                .collect(),
                        ),
                        interpolate: None,
                    })
                }
                Operator::Fetch { count } => {
                    query.limit_clause = Some(ast::LimitClause::LimitOffset {
                        limit: Some(ast::Expr::Value(
                            ast::Value::Number(count.to_string(), false).into(),
                        )),
                        offset: None,
                        limit_by: Vec::new(),
                    })
                }
            }
        }
        isolate_sort_aliases(query);
        SqlArtifact {
            dialect: "datafusion-55",
            snapshot_id: snapshot_id.into(),
            statement: statements[0].to_string(),
            parameters,
        }
    }

    pub(super) async fn plan_direct(
        &self,
        engine: &Engine,
    ) -> Result<DataFrame, CompileDiagnostic> {
        let mut frame = None;
        for node in &self.nodes {
            if let Operator::Scan { relation, .. } = &node.operator {
                // Use the engine's registered-relation/read-only gate for the
                // scan. All subsequent expressions are from the checked IR.
                let sql = self.scan_sql(relation);
                frame = Some(
                    engine
                        .plan_generated_sql(&sql)
                        .await
                        .map_err(backend_error)?,
                );
                continue;
            }
            let input = frame.take().expect("verified scan-first plan");
            frame = Some(
                match &node.operator {
                    Operator::Scan { .. } => unreachable!(),
                    Operator::Lookup { lookup } => lookup::plan_lookup(input, engine, lookup).await,
                    Operator::Window { windows } => {
                        let mut output = input;
                        for window in windows {
                            output = output
                                .with_column(
                                    window.output.field.name(),
                                    window::df_window(&window.window),
                                )
                                .map_err(backend_error)?;
                        }
                        Ok(output)
                    }
                    Operator::Derive { ratios } => {
                        let mut output = input;
                        for ratio in ratios {
                            output = output
                                .with_column(
                                    ratio.output.field.name(),
                                    semantic_engine::semantic_ratio_i64_v1().call(vec![
                                        df_field(&ratio.numerator),
                                        df_field(&ratio.denominator),
                                        lit(ratio.zero == ZeroDivision::Zero),
                                    ]),
                                )
                                .map_err(backend_error)?;
                        }
                        Ok(output)
                    }
                    Operator::Related { relationship } => {
                        let mut right = engine
                            .plan_generated_sql(&related_scan(relationship, None))
                            .await
                            .map_err(backend_error)?;
                        if let Some(predicate) = &relationship.predicate {
                            right = right
                                .filter(df_predicate_aliased(predicate, "rhs"))
                                .map_err(backend_error)?;
                        }
                        let on = relationship.keys.iter().map(|(left, right)| {
                            if relationship.null_keys_match {
                                Expr::BinaryExpr(datafusion::logical_expr::expr::BinaryExpr::new(
                                    Box::new(df_field(left)),
                                    datafusion::logical_expr::Operator::IsNotDistinctFrom,
                                    Box::new(df_field_aliased(right, "rhs")),
                                ))
                            } else {
                                df_field(left).eq(df_field_aliased(right, "rhs"))
                            }
                        });
                        input.join_on(
                            right,
                            if relationship.mode == ExistenceMode::Exists {
                                datafusion::logical_expr::JoinType::LeftSemi
                            } else {
                                datafusion::logical_expr::JoinType::LeftAnti
                            },
                            on,
                        )
                    }
                    Operator::Aggregate { groups, aggregates } => input.aggregate(
                        groups
                            .iter()
                            .map(|g| df_field(&g.field).alias(g.output.field.name()))
                            .collect::<Vec<_>>(),
                        aggregates
                            .iter()
                            .map(|a| df_aggregate(a).alias(a.output.field.name()))
                            .collect::<Vec<_>>(),
                    ),
                    Operator::Filter { predicate } => input.filter(df_predicate(predicate)),
                    Operator::Sort { keys } => input.sort(
                        keys.iter()
                            .map(|k| {
                                df_field(&k.field).sort(
                                    k.direction == Direction::Asc,
                                    k.nulls == NullOrder::First,
                                )
                            })
                            .collect(),
                    ),
                    Operator::Project { columns } => input.select(
                        columns
                            .iter()
                            .map(|p| df_field(&p.field).alias(&p.alias))
                            .collect::<Vec<_>>(),
                    ),
                    Operator::Fetch { count } => input.limit(0, Some(*count as usize)),
                }
                .map_err(backend_error)?,
            );
        }
        let frame = frame.expect("verified nonempty plan");
        let columns = self
            .nodes
            .iter()
            .find_map(|n| {
                if let Operator::Project { columns } = &n.operator {
                    Some(columns)
                } else {
                    None
                }
            })
            .expect("verified projection");
        if frame.schema().fields().len() != columns.len()
            || columns
                .iter()
                .zip(frame.schema().fields())
                .any(|(expected, actual)| {
                    expected.alias != *actual.name()
                        || expected.field.field.data_type() != actual.data_type()
                        || expected.field.field.is_nullable() != actual.is_nullable()
                })
        {
            return Err(diagnostic(
                "output_contract",
                "Backend output does not match the bound projection contract",
            ));
        }
        Ok(frame)
    }

    /// Hydrate the scan with only used fields. A SELECT * intermediate makes
    /// DataFusion expand and normalize every field of a wide relation before
    /// our projection pass can discard them.
    fn scan_sql(&self, relation: &ObjectRef) -> String {
        fn predicate_fields<'a>(predicate: &'a BoundPredicate, fields: &mut BTreeSet<&'a str>) {
            match predicate {
                BoundPredicate::Compare { field, .. } | BoundPredicate::IsNull { field, .. } => {
                    if field.instance != "$output" {
                        fields.insert(field.field.name());
                    }
                }
                BoundPredicate::Not { predicate } | BoundPredicate::Mapped { predicate, .. } => {
                    predicate_fields(predicate, fields)
                }
                BoundPredicate::All { predicates } | BoundPredicate::Any { predicates } => {
                    for predicate in predicates {
                        predicate_fields(predicate, fields);
                    }
                }
            }
        }
        let mut fields = BTreeSet::new();
        for node in &self.nodes {
            match &node.operator {
                Operator::Lookup { lookup } => fields.extend(
                    lookup
                        .relationship
                        .keys
                        .iter()
                        .map(|(left, _)| left.field.name().as_str()),
                ),
                Operator::Window { windows } => {
                    for window in windows {
                        fields.extend(
                            window
                                .window
                                .input
                                .iter()
                                .chain(&window.window.partition_by)
                                .chain(window.window.order_by.iter().map(|o| &o.0))
                                .filter(|f| f.instance != "$output")
                                .map(|f| f.field.name().as_str()),
                        );
                    }
                }
                Operator::Related { relationship } => {
                    fields.extend(
                        relationship
                            .keys
                            .iter()
                            .map(|(left, _)| left.field.name().as_str()),
                    );
                }
                Operator::Aggregate { groups, aggregates } => {
                    fields.extend(
                        groups
                            .iter()
                            .filter(|g| g.field.instance != "$output")
                            .map(|g| g.field.field.name().as_str()),
                    );
                    for aggregate in aggregates {
                        if let Some(predicate) = &aggregate.filter {
                            predicate_fields(predicate, &mut fields);
                        }
                    }
                    fields.extend(
                        aggregates
                            .iter()
                            .filter_map(|a| a.field.as_ref().map(|f| f.field.name().as_str())),
                    );
                }
                Operator::Filter { predicate } => predicate_fields(predicate, &mut fields),
                Operator::Sort { keys } => fields.extend(
                    keys.iter()
                        .filter(|k| k.field.instance != "$output")
                        .map(|k| k.field.field.name().as_str()),
                ),
                Operator::Project { columns } => fields.extend(
                    columns
                        .iter()
                        .filter(|p| p.field.instance != "$output")
                        .map(|p| p.field.field.name().as_str()),
                ),
                _ => {}
            }
        }
        let mut statements = Parser::parse_sql(&GenericDialect {}, "SELECT x FROM t AS src")
            .expect("static scan skeleton");
        let ast::Statement::Query(query) = &mut statements[0] else {
            unreachable!()
        };
        let ast::SetExpr::Select(select) = query.body.as_mut() else {
            unreachable!()
        };
        let ast::TableFactor::Table { name, .. } = &mut select.from[0].relation else {
            unreachable!()
        };
        *name = ast::ObjectName::from(vec![ident(&relation.id)]);
        if fields.is_empty() {
            // COUNT(*) needs row multiplicity but no source columns.
            select.projection = vec![ast::SelectItem::UnnamedExpr(ast::Expr::Value(
                ast::Value::Number("1".into(), false).into(),
            ))];
            return statements[0].to_string();
        }
        select.projection = fields
            .into_iter()
            .map(|name| {
                ast::SelectItem::UnnamedExpr(ast::Expr::CompoundIdentifier(vec![
                    ident("src"),
                    ident(name),
                ]))
            })
            .collect();
        statements[0].to_string()
    }
}

/// DataFusion's SQL name resolver can treat a qualified source sort key as
/// ambiguous when an output alias shadows it. Introduce backend-local slots in
/// a subquery, disjoint from every output alias, so presentation names never
/// choose the sort expression. Filtering remains inside; ordering/fetch outside.
fn isolate_sort_aliases(query: &mut ast::Query) {
    let ast::SetExpr::Select(select) = query.body.as_ref() else {
        unreachable!()
    };
    let Some(ast::OrderBy {
        kind: ast::OrderByKind::Expressions(keys),
        ..
    }) = &query.order_by
    else {
        return;
    };
    let mut aliases = BTreeSet::new();
    let mut fields = BTreeSet::new();
    let mut shadowed = BTreeSet::new();
    for item in &select.projection {
        let ast::SelectItem::ExprWithAlias {
            expr: ast::Expr::CompoundIdentifier(parts),
            alias,
        } = item
        else {
            unreachable!()
        };
        aliases.insert(alias.value.clone());
        fields.insert(parts[1].value.clone());
        if parts[1].value != alias.value {
            shadowed.insert(alias.value.clone());
        }
    }
    let mut needs_slots = false;
    for key in keys {
        let ast::Expr::CompoundIdentifier(parts) = &key.expr else {
            unreachable!()
        };
        fields.insert(parts[1].value.clone());
        needs_slots |= shadowed.contains(&parts[1].value);
    }
    if !needs_slots {
        return;
    }
    let slots: BTreeMap<_, _> = fields
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let mut slot = format!("__semantic_slot_{index}");
            while aliases.contains(&slot) {
                slot.push('_');
            }
            (name, slot)
        })
        .collect();
    let mut inner = query.clone();
    inner.order_by = None;
    inner.limit_clause = None;
    let ast::SetExpr::Select(inner_select) = inner.body.as_mut() else {
        unreachable!()
    };
    inner_select.projection = slots
        .iter()
        .map(|(name, slot)| ast::SelectItem::ExprWithAlias {
            expr: ast::Expr::CompoundIdentifier(vec![ident("src"), ident(name)]),
            alias: ident(slot),
        })
        .collect();
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.selection = None;
    let mut skeleton =
        Parser::parse_sql(&GenericDialect {}, "SELECT x FROM (SELECT x FROM t) AS src")
            .expect("static subquery skeleton");
    let ast::Statement::Query(outer) = &mut skeleton[0] else {
        unreachable!()
    };
    let ast::SetExpr::Select(outer_select) = outer.body.as_mut() else {
        unreachable!()
    };
    select.from = std::mem::take(&mut outer_select.from);
    let ast::TableFactor::Derived { subquery, .. } = &mut select.from[0].relation else {
        unreachable!()
    };
    **subquery = inner;
    for item in &mut select.projection {
        let ast::SelectItem::ExprWithAlias {
            expr: ast::Expr::CompoundIdentifier(parts),
            ..
        } = item
        else {
            unreachable!()
        };
        parts[1] = ident(&slots[&parts[1].value]);
    }
    let Some(ast::OrderBy {
        kind: ast::OrderByKind::Expressions(keys),
        ..
    }) = &mut query.order_by
    else {
        unreachable!()
    };
    for key in keys {
        let ast::Expr::CompoundIdentifier(parts) = &mut key.expr else {
            unreachable!()
        };
        parts[1] = ident(&slots[&parts[1].value]);
    }
}

pub(super) fn backend_error(_: impl std::fmt::Display) -> CompileDiagnostic {
    // Backend errors can contain user literals or physical paths. The normal
    // decision record intentionally retains only this stable diagnostic.
    diagnostic(
        "backend_validation",
        "The engine rejected the deterministic plan",
    )
}
fn ident(name: &str) -> ast::Ident {
    ast::Ident::with_quote('"', name)
}
fn sql_field(field: &BoundField) -> ast::Expr {
    sql_field_aliased(field, "src")
}
fn sql_field_aliased(field: &BoundField, alias: &str) -> ast::Expr {
    ast::Expr::CompoundIdentifier(vec![ident(alias), ident(field.field.name())])
}
fn binary(left: ast::Expr, op: ast::BinaryOperator, right: ast::Expr) -> ast::Expr {
    ast::Expr::Nested(Box::new(ast::Expr::BinaryOp {
        left: Box::new(left),
        op,
        right: Box::new(right),
    }))
}
fn sql_predicate(predicate: &BoundPredicate, parameters: &mut Vec<Literal>) -> ast::Expr {
    sql_predicate_aliased(predicate, parameters, "src")
}
fn sql_predicate_aliased(
    predicate: &BoundPredicate,
    parameters: &mut Vec<Literal>,
    alias: &str,
) -> ast::Expr {
    match predicate {
        BoundPredicate::Mapped { predicate, .. } => {
            sql_predicate_aliased(predicate, parameters, alias)
        }
        BoundPredicate::Compare {
            field,
            operator,
            value,
        } => {
            parameters.push(value.clone());
            let value = ast::Expr::Cast {
                kind: ast::CastKind::Cast,
                expr: Box::new(ast::Expr::Value(
                    ast::Value::Placeholder(format!("${}", parameters.len())).into(),
                )),
                data_type: super::literal::sql_type(value),
                format: None,
                array: false,
            };
            let op = match operator {
                Comparison::Eq => ast::BinaryOperator::Eq,
                Comparison::NotEq => ast::BinaryOperator::NotEq,
                Comparison::Lt => ast::BinaryOperator::Lt,
                Comparison::LtEq => ast::BinaryOperator::LtEq,
                Comparison::Gt => ast::BinaryOperator::Gt,
                Comparison::GtEq => ast::BinaryOperator::GtEq,
            };
            binary(sql_field_aliased(field, alias), op, value)
        }
        BoundPredicate::IsNull { field, negated } => {
            if *negated {
                ast::Expr::IsNotNull(Box::new(sql_field_aliased(field, alias)))
            } else {
                ast::Expr::IsNull(Box::new(sql_field_aliased(field, alias)))
            }
        }
        BoundPredicate::Not { predicate } => ast::Expr::UnaryOp {
            op: ast::UnaryOperator::Not,
            expr: Box::new(ast::Expr::Nested(Box::new(sql_predicate_aliased(
                predicate, parameters, alias,
            )))),
        },
        BoundPredicate::All { predicates } | BoundPredicate::Any { predicates } => {
            let op = if matches!(predicate, BoundPredicate::All { .. }) {
                ast::BinaryOperator::And
            } else {
                ast::BinaryOperator::Or
            };
            predicates
                .iter()
                .map(|p| sql_predicate_aliased(p, parameters, alias))
                .reduce(|l, r| binary(l, op.clone(), r))
                .expect("validated nonempty boolean")
        }
    }
}
fn df_field(field: &BoundField) -> Expr {
    Expr::Column(Column::new(
        if field.instance == "$output" {
            None
        } else {
            Some("src")
        },
        field.field.name(),
    ))
}
pub(super) fn scalar(value: &Literal) -> ScalarValue {
    super::literal::checked_scalar(value).expect("bound literal is validated")
}

fn df_predicate(predicate: &BoundPredicate) -> Expr {
    df_predicate_aliased(predicate, "src")
}
fn df_field_aliased(field: &BoundField, alias: &str) -> Expr {
    Expr::Column(Column::new(
        (field.instance != "$output").then_some(alias),
        field.field.name(),
    ))
}
fn df_predicate_aliased(predicate: &BoundPredicate, alias: &str) -> Expr {
    match predicate {
        BoundPredicate::Mapped { predicate, .. } => df_predicate_aliased(predicate, alias),
        BoundPredicate::Compare {
            field,
            operator,
            value,
        } => {
            let field = df_field_aliased(field, alias);
            let value = lit(scalar(value));
            match operator {
                Comparison::Eq => field.eq(value),
                Comparison::NotEq => field.not_eq(value),
                Comparison::Lt => field.lt(value),
                Comparison::LtEq => field.lt_eq(value),
                Comparison::Gt => field.gt(value),
                Comparison::GtEq => field.gt_eq(value),
            }
        }
        BoundPredicate::IsNull { field, negated } => {
            if *negated {
                df_field_aliased(field, alias).is_not_null()
            } else {
                df_field_aliased(field, alias).is_null()
            }
        }
        BoundPredicate::Not { predicate } => !df_predicate_aliased(predicate, alias),
        BoundPredicate::All { predicates } | BoundPredicate::Any { predicates } => predicates
            .iter()
            .map(|p| df_predicate_aliased(p, alias))
            .reduce(|l, r| {
                if matches!(predicate, BoundPredicate::All { .. }) {
                    l.and(r)
                } else {
                    l.or(r)
                }
            })
            .expect("validated nonempty boolean"),
    }
}

fn wrap_query(query: &mut ast::Query) {
    let mut skeleton =
        Parser::parse_sql(&GenericDialect {}, "SELECT x FROM (SELECT x FROM t) AS src")
            .expect("static aggregate wrapper");
    let ast::Statement::Query(outer) = &mut skeleton[0] else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = outer.body.as_mut() else {
        unreachable!()
    };
    let ast::TableFactor::Derived { subquery, .. } = &mut select.from[0].relation else {
        unreachable!()
    };
    std::mem::swap(query, subquery);
    *query = *outer.clone();
}
fn sql_aggregate(aggregate: &Aggregation, parameters: &mut Vec<Literal>) -> ast::Expr {
    let mut skeleton = Parser::parse_sql(&GenericDialect {}, "SELECT count(x)")
        .expect("static aggregate skeleton");
    let ast::Statement::Query(query) = &mut skeleton[0] else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    let ast::SelectItem::UnnamedExpr(ast::Expr::Function(function)) = &mut select.projection[0]
    else {
        unreachable!()
    };
    function.name = ast::ObjectName::from(vec![ast::Ident::new(match aggregate.function {
        AggregateFunction::Count => "count",
        AggregateFunction::Sum => "semantic_sum_v1",
        AggregateFunction::Min => "min",
        AggregateFunction::Max => "max",
    })]);
    let ast::FunctionArguments::List(arguments) = &mut function.args else {
        unreachable!()
    };
    arguments.duplicate_treatment = aggregate
        .distinct
        .then_some(ast::DuplicateTreatment::Distinct);
    arguments.args = vec![ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(
        aggregate
            .field
            .as_ref()
            .map(sql_field)
            .unwrap_or_else(|| ast::Expr::Value(ast::Value::Number("1".into(), false).into())),
    ))];
    function.filter = aggregate
        .filter
        .as_ref()
        .map(|p| Box::new(sql_predicate(p, parameters)));
    ast::Expr::Function(function.clone())
}
fn df_aggregate(aggregate: &Aggregation) -> Expr {
    use datafusion::functions_aggregate::{count, min_max};
    let function = match aggregate.function {
        AggregateFunction::Count => count::count_udaf(),
        AggregateFunction::Sum => semantic_engine::semantic_sum_v1(),
        AggregateFunction::Min => min_max::min_udaf(),
        AggregateFunction::Max => min_max::max_udaf(),
    };
    Expr::AggregateFunction(datafusion::logical_expr::expr::AggregateFunction::new_udf(
        function,
        vec![
            aggregate
                .field
                .as_ref()
                .map(df_field)
                .unwrap_or_else(|| lit(1i64)),
        ],
        aggregate.distinct,
        aggregate.filter.as_ref().map(|p| Box::new(df_predicate(p))),
        vec![],
        None,
    ))
}

fn sql_relationship(relationship: &BoundRelationship, parameters: &mut Vec<Literal>) -> ast::Expr {
    let mut skeleton = Parser::parse_sql(&GenericDialect {}, "SELECT 1 FROM t AS rhs")
        .expect("static existence skeleton");
    let ast::Statement::Query(query) = &mut skeleton[0] else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    let ast::TableFactor::Table { name, .. } = &mut select.from[0].relation else {
        unreachable!()
    };
    *name = ast::ObjectName::from(vec![ident(&relationship.right.id)]);
    let mut predicates = relationship
        .keys
        .iter()
        .map(|(left, right)| {
            let left = sql_field(left);
            let right = sql_field_aliased(right, "rhs");
            if relationship.null_keys_match {
                ast::Expr::IsNotDistinctFrom(Box::new(left), Box::new(right))
            } else {
                binary(left, ast::BinaryOperator::Eq, right)
            }
        })
        .collect::<Vec<_>>();
    if let Some(predicate) = &relationship.predicate {
        predicates.push(sql_predicate_aliased(predicate, parameters, "rhs"));
    }
    select.selection = predicates
        .into_iter()
        .reduce(|l, r| binary(l, ast::BinaryOperator::And, r));
    ast::Expr::Exists {
        subquery: query.clone(),
        negated: relationship.mode == ExistenceMode::Absent,
    }
}
fn related_scan(relationship: &BoundRelationship, extra: Option<&BoundField>) -> String {
    fn collect(predicate: &BoundPredicate, fields: &mut BTreeSet<String>) {
        match predicate {
            BoundPredicate::Compare { field, .. } | BoundPredicate::IsNull { field, .. } => {
                fields.insert(field.field.name().clone());
            }
            BoundPredicate::Not { predicate } | BoundPredicate::Mapped { predicate, .. } => {
                collect(predicate, fields)
            }
            BoundPredicate::All { predicates } | BoundPredicate::Any { predicates } => {
                for p in predicates {
                    collect(p, fields);
                }
            }
        }
    }
    let mut fields: BTreeSet<_> = relationship
        .keys
        .iter()
        .map(|(_, right)| right.field.name().clone())
        .collect();
    if let Some(predicate) = &relationship.predicate {
        collect(predicate, &mut fields);
    }
    if let Some(field) = extra {
        fields.insert(field.field.name().clone());
    }
    let mut skeleton = Parser::parse_sql(&GenericDialect {}, "SELECT x FROM t AS rhs")
        .expect("static related scan skeleton");
    let ast::Statement::Query(query) = &mut skeleton[0] else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    let ast::TableFactor::Table { name, .. } = &mut select.from[0].relation else {
        unreachable!()
    };
    *name = ast::ObjectName::from(vec![ident(&relationship.right.id)]);
    select.projection = fields
        .into_iter()
        .map(|name| {
            ast::SelectItem::UnnamedExpr(ast::Expr::CompoundIdentifier(vec![
                ident("rhs"),
                ident(&name),
            ]))
        })
        .collect();
    skeleton[0].to_string()
}

fn sql_ratio(ratio: &RatioExpression) -> ast::Expr {
    let mut skeleton = Parser::parse_sql(
        &GenericDialect {},
        "SELECT semantic_ratio_i64_v1(x,y,false)",
    )
    .expect("static ratio function");
    let ast::Statement::Query(query) = &mut skeleton[0] else {
        unreachable!()
    };
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    let ast::SelectItem::UnnamedExpr(ast::Expr::Function(function)) = &mut select.projection[0]
    else {
        unreachable!()
    };
    let ast::FunctionArguments::List(arguments) = &mut function.args else {
        unreachable!()
    };
    arguments.args = vec![
        sql_field(&ratio.numerator),
        sql_field(&ratio.denominator),
        ast::Expr::Value(ast::Value::Boolean(ratio.zero == ZeroDivision::Zero).into()),
    ]
    .into_iter()
    .map(|expr| ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(expr)))
    .collect();
    ast::Expr::Function(function.clone())
}
