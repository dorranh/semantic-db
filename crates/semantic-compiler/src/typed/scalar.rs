//! Checked scalar Boolean expressions for consumers with a known input scope.
//! This first profile owns graph Filter predicates; other scalar operators are
//! added only when a compiler consumer uses them.

use super::{CompileDiagnostic, bind::BoundPredicate, diagnostic, literal, lower};
use datafusion::{
    common::Column,
    logical_expr::{Expr, lit},
    sql::sqlparser::{ast, dialect::GenericDialect, parser::Parser},
};
use semantic_catalog::{DataType, Field, ObjectRef};
use semantic_plan::typed::{Comparison, Literal};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub(super) struct CheckedSlot {
    pub(super) scope: String,
    id: String,
    pub(super) field: Field,
}

impl CheckedSlot {
    pub(super) fn new(scope: &str, id: &str, field: &Field) -> Self {
        Self {
            scope: scope.into(),
            id: id.into(),
            field: field.clone(),
        }
    }

    fn column(&self) -> &str {
        self.field.name()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum CheckedPredicate {
    Mapped {
        definition: ObjectRef,
        phrase: String,
        predicate: Box<Self>,
    },
    Compare {
        slot: CheckedSlot,
        operator: Comparison,
        value: Literal,
    },
    IsNull {
        slot: CheckedSlot,
        negated: bool,
    },
    All {
        children: Vec<Self>,
    },
    Any {
        children: Vec<Self>,
    },
    Not {
        inner: Box<Self>,
    },
}

impl CheckedPredicate {
    pub(super) fn in_scope(&self, scope: &str) -> bool {
        match self {
            Self::Mapped { predicate, .. } => predicate.in_scope(scope),
            Self::Compare { slot, .. } | Self::IsNull { slot, .. } => slot.scope == scope,
            Self::All { children } | Self::Any { children } => {
                children.iter().all(|child| child.in_scope(scope))
            }
            Self::Not { inner } => inner.in_scope(scope),
        }
    }

    pub(super) fn output(predicate: &BoundPredicate) -> Result<Self, CompileDiagnostic> {
        match predicate {
            BoundPredicate::Mapped {
                definition,
                phrase,
                predicate,
            } => Ok(Self::Mapped {
                definition: definition.clone(),
                phrase: phrase.clone(),
                predicate: Box::new(Self::output(predicate)?),
            }),
            BoundPredicate::Compare {
                field,
                operator,
                value,
            } => {
                if field.instance != "$output" {
                    return Err(diagnostic(
                        "output_filter_scope",
                        "Output filter reads must reference output slots",
                    ));
                }
                Self::compare(
                    CheckedSlot::new("$output", field.field.name(), &field.field),
                    *operator,
                    value.clone(),
                )
            }
            BoundPredicate::IsNull { field, negated } => {
                if field.instance != "$output" {
                    return Err(diagnostic(
                        "output_filter_scope",
                        "Output filter reads must reference output slots",
                    ));
                }
                Ok(Self::is_null(
                    CheckedSlot::new("$output", field.field.name(), &field.field),
                    *negated,
                ))
            }
            BoundPredicate::All { predicates } => Self::all(
                predicates
                    .iter()
                    .map(Self::output)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            BoundPredicate::Any { predicates } => Self::any(
                predicates
                    .iter()
                    .map(Self::output)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            BoundPredicate::Not { predicate } => Ok(Self::not(Self::output(predicate)?)),
        }
    }

    pub(super) fn compare(
        slot: CheckedSlot,
        operator: Comparison,
        value: Literal,
    ) -> Result<Self, CompileDiagnostic> {
        let literal_type = literal::checked_scalar(&value)?.data_type();
        if slot.field.data_type() != &literal_type {
            return Err(diagnostic(
                "comparison_type",
                "Graph comparison requires an exact output/literal type",
            ));
        }
        if matches!(value, Literal::Boolean(_))
            && !matches!(operator, Comparison::Eq | Comparison::NotEq)
        {
            return Err(diagnostic(
                "comparison_operator",
                "Boolean comparisons support equality only",
            ));
        }
        Ok(Self::Compare {
            slot,
            operator,
            value,
        })
    }

    pub(super) fn is_null(slot: CheckedSlot, negated: bool) -> Self {
        Self::IsNull { slot, negated }
    }

    pub(super) fn not(inner: Self) -> Self {
        Self::Not {
            inner: Box::new(inner),
        }
    }

    pub(super) fn all(children: Vec<Self>) -> Result<Self, CompileDiagnostic> {
        if children.is_empty() {
            return Err(diagnostic(
                "empty_boolean",
                "Graph boolean groups require bounded nonempty predicates",
            ));
        }
        Ok(Self::All { children })
    }

    pub(super) fn any(children: Vec<Self>) -> Result<Self, CompileDiagnostic> {
        if children.is_empty() {
            return Err(diagnostic(
                "empty_boolean",
                "Graph boolean groups require bounded nonempty predicates",
            ));
        }
        Ok(Self::Any { children })
    }

    pub(super) fn direct(&self) -> Expr {
        match self {
            Self::Mapped { predicate, .. } => predicate.direct(),
            Self::Compare {
                slot,
                operator,
                value,
            } => {
                let left = Expr::Column(Column::new_unqualified(slot.column()));
                let right = lit(lower::scalar(value));
                match operator {
                    Comparison::Eq => left.eq(right),
                    Comparison::NotEq => left.not_eq(right),
                    Comparison::Lt => left.lt(right),
                    Comparison::LtEq => left.lt_eq(right),
                    Comparison::Gt => left.gt(right),
                    Comparison::GtEq => left.gt_eq(right),
                }
            }
            Self::IsNull { slot, negated } => {
                let column = Expr::Column(Column::new_unqualified(slot.column()));
                if *negated {
                    column.is_not_null()
                } else {
                    column.is_null()
                }
            }
            Self::Not { inner } => !inner.direct(),
            Self::All { children } | Self::Any { children } => children
                .iter()
                .map(Self::direct)
                .reduce(|left, right| {
                    if matches!(self, Self::All { .. }) {
                        left.and(right)
                    } else {
                        left.or(right)
                    }
                })
                .expect("checked nonempty Boolean expression"),
        }
    }

    pub(super) fn sql(&self, alias: &str, parameters: &mut Vec<Literal>) -> ast::Expr {
        match self {
            Self::Mapped { predicate, .. } => predicate.sql(alias, parameters),
            Self::Compare {
                slot,
                operator,
                value,
            } => {
                parameters.push(value.clone());
                let right = ast::Expr::Cast {
                    kind: ast::CastKind::Cast,
                    expr: Box::new(ast::Expr::Value(
                        ast::Value::Placeholder(format!("${}", parameters.len())).into(),
                    )),
                    data_type: literal::sql_type(value),
                    format: None,
                    array: false,
                };
                ast::Expr::BinaryOp {
                    left: Box::new(sql_column(alias, slot.column())),
                    op: match operator {
                        Comparison::Eq => ast::BinaryOperator::Eq,
                        Comparison::NotEq => ast::BinaryOperator::NotEq,
                        Comparison::Lt => ast::BinaryOperator::Lt,
                        Comparison::LtEq => ast::BinaryOperator::LtEq,
                        Comparison::Gt => ast::BinaryOperator::Gt,
                        Comparison::GtEq => ast::BinaryOperator::GtEq,
                    },
                    right: Box::new(right),
                }
            }
            Self::IsNull { slot, negated } => {
                let column = Box::new(sql_column(alias, slot.column()));
                if *negated {
                    ast::Expr::IsNotNull(column)
                } else {
                    ast::Expr::IsNull(column)
                }
            }
            Self::Not { inner } => ast::Expr::UnaryOp {
                op: ast::UnaryOperator::Not,
                expr: Box::new(ast::Expr::Nested(Box::new(inner.sql(alias, parameters)))),
            },
            Self::All { children } | Self::Any { children } => children
                .iter()
                .map(|child| child.sql(alias, parameters))
                .reduce(|left, right| ast::Expr::BinaryOp {
                    left: Box::new(left),
                    op: if matches!(self, Self::All { .. }) {
                        ast::BinaryOperator::And
                    } else {
                        ast::BinaryOperator::Or
                    },
                    right: Box::new(right),
                })
                .expect("checked nonempty Boolean expression"),
        }
    }
}

/// One graph output conditional. Both branches are exact typed, non-null
/// literals; a nullable predicate still selects the ELSE branch on UNKNOWN.
/// The SQL planner conservatively marks a CASE over parameters nullable, so
/// the portable slot contract also permits nullability.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CheckedConditional {
    predicate: CheckedPredicate,
    then_value: Literal,
    else_value: Literal,
    result_type: DataType,
    nullable: bool,
}

impl CheckedConditional {
    pub(super) fn new(
        scope: &str,
        predicate: CheckedPredicate,
        then_value: Literal,
        else_value: Literal,
    ) -> Result<Self, CompileDiagnostic> {
        if !predicate.in_scope(scope) {
            return Err(diagnostic(
                "graph_conditional_scope",
                "Graph conditional predicate must read one input scope",
            ));
        }
        let result_type = literal::checked_scalar(&then_value)?.data_type();
        if literal::checked_scalar(&else_value)?.data_type() != result_type {
            return Err(diagnostic(
                "graph_conditional_type",
                "Graph conditional branches require the same exact scalar type",
            ));
        }
        Ok(Self {
            predicate,
            then_value,
            else_value,
            result_type,
            nullable: true,
        })
    }

    pub(super) fn result_type(&self) -> &DataType {
        &self.result_type
    }

    pub(super) fn nullable(&self) -> bool {
        self.nullable
    }

    pub(super) fn direct(&self) -> Result<Expr, CompileDiagnostic> {
        datafusion::logical_expr::when(
            self.predicate.direct(),
            lit(literal::checked_scalar(&self.then_value)?),
        )
        .otherwise(lit(literal::checked_scalar(&self.else_value)?))
        .map_err(lower::backend_error)
    }

    pub(super) fn sql(&self, alias: &str, parameters: &mut Vec<Literal>) -> ast::Expr {
        let mut statements = Parser::parse_sql(
            &GenericDialect {},
            "SELECT CASE WHEN true THEN 0 ELSE 1 END",
        )
        .expect("static checked conditional skeleton");
        let ast::Statement::Query(query) = &mut statements[0] else {
            unreachable!()
        };
        let ast::SetExpr::Select(select) = query.body.as_mut() else {
            unreachable!()
        };
        let ast::SelectItem::UnnamedExpr(ast::Expr::Case {
            conditions,
            else_result,
            ..
        }) = &mut select.projection[0]
        else {
            unreachable!()
        };
        conditions[0].condition = self.predicate.sql(alias, parameters);
        conditions[0].result = parameter_expr(&self.then_value, parameters);
        *else_result = Some(Box::new(parameter_expr(&self.else_value, parameters)));
        let ast::SelectItem::UnnamedExpr(expression) = select.projection.remove(0) else {
            unreachable!()
        };
        expression
    }
}

fn parameter_expr(value: &Literal, parameters: &mut Vec<Literal>) -> ast::Expr {
    parameters.push(value.clone());
    ast::Expr::Cast {
        kind: ast::CastKind::Cast,
        expr: Box::new(ast::Expr::Value(
            ast::Value::Placeholder(format!("${}", parameters.len())).into(),
        )),
        data_type: literal::sql_type(value),
        format: None,
        array: false,
    }
}

fn sql_column(alias: &str, name: &str) -> ast::Expr {
    ast::Expr::CompoundIdentifier(vec![
        ast::Ident::with_quote('"', alias),
        ast::Ident::with_quote('"', name),
    ])
}

/// The first consumer-backed exact cast profile. The closed constructor
/// prevents implicit float conversion, truncation, or cross-scope slot reads.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CheckedCast {
    input: CheckedSlot,
    result_type: DataType,
    nullable: bool,
}

impl CheckedCast {
    pub(super) fn int64_to_decimal128_scale0(
        scope: &str,
        input: CheckedSlot,
    ) -> Result<Self, CompileDiagnostic> {
        if input.scope != scope {
            return Err(diagnostic(
                "graph_cast_scope",
                "Graph cast must read a slot from its input node",
            ));
        }
        if input.field.data_type() != &DataType::Int64 {
            return Err(diagnostic(
                "graph_cast_type",
                "Graph cast requires an exact Int64 input",
            ));
        }
        Ok(Self {
            nullable: input.field.is_nullable(),
            input,
            result_type: DataType::Decimal128(38, 0),
        })
    }

    pub(super) fn result_type(&self) -> &DataType {
        &self.result_type
    }

    pub(super) fn nullable(&self) -> bool {
        self.nullable
    }

    pub(super) fn direct(&self) -> Expr {
        datafusion::logical_expr::cast(
            Expr::Column(Column::new_unqualified(self.input.column())),
            self.result_type.clone(),
        )
    }

    pub(super) fn sql(&self, alias: &str) -> ast::Expr {
        ast::Expr::Cast {
            kind: ast::CastKind::Cast,
            expr: Box::new(sql_column(alias, self.input.column())),
            data_type: ast::DataType::Decimal(ast::ExactNumberInfo::PrecisionAndScale(38, 0)),
            format: None,
            array: false,
        }
    }
}

/// A Boolean value projection, distinct from a filtering predicate. The
/// narrow Int64 signature makes physical input checks explicit at binding.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CheckedNullTest {
    input: CheckedSlot,
    negated: bool,
    result_type: DataType,
    nullable: bool,
}

impl CheckedNullTest {
    pub(super) fn int64(
        scope: &str,
        input: CheckedSlot,
        negated: bool,
    ) -> Result<Self, CompileDiagnostic> {
        if input.scope != scope {
            return Err(diagnostic(
                "graph_null_test_scope",
                "Graph null test must read a slot from its input node",
            ));
        }
        if input.field.data_type() != &DataType::Int64 {
            return Err(diagnostic(
                "graph_null_test_type",
                "Graph null test requires an exact Int64 input",
            ));
        }
        Ok(Self {
            input,
            negated,
            result_type: DataType::Boolean,
            nullable: false,
        })
    }

