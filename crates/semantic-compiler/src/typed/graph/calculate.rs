use super::super::scalar::{
    CheckedCast, CheckedConditional, CheckedNullTest, CheckedPredicate, CheckedScalarCall,
    CheckedSlot, CheckedSlotComparison,
};
use super::*;
use semantic_catalog::{FactResolution, Presence, SlotMeaning, checked_unit_quotient};

#[derive(Debug, Clone, Serialize)]
pub(super) struct Calculation {
    pub input: usize,
    projections: Vec<(String, String)>,
    ratios: Vec<Ratio>,
}
#[derive(Debug, Clone, Serialize)]
struct Ratio {
    expression: CheckedScalarCall,
    alias: String,
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct Filter {
    pub input: usize,
    predicate: CheckedPredicate,
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct Conditional {
    pub input: usize,
    projections: Vec<(String, String)>,
    outputs: Vec<ConditionalOutput>,
}
#[derive(Debug, Clone, Serialize)]
struct ConditionalOutput {
    expression: CheckedConditional,
    alias: String,
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct Cast {
    pub input: usize,
    projections: Vec<(String, String)>,
    outputs: Vec<CastOutput>,
}
#[derive(Debug, Clone, Serialize)]
struct CastOutput {
    expression: CheckedCast,
    alias: String,
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct NullTest {
    pub input: usize,
    projections: Vec<(String, String)>,
    outputs: Vec<NullTestOutput>,
}
#[derive(Debug, Clone, Serialize)]
struct NullTestOutput {
    expression: CheckedNullTest,
    alias: String,
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct SlotComparison {
    pub input: usize,
    projections: Vec<(String, String)>,
    outputs: Vec<ComparisonOutput>,
}
#[derive(Debug, Clone, Serialize)]
struct ComparisonOutput {
    expression: CheckedSlotComparison,
    alias: String,
}

pub(super) fn bind_slot_comparison(
    input: usize,
    source: &CheckedNode,
    passthrough: &[GraphProjection],
    comparisons: &[GraphSlotComparison],
    options: &CompileOptions,
) -> Result<(SlotComparison, Vec<Slot>, Option<BTreeSet<String>>), CompileDiagnostic> {
    if comparisons.is_empty()
        || passthrough.len().saturating_add(comparisons.len()) > options.max_nodes
    {
        return Err(diagnostic(
            "graph_comparison",
            "Slot comparison needs bounded, nonempty outputs",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut slots = Vec::new();
    let mut projections = Vec::new();
    let mut keys = BTreeSet::new();
    for projection in passthrough {
        options.check()?;
        unique_output(&projection.id, &projection.alias, &mut ids, &mut aliases)?;
        let prior = slot(source, &projection.slot)?;
        if source
            .group_keys
            .as_ref()
            .is_some_and(|g| g.contains(&projection.slot))
        {
            keys.insert(projection.id.clone());
        }
        projections.push((prior.field.name().clone(), projection.alias.clone()));
        slots.push(Slot {
            id: projection.id.clone(),
            field: Field::new(
                &projection.alias,
                prior.field.data_type().clone(),
                prior.field.is_nullable(),
            ),
            origin: prior.origin.clone(),
            meaning: prior.meaning.clone(),
        });
    }
    let mut outputs = Vec::new();
    for comparison in comparisons {
        options.check()?;
        unique_output(&comparison.id, &comparison.alias, &mut ids, &mut aliases)?;
        let left = slot(source, &comparison.left)?;
        let right = slot(source, &comparison.right)?;
        let expression = CheckedSlotComparison::int64(
            &source.id,
            CheckedSlot::new(&source.id, &left.id, &left.field),
            CheckedSlot::new(&source.id, &right.id, &right.field),
            comparison.operator,
        )?;
        compatible_comparison_units(&left.meaning, &right.meaning)?;
        slots.push(Slot {
            id: comparison.id.clone(),
            field: Field::new(
                &comparison.alias,
                expression.result_type().clone(),
                expression.nullable(),
            ),
            origin: None,
            meaning: SlotMeaning::default(),
        });
        outputs.push(ComparisonOutput {
            expression,
            alias: comparison.alias.clone(),
        });
    }
    let grain = source
        .group_keys
        .as_ref()
        .and_then(|g| (keys.len() == g.len()).then_some(keys));
    Ok((
        SlotComparison {
            input,
            projections,
            outputs,
        },
        slots,
        grain,
    ))
}

fn compatible_comparison_units(
    left: &SlotMeaning,
    right: &SlotMeaning,
) -> Result<(), CompileDiagnostic> {
    match (&left.unit, &right.unit) {
        (FactResolution::Unknown, FactResolution::Unknown) => Ok(()),
        (
            FactResolution::Known {
                value: Presence::Value(left),
                ..
            },
            FactResolution::Known {
                value: Presence::Value(right),
                ..
            },
        ) if left == right => Ok(()),
        _ => Err(diagnostic(
            "graph_comparison_unit",
            "Graph slot comparison requires equal authored units or two unknown units",
        )),
    }
}

pub(super) fn bind_null_test(
    input: usize,
    source: &CheckedNode,
    passthrough: &[GraphProjection],
    tests: &[GraphNullTest],
    options: &CompileOptions,
) -> Result<(NullTest, Vec<Slot>, Option<BTreeSet<String>>), CompileDiagnostic> {
    if tests.is_empty() || passthrough.len().saturating_add(tests.len()) > options.max_nodes {
        return Err(diagnostic(
            "graph_null_test",
            "Null test needs bounded, nonempty outputs",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut slots = Vec::new();
    let mut projections = Vec::new();
    let mut keys = BTreeSet::new();
    for projection in passthrough {
        options.check()?;
        unique_output(&projection.id, &projection.alias, &mut ids, &mut aliases)?;
        let prior = slot(source, &projection.slot)?;
        if source
            .group_keys
            .as_ref()
            .is_some_and(|g| g.contains(&projection.slot))
        {
            keys.insert(projection.id.clone());
        }
        projections.push((prior.field.name().clone(), projection.alias.clone()));
        slots.push(Slot {
            id: projection.id.clone(),
            field: Field::new(
                &projection.alias,
                prior.field.data_type().clone(),
                prior.field.is_nullable(),
            ),
            origin: prior.origin.clone(),
            meaning: prior.meaning.clone(),
        });
    }
    let mut outputs = Vec::new();
    for test in tests {
        options.check()?;
        unique_output(&test.id, &test.alias, &mut ids, &mut aliases)?;
        let prior = slot(source, &test.slot)?;
        let expression = CheckedNullTest::int64(
            &source.id,
            CheckedSlot::new(&source.id, &prior.id, &prior.field),
            test.operator == GraphNullOperator::IsNotNull,
        )?;
        slots.push(Slot {
            id: test.id.clone(),
            field: Field::new(
                &test.alias,
                expression.result_type().clone(),
                expression.nullable(),
            ),
            origin: None,
            meaning: SlotMeaning::default(),
        });
        outputs.push(NullTestOutput {
            expression,
            alias: test.alias.clone(),
        });
    }
    let grain = source
        .group_keys
        .as_ref()
        .and_then(|g| (keys.len() == g.len()).then_some(keys));
    Ok((
        NullTest {
            input,
            projections,
            outputs,
        },
        slots,
        grain,
    ))
}

pub(super) fn bind_cast(
    input: usize,
    source: &CheckedNode,
    passthrough: &[GraphProjection],
    casts: &[GraphCast],
    options: &CompileOptions,
) -> Result<(Cast, Vec<Slot>, Option<BTreeSet<String>>), CompileDiagnostic> {
    if casts.is_empty() || passthrough.len().saturating_add(casts.len()) > options.max_nodes {
        return Err(diagnostic(
            "graph_cast",
            "Cast needs bounded, nonempty outputs",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut slots = Vec::new();
    let mut projections = Vec::new();
    let mut keys = BTreeSet::new();
    for projection in passthrough {
        options.check()?;
        unique_output(&projection.id, &projection.alias, &mut ids, &mut aliases)?;
        let prior = slot(source, &projection.slot)?;
        if source
            .group_keys
            .as_ref()
            .is_some_and(|g| g.contains(&projection.slot))
        {
            keys.insert(projection.id.clone());
        }
        projections.push((prior.field.name().clone(), projection.alias.clone()));
        slots.push(Slot {
            id: projection.id.clone(),
            field: Field::new(
                &projection.alias,
                prior.field.data_type().clone(),
                prior.field.is_nullable(),
            ),
            origin: prior.origin.clone(),
            meaning: prior.meaning.clone(),
        });
    }
    let mut outputs = Vec::new();
    for cast in casts {
        options.check()?;
        unique_output(&cast.id, &cast.alias, &mut ids, &mut aliases)?;
        let prior = slot(source, &cast.slot)?;
        let expression = match cast.target {
            GraphCastTarget::Decimal128Scale0 => CheckedCast::int64_to_decimal128_scale0(
                &source.id,
                CheckedSlot::new(&source.id, &prior.id, &prior.field),
            )?,
        };
        slots.push(Slot {
            id: cast.id.clone(),
            field: Field::new(
                &cast.alias,
                expression.result_type().clone(),
                expression.nullable(),
            ),
            origin: prior.origin.clone(),
            meaning: prior.meaning.clone(),
        });
        outputs.push(CastOutput {
            expression,
            alias: cast.alias.clone(),
        });
    }
    let grain = source
        .group_keys
        .as_ref()
        .and_then(|g| (keys.len() == g.len()).then_some(keys));
    Ok((
        Cast {
            input,
            projections,
            outputs,
        },
        slots,
        grain,
    ))
}

pub(super) fn bind_conditional(
    input: usize,
    source: &CheckedNode,
    passthrough: &[GraphProjection],
    outputs: &[GraphConditional],
    options: &CompileOptions,
) -> Result<(Conditional, Vec<Slot>, Option<BTreeSet<String>>), CompileDiagnostic> {
    if outputs.is_empty() || passthrough.len().saturating_add(outputs.len()) > options.max_nodes {
        return Err(diagnostic(
            "graph_conditional",
            "Conditional needs bounded, nonempty outputs",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut slots = Vec::new();
    let mut projections = Vec::new();
    let mut keys = BTreeSet::new();
    for projection in passthrough {
        options.check()?;
        unique_output(&projection.id, &projection.alias, &mut ids, &mut aliases)?;
        let prior = slot(source, &projection.slot)?;
        if source
            .group_keys
            .as_ref()
            .is_some_and(|g| g.contains(&projection.slot))
        {
            keys.insert(projection.id.clone());
        }
        projections.push((prior.field.name().clone(), projection.alias.clone()));
        slots.push(Slot {
            id: projection.id.clone(),
            field: Field::new(
                &projection.alias,
                prior.field.data_type().clone(),
                prior.field.is_nullable(),
            ),
            origin: prior.origin.clone(),
            meaning: prior.meaning.clone(),
        });
    }
    let mut checked = Vec::new();
    for output in outputs {
        options.check()?;
        unique_output(&output.id, &output.alias, &mut ids, &mut aliases)?;
        let predicate = bind_predicate(source, &output.when, options, 1)?;
        let expression = CheckedConditional::new(
            &source.id,
            predicate,
            output.then_value.clone(),
            output.else_value.clone(),
        )?;
        slots.push(Slot {
            id: output.id.clone(),
            field: Field::new(
                &output.alias,
                expression.result_type().clone(),
                expression.nullable(),
            ),
            origin: None,
            meaning: SlotMeaning::default(),
        });
        checked.push(ConditionalOutput {
            expression,
            alias: output.alias.clone(),
        });
    }
    let grain = source
        .group_keys
        .as_ref()
        .and_then(|g| (keys.len() == g.len()).then_some(keys));
    Ok((
        Conditional {
            input,
            projections,
            outputs: checked,
        },
        slots,
        grain,
    ))
}

pub(super) fn bind_calculation(
    input: usize,
    source: &CheckedNode,
    passthrough: &[GraphProjection],
    ratios: &[GraphRatio],
    options: &CompileOptions,
) -> Result<(Calculation, Vec<Slot>, Option<BTreeSet<String>>), CompileDiagnostic> {
    if passthrough.len().saturating_add(ratios.len()) > options.max_nodes || ratios.is_empty() {
        return Err(diagnostic(
            "graph_calculation",
            "Calculation needs bounded, nonempty ratios",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut slots = Vec::new();
    let mut projections = Vec::new();
    let mut keys = BTreeSet::new();
    for projection in passthrough {
        options.check()?;
        unique_output(&projection.id, &projection.alias, &mut ids, &mut aliases)?;
        let prior = slot(source, &projection.slot)?;
        if source
            .group_keys
            .as_ref()
            .is_some_and(|g| g.contains(&projection.slot))
        {
            keys.insert(projection.id.clone());
        }
        projections.push((prior.field.name().clone(), projection.alias.clone()));
        slots.push(Slot {
            id: projection.id.clone(),
            field: Field::new(
                &projection.alias,
                prior.field.data_type().clone(),
                prior.field.is_nullable(),
            ),
            origin: prior.origin.clone(),
            meaning: prior.meaning.clone(),
        });
    }
    let mut checked = Vec::new();
    for ratio in ratios {
        options.check()?;
        unique_output(&ratio.id, &ratio.alias, &mut ids, &mut aliases)?;
        let numerator = slot(source, &ratio.numerator)?;
        let denominator = slot(source, &ratio.denominator)?;
        let expression = CheckedScalarCall::ratio_i64_v1(
            CheckedSlot::new(&source.id, &numerator.id, &numerator.field),
            CheckedSlot::new(&source.id, &denominator.id, &denominator.field),
            ratio.zero == ZeroDivision::Zero,
        )?;
        let unit = checked_unit_quotient(&numerator.meaning.unit, &denominator.meaning.unit)
            .map_err(|_| {
                diagnostic(
                    "graph_ratio_unit",
                    "Graph ratio operands require non-null, non-conflicting units",
                )
            })?;
        if ratio
            .required_unit
            .as_ref()
            .is_some_and(|required| !matches!(&unit, Presence::Value(actual) if actual == required))
        {
            return Err(diagnostic(
                "graph_ratio_unit",
                "Requested graph ratio unit does not match the authored operands",
            ));
        }
        let meaning = SlotMeaning {
            unit: match unit {
                Presence::Value(unit) => FactResolution::Known {
                    value: Presence::Value(unit),
                    contributors: vec![],
                },
                Presence::Missing => FactResolution::Unknown,
                Presence::Null => unreachable!("checked quotient rejects null"),
            },
            source_grain: if numerator.meaning.source_grain == denominator.meaning.source_grain {
                numerator.meaning.source_grain.clone()
            } else {
                FactResolution::Unknown
            },
            entity: if numerator.meaning.entity == denominator.meaning.entity {
                numerator.meaning.entity.clone()
            } else {
                FactResolution::Unknown
            },
        };
        checked.push(Ratio {
            expression: expression.clone(),
            alias: ratio.alias.clone(),
        });
        slots.push(Slot {
            id: ratio.id.clone(),
            field: Field::new(
                &ratio.alias,
                expression.result_type().clone(),
                expression.nullable(),
            ),
            origin: None,
            meaning,
        });
    }
    let grain = source
        .group_keys
        .as_ref()
        .and_then(|g| (keys.len() == g.len()).then_some(keys));
    Ok((
        Calculation {
            input,
            projections,
            ratios: checked,
        },
        slots,
        grain,
    ))
}

pub(super) fn bind_filter(
    input: usize,
    source: &CheckedNode,
    predicate: &OutputPredicate,
    options: &CompileOptions,
) -> Result<Filter, CompileDiagnostic> {
    Ok(Filter {
        input,
        predicate: bind_predicate(source, predicate, options, 1)?,
    })
}
fn bind_predicate(
    source: &CheckedNode,
    predicate: &OutputPredicate,
    options: &CompileOptions,
    depth: usize,
) -> Result<CheckedPredicate, CompileDiagnostic> {
    options.check()?;
    if depth > options.max_depth {
        return Err(diagnostic(
            "work_limit",
            "Graph predicate depth budget exhausted",
        ));
    }
    Ok(match predicate {
        RowPredicate::CompareMapped { .. } => {
            return Err(diagnostic(
                "graph_mapping",
                "Graph outputs have no authored value mapping",
            ));
        }
        RowPredicate::CompareParameter { .. } => {
            return Err(diagnostic(
                "unbound_parameter",
                "A graph comparison parameter must be bound before compilation",
            ));
        }
        RowPredicate::Compare {
            field,
            operator,
            value,
        } => {
            let column = slot(source, &field.slot)?;
            CheckedPredicate::compare(
                CheckedSlot::new(&source.id, &column.id, &column.field),
                *operator,
                value.clone(),
            )?
        }
        RowPredicate::IsNull { field, negated } => {
            let column = slot(source, &field.slot)?;
            CheckedPredicate::is_null(
                CheckedSlot::new(&source.id, &column.id, &column.field),
                *negated,
            )
        }
        RowPredicate::Not { predicate } => {
            CheckedPredicate::not(bind_predicate(source, predicate, options, depth + 1)?)
        }
        RowPredicate::All { predicates } | RowPredicate::Any { predicates } => {
            if predicates.is_empty() || predicates.len() > options.max_nodes {
                return Err(diagnostic(
                    "empty_boolean",
                    "Graph boolean groups require bounded nonempty predicates",
                ));
            }
            let children = predicates
                .iter()
                .map(|p| bind_predicate(source, p, options, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            if matches!(predicate, RowPredicate::All { .. }) {
                CheckedPredicate::all(children)?
            } else {
                CheckedPredicate::any(children)?
            }
        }
    })
}

pub(super) fn plan_calculation(
    calc: &Calculation,
    frame: DataFrame,
) -> Result<DataFrame, CompileDiagnostic> {
    let mut expressions = calc
        .projections
        .iter()
        .map(|(column, alias)| col(column).alias(alias))
        .collect::<Vec<_>>();
    expressions.extend(
        calc.ratios
            .iter()
            .map(|ratio| ratio.expression.direct().alias(&ratio.alias)),
    );
    frame.select(expressions).map_err(lower::backend_error)
}
pub(super) fn plan_filter(
    filter: &Filter,
    frame: DataFrame,
) -> Result<DataFrame, CompileDiagnostic> {
    frame
        .filter(filter.predicate.direct())
        .map_err(lower::backend_error)
}
pub(super) fn plan_conditional(
    conditional: &Conditional,
    frame: DataFrame,
) -> Result<DataFrame, CompileDiagnostic> {
    let mut expressions = conditional
        .projections
        .iter()
        .map(|(column, alias)| col(column).alias(alias))
        .collect::<Vec<_>>();
    for output in &conditional.outputs {
        expressions.push(output.expression.direct()?.alias(&output.alias));
    }
    frame.select(expressions).map_err(lower::backend_error)
}
pub(super) fn plan_cast(cast: &Cast, frame: DataFrame) -> Result<DataFrame, CompileDiagnostic> {
    let mut expressions = cast
        .projections
        .iter()
        .map(|(column, alias)| col(column).alias(alias))
        .collect::<Vec<_>>();
    expressions.extend(
        cast.outputs
            .iter()
            .map(|output| output.expression.direct().alias(&output.alias)),
    );
    frame.select(expressions).map_err(lower::backend_error)
}
pub(super) fn plan_null_test(
    test: &NullTest,
    frame: DataFrame,
) -> Result<DataFrame, CompileDiagnostic> {
    let mut expressions = test
        .projections
        .iter()
        .map(|(column, alias)| col(column).alias(alias))
        .collect::<Vec<_>>();
    expressions.extend(
        test.outputs
            .iter()
            .map(|output| output.expression.direct().alias(&output.alias)),
    );
    frame.select(expressions).map_err(lower::backend_error)
}
pub(super) fn plan_slot_comparison(
    comparison: &SlotComparison,
    frame: DataFrame,
) -> Result<DataFrame, CompileDiagnostic> {
    let mut expressions = comparison
        .projections
        .iter()
        .map(|(column, alias)| col(column).alias(alias))
        .collect::<Vec<_>>();
    expressions.extend(
        comparison
            .outputs
            .iter()
            .map(|output| output.expression.direct().alias(&output.alias)),
    );
    frame.select(expressions).map_err(lower::backend_error)
}

pub(super) fn emit_calculation(calc: &Calculation, names: &[String]) -> Box<ast::Query> {
    let mut query = table_query(calc.input, names);
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = calc
        .projections
        .iter()
        .map(|(column, alias)| ast::SelectItem::ExprWithAlias {
            expr: sql_column(column),
            alias: ident(alias),
        })
        .collect();
    for ratio in &calc.ratios {
        select.projection.push(ast::SelectItem::ExprWithAlias {
            expr: ratio.expression.sql("src"),
            alias: ident(&ratio.alias),
        });
    }
    query
}
pub(super) fn emit_filter(
    filter: &Filter,
    names: &[String],
    parameters: &mut Vec<Literal>,
) -> Box<ast::Query> {
    let mut query = table_query(filter.input, names);
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.selection = Some(filter.predicate.sql("src", parameters));
    query
}
pub(super) fn emit_conditional(
    conditional: &Conditional,
    names: &[String],
    parameters: &mut Vec<Literal>,
) -> Box<ast::Query> {
    let mut query = table_query(conditional.input, names);
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = conditional
        .projections
        .iter()
        .map(|(column, alias)| ast::SelectItem::ExprWithAlias {
            expr: sql_column(column),
            alias: ident(alias),
        })
        .collect();
    for output in &conditional.outputs {
        select.projection.push(ast::SelectItem::ExprWithAlias {
            expr: output.expression.sql("src", parameters),
            alias: ident(&output.alias),
        });
    }
    query
}
pub(super) fn emit_cast(cast: &Cast, names: &[String]) -> Box<ast::Query> {
    let mut query = table_query(cast.input, names);
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = cast
        .projections
        .iter()
        .map(|(column, alias)| ast::SelectItem::ExprWithAlias {
            expr: sql_column(column),
            alias: ident(alias),
        })
        .collect();
    for output in &cast.outputs {
        select.projection.push(ast::SelectItem::ExprWithAlias {
            expr: output.expression.sql("src"),
            alias: ident(&output.alias),
        });
    }
    query
}
pub(super) fn emit_null_test(test: &NullTest, names: &[String]) -> Box<ast::Query> {
    let mut query = table_query(test.input, names);
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = test
        .projections
        .iter()
        .map(|(column, alias)| ast::SelectItem::ExprWithAlias {
            expr: sql_column(column),
            alias: ident(alias),
        })
        .collect();
    for output in &test.outputs {
        select.projection.push(ast::SelectItem::ExprWithAlias {
            expr: output.expression.sql("src"),
            alias: ident(&output.alias),
        });
    }
    query
}
pub(super) fn emit_slot_comparison(
    comparison: &SlotComparison,
    names: &[String],
) -> Box<ast::Query> {
    let mut query = table_query(comparison.input, names);
    let ast::SetExpr::Select(select) = query.body.as_mut() else {
        unreachable!()
    };
    select.projection = comparison
        .projections
        .iter()
        .map(|(column, alias)| ast::SelectItem::ExprWithAlias {
            expr: sql_column(column),
            alias: ident(alias),
        })
        .collect();
    for output in &comparison.outputs {
        select.projection.push(ast::SelectItem::ExprWithAlias {
            expr: output.expression.sql("src"),
            alias: ident(&output.alias),
        });
    }
    query
}
fn sql_column(column: &str) -> ast::Expr {
    ast::Expr::CompoundIdentifier(vec![ident("src"), ident(column)])
}
