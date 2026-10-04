//! Integer scalar arithmetic is checked before optimizer constant folding.
//! The analyzer runs after ordinary type coercion, preserving its chosen width.
use datafusion::{
    arrow::{
        compute::kernels::numeric,
        datatypes::{DataType, Field, FieldRef},
    },
    common::{
        DFSchema, Result, ScalarValue,
        config::ConfigOptions,
        tree_node::{Transformed, TreeNode},
    },
    logical_expr::{
        ColumnarValue, Expr, ExprSchemable, LogicalPlan, Operator, ReturnFieldArgs,
        ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility, utils::merge_schema,
    },
    optimizer::{analyzer::AnalyzerRule, utils::NamePreserver},
};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct CheckedIntegerArithmetic;

impl AnalyzerRule for CheckedIntegerArithmetic {
    fn name(&self) -> &str {
        "semantic_checked_integer_arithmetic_v1"
    }

    fn analyze(&self, plan: LogicalPlan, _: &ConfigOptions) -> Result<LogicalPlan> {
        Ok(plan
            .transform_up_with_subqueries(|plan| {
                let mut schema = merge_schema(&plan.inputs());
                if let LogicalPlan::TableScan(scan) = &plan {
                    schema.merge(&DFSchema::try_from_qualified_schema(
                        scan.table_name.clone(),
                        &scan.source.schema(),
                    )?);
                }
                let names = NamePreserver::new(&plan);
                plan.map_expressions(|expr| {
                    let name = names.save(&expr);
                    Ok(expr
                        .transform_up(|expr| {
                            if let Expr::Negative(value) = &expr {
                                let data_type = value.get_type(&schema)?;
                                if matches!(
                                    data_type,
                                    DataType::Int8
                                        | DataType::Int16
                                        | DataType::Int32
                                        | DataType::Int64
                                ) {
                                    return Ok(Transformed::yes(
                                        negation(data_type).call(vec![*value.clone()]),
                                    ));
                                }
                            }
                            if let Expr::BinaryExpr(binary) = &expr
                                && matches!(
                                    binary.op,
                                    Operator::Plus | Operator::Minus | Operator::Multiply
                                )
                            {
                                let left = binary.left.get_type(&schema)?;
                                let right = binary.right.get_type(&schema)?;
                                if left.is_integer() && left == right {
                                    let checked = function(binary.op, left);
                                    return Ok(Transformed::yes(
                                        checked.call(vec![
                                            *binary.left.clone(),
                                            *binary.right.clone(),
                                        ]),
                                    ));
                                }
                            }
                            Ok(Transformed::no(expr))
                        })?
                        .update_data(|expr| name.restore(expr)))
                })
            })?
            .data)
    }
}

fn negation(data_type: DataType) -> ScalarUDF {
    checked_function(None, data_type)
}

fn function(operator: Operator, data_type: DataType) -> ScalarUDF {
    checked_function(Some(operator), data_type)
}

fn checked_function(operator: Option<Operator>, data_type: DataType) -> ScalarUDF {
    let operation = match operator {
        Some(Operator::Plus) => "add",
        Some(Operator::Minus) => "sub",
        Some(Operator::Multiply) => "mul",
        None => "neg",
        _ => unreachable!("closed checked arithmetic profile"),
    };
    let arity = if operator.is_some() { 2 } else { 1 };
    ScalarUDF::from(CheckedArithmetic {
        name: format!(
            "semantic_checked_{operation}_{}_v1",
            data_type.to_string().to_lowercase()
        ),
        signature: Signature::exact(vec![data_type.clone(); arity], Volatility::Immutable),
        data_type,
        operator,
    })
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct CheckedArithmetic {
    name: String,
    signature: Signature,
    data_type: DataType,
    operator: Option<Operator>,
}

impl ScalarUDFImpl for CheckedArithmetic {
    fn name(&self) -> &str {
        &self.name
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(self.data_type.clone())
    }
    fn return_field_from_args(&self, arguments: ReturnFieldArgs) -> Result<FieldRef> {
        let mut field = Field::new(
            self.name(),
            self.data_type.clone(),
            arguments.arg_fields.iter().any(|field| field.is_nullable()),
        );
        if self.operator.is_none() {
            field = field.with_metadata(arguments.arg_fields[0].metadata().clone());
        }
        Ok(Arc::new(field))
    }
    fn invoke_with_args(&self, arguments: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arguments = &arguments.args;
        let arrays = ColumnarValue::values_to_arrays(arguments)?;
        let result = match self.operator {
            Some(Operator::Plus) => numeric::add(&arrays[0], &arrays[1]),
            Some(Operator::Minus) => numeric::sub(&arrays[0], &arrays[1]),
            Some(Operator::Multiply) => numeric::mul(&arrays[0], &arrays[1]),
            None => numeric::neg(arrays[0].as_ref()),
            _ => unreachable!("closed checked arithmetic profile"),
        }?;
        if arguments
            .iter()
            .all(|argument| matches!(argument, ColumnarValue::Scalar(_)))
        {
            Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                result.as_ref(),
                0,
            )?))
        } else {
            Ok(ColumnarValue::Array(result))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_checked_integer_function_has_an_explicit_local_profile_contract() {
        for operator in [Operator::Plus, Operator::Minus, Operator::Multiply] {
            for data_type in [
                DataType::Int8,
                DataType::Int16,
                DataType::Int32,
                DataType::Int64,
                DataType::UInt8,
                DataType::UInt16,
                DataType::UInt32,
                DataType::UInt64,
            ] {
                let udf = function(operator, data_type);
                let capability = crate::MVP_EXECUTION_PROFILE
                    .compiler_function(crate::FunctionKind::Scalar, udf.name())
                    .expect("checked arithmetic must not be shipped to a remote backend");
                assert_eq!(capability.placement, crate::FunctionPlacement::LocalOnly);
            }
        }
        for data_type in [
            DataType::Int8,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
        ] {
            let udf = negation(data_type);
            let capability = crate::MVP_EXECUTION_PROFILE
                .compiler_function(crate::FunctionKind::Scalar, udf.name())
                .expect("checked negation must remain local");
            assert_eq!(capability.placement, crate::FunctionPlacement::LocalOnly);
        }
    }
}