    pub(super) fn result_type(&self) -> &DataType {
        &self.result_type
    }

    pub(super) fn nullable(&self) -> bool {
        self.nullable
    }

    pub(super) fn direct(&self) -> Expr {
        let column = Expr::Column(Column::new_unqualified(self.input.column()));
        if self.negated {
            column.is_not_null()
        } else {
            column.is_null()
        }
    }

    pub(super) fn sql(&self, alias: &str) -> ast::Expr {
        let column = Box::new(sql_column(alias, self.input.column()));
        if self.negated {
            ast::Expr::IsNotNull(column)
        } else {
            ast::Expr::IsNull(column)
        }
    }
}

/// A value-producing, two-slot comparison within one graph input node. It
/// retains SQL three-valued logic when either Int64 input is nullable.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CheckedSlotComparison {
    left: CheckedSlot,
    right: CheckedSlot,
    operator: Comparison,
    result_type: DataType,
    nullable: bool,
}

impl CheckedSlotComparison {
    pub(super) fn integer(
        scope: &str,
        left: CheckedSlot,
        right: CheckedSlot,
        operator: Comparison,
    ) -> Result<Self, CompileDiagnostic> {
        if left.scope != scope || right.scope != scope {
            return Err(diagnostic(
                "graph_comparison_scope",
                "Graph slot comparison operands must belong to one input node",
            ));
        }
        if left.field.data_type() != right.field.data_type()
            || !matches!(
                left.field.data_type(),
                DataType::Int8
                    | DataType::Int16
                    | DataType::Int32
                    | DataType::Int64
                    | DataType::UInt8
                    | DataType::UInt16
                    | DataType::UInt32
                    | DataType::UInt64
            )
        {
            return Err(diagnostic(
                "graph_comparison_type",
                "Graph slot comparison requires two operands of the same exact integer type",
            ));
        }
        Ok(Self {
            nullable: left.field.is_nullable() || right.field.is_nullable(),
            left,
            right,
            operator,
            result_type: DataType::Boolean,
        })
    }

