use super::*;
use datafusion::logical_expr::{JoinType, Operator, expr::BinaryExpr, lit, when};
use semantic_catalog::{CatalogSnapshot, ObjectRef};

#[derive(Debug, Clone, Serialize)]
pub(super) struct Composition {
    pub left: usize,
    pub right: usize,
    pub definition: Option<ObjectRef>,
    domain: GroupDomain,
    nulls: NullAlignment,
    keys: Vec<Key>,
    outputs: Vec<Value>,
    presence: String,
}
#[derive(Debug, Clone, Serialize)]
struct Key {
    left: String,
    right: String,
    alias: String,
}
#[derive(Debug, Clone, Serialize)]
struct Value {
    side: Side,
    column: String,
    alias: String,
    missing: MissingGroup,
    zero: Option<Literal>,
}

type BoundComposition = (Composition, Vec<Slot>, Option<BTreeSet<String>>);

pub(super) fn bind(
    snapshot: &CatalogSnapshot,
    operation: &GraphOperation,
    left: usize,
    right: usize,
    nodes: &[CheckedNode],
    options: &CompileOptions,
) -> Result<BoundComposition, CompileDiagnostic> {
    let GraphOperation::Compose {
        relationship_relation,
        relationship,
        role,
        domain,
        null_alignment,
        keys,
        outputs,
        ..
    } = operation
    else {
        unreachable!()
    };
    let l = &nodes[left];
    let r = &nodes[right];
    let (Some(lgrain), Some(rgrain)) = (&l.group_keys, &r.group_keys) else {
        return Err(diagnostic(
            "composition_grain",
            "Each fact input must be independently aggregated to a proved group grain",
        ));
    };
    if keys.len().saturating_add(outputs.len()) > options.max_nodes || outputs.is_empty() {
        return Err(diagnostic(
            "work_limit",
            "Composition requires bounded keys and nonempty measure outputs",
        ));
    }
    if keys.iter().map(|k| k.left.clone()).collect::<BTreeSet<_>>() != *lgrain
        || keys
            .iter()
            .map(|k| k.right.clone())
            .collect::<BTreeSet<_>>()
            != *rgrain
        || keys.len() != lgrain.len()
        || keys.len() != rgrain.len()
    {
        return Err(diagnostic(
            "composition_grain",
            "Alignment must cover the complete grain of both inputs exactly once",
        ));
    }
    let relation = if keys.is_empty() {
        if !relationship.is_empty() || !relationship_relation.is_empty() || !role.is_empty() {
            return Err(diagnostic(
                "composition_relationship",
                "Scalar composition has no dimension relationship",
            ));
        }
        None
    } else {
        if !context::allowed(relationship_relation, options) {
            return Err(diagnostic(
                "access_scope",
                "Composition relationship is outside the access scope",
            ));
        }
        Some(snapshot.relation(relationship_relation).ok_or_else(|| {
            diagnostic(
                "unknown_relation",
                "Composition relationship owner is missing",
            )
        })?)
    };
    let contract = relation
        .map(|relation| {
            relation
                .definition()
                .semantics
                .as_ref()
                .and_then(|s| s.relationships.get(relationship))
                .ok_or_else(|| {
                    diagnostic(
                        "unknown_relationship",
                        "Group alignment needs an authored relationship",
                    )
                })
        })
        .transpose()?;
    if let Some(contract) = contract
        && (contract.id.is_empty()
            || contract.role != *role
            || !context::allowed(&contract.right_relation, options)
            || contract.key_pairs.len() != keys.len())
    {
        return Err(diagnostic(
            "composition_relationship",
            "Alignment must preserve the authored role and complete key tuple within scope",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut slots = Vec::new();
    let mut bound_keys = Vec::new();
    let mut matched_pairs = BTreeSet::new();
    for key in keys {
        options.check()?;
        unique_output(&key.id, &key.alias, &mut ids, &mut aliases)?;
        let a = slot(l, &key.left)?;
        let b = slot(r, &key.right)?;
        if a.field.data_type() != b.field.data_type() || !exact_type(a.field.data_type()) {
            return Err(diagnostic(
                "composition_key_type",
                "Group keys require identical exact comparison types",
            ));
        }
        let contract = contract.expect("nonempty keys require relationship");
        let pair = contract
            .key_pairs
            .iter()
            .position(|p| {
                a.origin.as_ref() == Some(&(relationship_relation.clone(), p.left_field.clone()))
                    && b.origin.as_ref()
                        == Some(&(contract.right_relation.clone(), p.right_field.clone()))
            })
            .ok_or_else(|| {
                diagnostic(
                    "composition_key_origin",
                    "Output group keys do not implement the authored relationship",
                )
            })?;
        if !matched_pairs.insert(pair) {
            return Err(diagnostic(
                "composition_key_origin",
                "Relationship keys cannot be reused",
            ));
        }
        bound_keys.push(Key {
            left: a.field.name().clone(),
            right: b.field.name().clone(),
            alias: key.alias.clone(),
        });
        slots.push(Slot {
            id: key.id.clone(),
            field: Field::new(&key.alias, a.field.data_type().clone(), true),
            origin: a.origin.clone(),
            meaning: if a.meaning == b.meaning {
                a.meaning.clone()
            } else {
                SlotMeaning::default()
            },
        });
    }
    let mut bound_values = Vec::new();
    for output in outputs {
        options.check()?;
        unique_output(&output.id, &output.alias, &mut ids, &mut aliases)?;
        let source = slot(if output.side == Side::Left { l } else { r }, &output.slot)?;
        let zero = if output.missing == MissingGroup::Zero {
            Some(zero(source.field.data_type())?)
        } else {
            None
        };
        bound_values.push(Value {
            side: output.side,
            column: source.field.name().clone(),
            alias: output.alias.clone(),
            missing: output.missing,
            zero,
        });
        slots.push(Slot {
            id: output.id.clone(),
            field: Field::new(&output.alias, source.field.data_type().clone(), true),
            origin: None,
            meaning: source.meaning.clone(),
        });
    }
    let mut presence = "__semantic_group_present".to_owned();
    while l
        .slots
        .iter()
        .chain(&r.slots)
        .any(|s| s.field.name() == &presence)
    {
        presence.push('_');
    }
    // FULL + ordinary null equality can retain a null-key row from each side.
    // Preserve both, and do not falsely claim a unique merged key for reuse.
    let unique = !(*domain == GroupDomain::Union
        && *null_alignment == NullAlignment::NeverMatch
        && keys.iter().any(|k| {
            slot(l, &k.left).unwrap().field.is_nullable()
                || slot(r, &k.right).unwrap().field.is_nullable()
        }));
    let grain = unique.then(|| keys.iter().map(|k| k.id.clone()).collect());
    Ok((
        Composition {
            left,
            right,
            definition: relation.map(|r| {
                r.definition_reference("relationship", relationship)
                    .expect("indexed relationship")
                    .clone()
            }),
            domain: *domain,
            nulls: *null_alignment,
            keys: bound_keys,
            outputs: bound_values,
            presence,
        },
        slots,
        grain,
    ))
}
fn zero(ty: &DataType) -> Result<Literal, CompileDiagnostic> {
    Ok(match ty {
        DataType::Int16 => Literal::Int16(0),
        DataType::Int32 => Literal::Int32(0),
        DataType::Int64 => Literal::Int64(0),
        DataType::UInt64 => Literal::UInt64(0),
        DataType::Decimal128(p, s) if *s >= 0 => Literal::Decimal128 {
            coefficient: "0".into(),
            precision: *p,
            scale: *s as u8,
        },
        _ => {
            return Err(diagnostic(
                "missing_group_type",
                "Zero filling requires an exact numeric measure",
            ));
        }
    })
}
fn alias(side: Side) -> &'static str {
    if side == Side::Left { "l" } else { "r" }
}
fn column(side: Side, name: &str) -> Expr {
    Expr::Column(Column::new(Some(alias(side)), name))
}
pub(super) fn plan(
    c: &Composition,
    left: DataFrame,
    right: DataFrame,
) -> Result<DataFrame, CompileDiagnostic> {
    let l = left
        .with_column(&c.presence, lit(true))
        .and_then(|f| f.alias("l"))
        .map_err(lower::backend_error)?;
    let r = right
        .with_column(&c.presence, lit(true))
        .and_then(|f| f.alias("r"))
        .map_err(lower::backend_error)?;
    let mut on: Vec<_> = c
        .keys
        .iter()
        .map(|k| {
            Expr::BinaryExpr(BinaryExpr::new(
                Box::new(column(Side::Left, &k.left)),
                if c.nulls == NullAlignment::Match {
                    Operator::IsNotDistinctFrom
                } else {
                    Operator::Eq
                },
                Box::new(column(Side::Right, &k.right)),
            ))
        })
        .collect();
    if on.is_empty() {
        on.push(lit(true));
    }
    let joined = l
        .join_on(
            r,
            match c.domain {
                GroupDomain::Union => JoinType::Full,
                GroupDomain::Intersection => JoinType::Inner,
                GroupDomain::Left => JoinType::Left,
                GroupDomain::Right => JoinType::Right,
            },
            on,
        )
        .map_err(lower::backend_error)?;
    let mut exprs = Vec::new();
    for key in &c.keys {
        let a = column(Side::Left, &key.left);
        let b = column(Side::Right, &key.right);
        exprs.push(
            when(a.clone().is_not_null(), a)
                .otherwise(b)
                .map_err(lower::backend_error)?
                .alias(&key.alias),
        );
    }
    for value in &c.outputs {
        let source = column(value.side, &value.column);
        let expression = if let Some(zero) = &value.zero {
            when(
                column(value.side, &c.presence).is_null(),
                lit(super::super::literal::checked_scalar(zero)?),
            )
            .otherwise(source)
            .map_err(lower::backend_error)?
        } else {
            source
        };
        exprs.push(expression.alias(&value.alias));
    }
    joined.select(exprs).map_err(lower::backend_error)
}
fn sql_col(side: Side, name: &str) -> ast::Expr {
    ast::Expr::CompoundIdentifier(vec![ident(alias(side)), ident(name)])
}
fn sql_case(condition: ast::Expr, yes: ast::Expr, no: ast::Expr) -> ast::Expr {
    let q = parsed("SELECT CASE WHEN true THEN 0 ELSE 1 END");
    let ast::SetExpr::Select(s) = *q.body else {
        unreachable!()
    };
    let ast::SelectItem::UnnamedExpr(mut expression) = s.projection.into_iter().next().unwrap()
    else {
        unreachable!()
    };
    let ast::Expr::Case {
        conditions,
        else_result,
        ..
    } = &mut expression
    else {
        unreachable!()
    };
    conditions[0].condition = condition;
    conditions[0].result = yes;
    *else_result = Some(Box::new(no));
    expression
}
pub(super) fn emit(c: &Composition, names: &[String]) -> Box<ast::Query> {
    let mut q = parsed("SELECT l.x FROM (SELECT 1) AS l FULL JOIN (SELECT 1) AS r ON true");
    let ast::SetExpr::Select(s) = q.body.as_mut() else {
        unreachable!()
    };
    let side_query = |index| {
        let mut q = table_query(index, names);
        let ast::SetExpr::Select(s) = q.body.as_mut() else {
            unreachable!()
        };
        s.projection.push(ast::SelectItem::ExprWithAlias {
            expr: ast::Expr::Value(ast::Value::Boolean(true).into()),
            alias: ident(&c.presence),
        });
        q
    };
    let ast::TableFactor::Derived { subquery, .. } = &mut s.from[0].relation else {
        unreachable!()
    };
    *subquery = side_query(c.left);
    let join = &mut s.from[0].joins[0];
    let ast::TableFactor::Derived { subquery, .. } = &mut join.relation else {
        unreachable!()
    };
    *subquery = side_query(c.right);
    let condition = c
        .keys
        .iter()
        .map(|k| {
            let a = sql_col(Side::Left, &k.left);
            let b = sql_col(Side::Right, &k.right);
            if c.nulls == NullAlignment::Match {
                ast::Expr::IsNotDistinctFrom(Box::new(a), Box::new(b))
            } else {
                ast::Expr::BinaryOp {
                    left: Box::new(a),
                    op: ast::BinaryOperator::Eq,
                    right: Box::new(b),
                }
            }
        })
        .map(|expr| ast::Expr::Nested(Box::new(expr)))
        .reduce(|a, b| ast::Expr::BinaryOp {
            left: Box::new(a),
            op: ast::BinaryOperator::And,
            right: Box::new(b),
        })
        .unwrap_or(ast::Expr::Value(ast::Value::Boolean(true).into()));
    let on = ast::JoinConstraint::On(condition);
    join.join_operator = match c.domain {
        GroupDomain::Union => ast::JoinOperator::FullOuter(on),
        GroupDomain::Intersection => ast::JoinOperator::Inner(on),
        GroupDomain::Left => ast::JoinOperator::LeftOuter(on),
        GroupDomain::Right => ast::JoinOperator::RightOuter(on),
    };
    s.projection = c
        .keys
        .iter()
        .map(|k| {
            let a = sql_col(Side::Left, &k.left);
            ast::SelectItem::ExprWithAlias {
                expr: sql_case(
                    ast::Expr::IsNotNull(Box::new(a.clone())),
                    a,
                    sql_col(Side::Right, &k.right),
                ),
                alias: ident(&k.alias),
            }
        })
        .collect();
    for value in &c.outputs {
        let source = sql_col(value.side, &value.column);
        let expr = if let Some(zero) = &value.zero {
            let zero = ast::Expr::Cast {
                kind: ast::CastKind::Cast,
                expr: Box::new(ast::Expr::Value(
                    ast::Value::Number("0".into(), false).into(),
                )),
                data_type: super::super::literal::sql_type(zero),
                format: None,
                array: false,
            };
            sql_case(
                ast::Expr::IsNull(Box::new(sql_col(value.side, &c.presence))),
                zero,
                source,
            )
        } else {
            source
        };
        s.projection.push(ast::SelectItem::ExprWithAlias {
            expr,
            alias: ident(&value.alias),
        });
    }
    q
}