    pub(super) fn result_type(&self) -> &DataType {
        &self.result_type
    }

    pub(super) fn nullable(&self) -> bool {
        self.nullable
    }

    pub(super) fn direct(&self) -> Expr {
        let left = Expr::Column(Column::new_unqualified(self.left.column()));
        let right = Expr::Column(Column::new_unqualified(self.right.column()));
        match self.operator {
            Comparison::Eq => left.eq(right),
            Comparison::NotEq => left.not_eq(right),
            Comparison::Lt => left.lt(right),
            Comparison::LtEq => left.lt_eq(right),
            Comparison::Gt => left.gt(right),
            Comparison::GtEq => left.gt_eq(right),
        }
    }

    pub(super) fn sql(&self, alias: &str) -> ast::Expr {
        ast::Expr::BinaryOp {
            left: Box::new(sql_column(alias, self.left.column())),
            op: match self.operator {
                Comparison::Eq => ast::BinaryOperator::Eq,
                Comparison::NotEq => ast::BinaryOperator::NotEq,
                Comparison::Lt => ast::BinaryOperator::Lt,
                Comparison::LtEq => ast::BinaryOperator::LtEq,
                Comparison::Gt => ast::BinaryOperator::Gt,
                Comparison::GtEq => ast::BinaryOperator::GtEq,
            },
            right: Box::new(sql_column(alias, self.right.column())),
        }
    }
}

/// The registry ID is part of the executable expression contract. More scalar
/// calls enter here only with a checked signature and an immediate consumer.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(super) enum CheckedScalarCall {
    RatioI64V1 {
        numerator: CheckedSlot,
        denominator: CheckedSlot,
        zero_on_zero: bool,
        result_type: DataType,
        nullable: bool,
    },
}

/// Finalizes the nullable exact weighted-mean state under an authored zero
/// policy. The fallback is a Decimal value with the same precision and scale,
/// never an integer coerced by the backend.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CheckedDecimalZeroFinalizer {
    pub(super) result_type: DataType,
    pub(super) nullable: bool,
}

impl CheckedDecimalZeroFinalizer {
    pub(super) fn new(output: &Field) -> Result<Self, CompileDiagnostic> {
        if output.data_type() != &DataType::Decimal128(38, 18) || output.is_nullable() {
            return Err(diagnostic(
                "weighted_zero_type",
                "Weighted zero finalization requires a nonnullable Decimal128(38,18) output",
            ));
        }
        Ok(Self {
            result_type: DataType::Decimal128(38, 18),
            nullable: false,
        })
    }

    pub(super) fn valid_for(&self, output: &Field) -> bool {
        self.result_type == DataType::Decimal128(38, 18)
            && !self.nullable
            && output.data_type() == &self.result_type
            && output.is_nullable() == self.nullable
    }

    pub(super) fn direct(&self, input: Expr) -> Expr {
        datafusion::functions::core::coalesce().call(vec![
            input,
            lit(datafusion::common::ScalarValue::Decimal128(Some(0), 38, 18)),
        ])
    }

    pub(super) fn sql(&self, input: ast::Expr) -> ast::Expr {
        let mut wrapper = Parser::parse_sql(
            &GenericDialect {},
            "SELECT coalesce(x, CAST(0 AS DECIMAL(38,18)))",
        )
        .expect("static checked Decimal zero finalizer");
        let ast::Statement::Query(query) = &mut wrapper[0] else {
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
        arguments.args[0] = ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(input));
        ast::Expr::Function(function.clone())
    }
}

impl CheckedScalarCall {
    pub(super) fn ratio_i64_v1(
        numerator: CheckedSlot,
        denominator: CheckedSlot,
        zero_on_zero: bool,
    ) -> Result<Self, CompileDiagnostic> {
        if numerator.scope != denominator.scope
            || numerator.field.data_type() != &DataType::Int64
            || denominator.field.data_type() != &DataType::Int64
        {
            return Err(diagnostic(
                "graph_ratio_type",
                "Graph ratios require exact Int64 components in one input scope",
            ));
        }
        Ok(Self::RatioI64V1 {
            numerator,
            denominator,
            zero_on_zero,
            result_type: DataType::Decimal128(38, 18),
            nullable: true,
        })
    }

    pub(super) fn result_type(&self) -> &DataType {
        match self {
            Self::RatioI64V1 { result_type, .. } => result_type,
        }
    }

    pub(super) fn nullable(&self) -> bool {
        match self {
            Self::RatioI64V1 { nullable, .. } => *nullable,
        }
    }

    pub(super) fn direct(&self) -> Expr {
        match self {
            Self::RatioI64V1 {
                numerator,
                denominator,
                zero_on_zero,
                ..
            } => semantic_engine::semantic_ratio_i64_v1().call(vec![
                Expr::Column(Column::new_unqualified(numerator.column())),
                Expr::Column(Column::new_unqualified(denominator.column())),
                lit(*zero_on_zero),
            ]),
        }
    }

    pub(super) fn sql(&self, alias: &str) -> ast::Expr {
        match self {
            Self::RatioI64V1 {
                numerator,
                denominator,
                zero_on_zero,
                ..
            } => {
                let mut statement = Parser::parse_sql(
                    &GenericDialect {},
                    "SELECT semantic_ratio_i64_v1(x,y,false)",
                )
                .expect("static registered-scalar skeleton");
                let ast::Statement::Query(query) = &mut statement[0] else {
                    unreachable!()
                };
                let ast::SetExpr::Select(select) = query.body.as_mut() else {
                    unreachable!()
                };
                let ast::SelectItem::UnnamedExpr(ast::Expr::Function(function)) =
                    &mut select.projection[0]
                else {
                    unreachable!()
                };
                let ast::FunctionArguments::List(arguments) = &mut function.args else {
                    unreachable!()
                };
                arguments.args = [
                    sql_column(alias, numerator.column()),
                    sql_column(alias, denominator.column()),
                    ast::Expr::Value(ast::Value::Boolean(*zero_on_zero).into()),
                ]
                .into_iter()
                .map(|expr| ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(expr)))
                .collect();
                ast::Expr::Function(function.clone())
            }
        }
    }
}
