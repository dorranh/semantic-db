//! Checked decimal multiplication with one explicit final quantization.
use datafusion::{
    arrow::{
        array::{Array, Decimal128Array},
        datatypes::{DataType, Field, FieldRef, i256},
    },
    common::{Result, ScalarValue},
    logical_expr::{
        ColumnarValue, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature,
        Volatility,
    },
};
use semantic_runtime::SemanticDataCondition;
use std::sync::{Arc, OnceLock};
fn error(message: &str) -> datafusion::error::DataFusionError {
    semantic_runtime::failure(message)
}
fn control(value: &ScalarValue) -> Result<i64> {
    match value {
        ScalarValue::Int64(Some(v)) => Ok(*v),
        ScalarValue::Int32(Some(v)) => Ok(i64::from(*v)),
        _ => Err(error(
            "decimal quantization controls must be constant exact integers",
        )),
    }
}
fn controls(p: i64, s: i64, r: i64) -> Result<(u8, i8)> {
    if !(1..=38).contains(&p) || !(0..=p).contains(&s) || !(0..=2).contains(&r) {
        return Err(error(
            "decimal quantization precision, scale or rounding is invalid",
        ));
    }
    Ok((p as u8, s as i8))
}
fn power(exponent: u32) -> Result<i256> {
    (0..exponent).try_fold(i256::from_i128(1), |n, _| {
        n.checked_mul(i256::from_i128(10))
            .ok_or_else(|| error("decimal scale overflow"))
    })
}
/// Exact coefficient quantization; no intermediate truncation or binary floats.
fn quantize(
    amount: i128,
    rate: i128,
    amount_scale: i8,
    rate_scale: i8,
    p: u8,
    s: i8,
    r: i64,
) -> Result<i128> {
    let mut numerator = i256::from_i128(amount)
        .checked_mul(i256::from_i128(rate))
        .ok_or_else(|| error("decimal rate multiplication overflow"))?;
    let shift = i32::from(s) - i32::from(amount_scale) - i32::from(rate_scale);
    let divisor = if shift >= 0 {
        numerator = numerator
            .checked_mul(power(shift as u32)?)
            .ok_or_else(|| error("decimal rate scale overflow"))?;
        i256::from_i128(1)
    } else {
        power((-shift) as u32)?
    };
    let mut quotient = numerator
        .checked_div(divisor)
        .ok_or_else(|| error("decimal rate division overflow"))?;
    let remainder = numerator
        .checked_rem(divisor)
        .ok_or_else(|| error("decimal rate remainder overflow"))?;
    let absolute = remainder
        .checked_abs()
        .ok_or_else(|| error("decimal rate remainder overflow"))?;
    let doubled = absolute
        .checked_mul(i256::from_i128(2))
        .ok_or_else(|| error("decimal rate rounding overflow"))?;
    let odd = quotient
        .checked_rem(i256::from_i128(2))
        .ok_or_else(|| error("decimal rate rounding overflow"))?
        != i256::ZERO;
    if r != 0 && (doubled > divisor || (doubled == divisor && (r == 2 || odd))) {
        quotient = quotient
            .checked_add(i256::from_i128(if numerator < i256::ZERO { -1 } else { 1 }))
            .ok_or_else(|| error("decimal rate rounding overflow"))?;
    }
    let value = quotient
        .to_i128()
        .ok_or_else(|| error("decimal rate result overflow"))?;
    if value.unsigned_abs() >= 10_u128.pow(u32::from(p)) {
        return Err(error("decimal rate result precision overflow"));
    }
    Ok(value)
}
#[derive(Debug, PartialEq, Eq, Hash)]
struct DecimalRate {
    signature: Signature,
}
impl ScalarUDFImpl for DecimalRate {
    fn name(&self) -> &str {
        "semantic_decimal_rate_v1"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        if types.len()!=5 || !types[..2].iter().all(|t|matches!(t,DataType::Decimal128(p,s) if *p>0 && *p<=38 && *s>=0 && *s<=*p as i8)) || !types[2..].iter().all(DataType::is_signed_integer) { return Err(error("decimal rate requires two exact Decimal128 values and integer controls")); }
        Ok(vec![
            types[0].clone(),
            types[1].clone(),
            DataType::Int64,
            DataType::Int64,
            DataType::Int64,
        ])
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Err(error(
            "decimal rate output requires constant precision and scale",
        ))
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        self.coerce_types(
            &args
                .arg_fields
                .iter()
                .map(|f| f.data_type().clone())
                .collect::<Vec<_>>(),
        )?;
        let value = |i| {
            args.scalar_arguments
                .get(i)
                .and_then(|v| *v)
                .ok_or_else(|| error("decimal rate controls must be constant"))
                .and_then(control)
        };
        let (p, s) = controls(value(2)?, value(3)?, value(4)?)?;
        Ok(Arc::new(Field::new(
            self.name(),
            DataType::Decimal128(p, s),
            // SQL planning happens before decimal amount parameters are bound.
            // Keep the same conservative metadata for literal and parameter forms;
            // a nonnull result is valid under this nullable contract as well.
            true,
        )))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        if args.args.len() != 5 {
            return Err(error("decimal rate requires five arguments"));
        }
        self.coerce_types(
            &args
                .args
                .iter()
                .map(ColumnarValue::data_type)
                .collect::<Vec<_>>(),
        )?;
        let value = |i| match &args.args[i] {
            ColumnarValue::Scalar(v) => control(v),
            _ => Err(error("decimal rate controls must be constant")),
        };
        let rounding = value(4)?;
        let (p, s) = controls(value(2)?, value(3)?, rounding)?;
        if args.return_field.data_type() != &DataType::Decimal128(p, s) {
            return Err(error(
                "decimal rate output schema differs from its controls",
            ));
        }
        for argument in &args.args[..2] {
            if let ColumnarValue::Array(array) = argument
                && array.len() != args.number_rows
            {
                return Err(error(
                    "decimal rate array length differs from execution batch",
                ));
            }
        }
        let arrays = ColumnarValue::values_to_arrays(&args.args[..2])?;
        let amount = arrays[0]
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .ok_or_else(|| error("decimal rate amount type"))?;
        let rate = arrays[1]
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .ok_or_else(|| error("decimal rate value type"))?;
        if amount.len() != rate.len() {
            return Err(error("decimal rate argument lengths differ"));
        }
        let values = (0..amount.len())
            .map(|i| {
                if !amount.is_null(i)
                    && amount.value(i).unsigned_abs() >= 10_u128.pow(u32::from(amount.precision()))
                {
                    return Err(error(
                        "decimal rate amount coefficient exceeds its declared precision",
                    ));
                }
                if !rate.is_null(i)
                    && rate.value(i).unsigned_abs() >= 10_u128.pow(u32::from(rate.precision()))
                {
                    return Err(SemanticDataCondition::InvalidRate.into_datafusion());
                }
                if rate.is_null(i) {
                    return Err(SemanticDataCondition::RateValueMissing.into_datafusion());
                }
                if rate.value(i) <= 0 {
                    return Err(SemanticDataCondition::InvalidRate.into_datafusion());
                }
                if amount.is_null(i) {
                    if !args.return_field.is_nullable() {
                        return Err(error(
                            "decimal rate null violates its declared output schema",
                        ));
                    }
                    return Ok(None);
                }
                quantize(
                    amount.value(i),
                    rate.value(i),
                    amount.scale(),
                    rate.scale(),
                    p,
                    s,
                    rounding,
                )
                .map(Some)
            })
            .collect::<Result<Vec<_>>>()?;
        let result = Arc::new(Decimal128Array::from(values).with_precision_and_scale(p, s)?)
            as datafusion::arrow::array::ArrayRef;
        if args.args[..2]
            .iter()
            .all(|a| matches!(a, ColumnarValue::Scalar(_)))
        {
            Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                &result, 0,
            )?))
        } else {
            Ok(ColumnarValue::Array(result))
        }
    }
}
/// Closed local decimal-rate function; output controls must be immutable literals.
pub fn semantic_decimal_rate_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(ScalarUDF::from(DecimalRate {
                signature: Signature::user_defined(Volatility::Immutable),
            }))
        })
        .clone()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_quantization_is_exact_across_ties_scales_and_precision() {
        for (a, r, expected) in [
            (100, 950050, 9501),
            (-100, 950050, -9501),
            (100, 950049, 9500),
            (100, 950051, 9501),
            (0, 950050, 0),
        ] {
            assert_eq!(quantize(a, r, 0, 6, 18, 2, 2).unwrap(), expected);
        }
        assert_eq!(quantize(100, 950050, 0, 6, 18, 2, 1).unwrap(), 9500);
        assert_eq!(quantize(100, -950050, 0, 6, 18, 2, 2).unwrap(), -9501);
        assert_eq!(quantize(10000, 950050, 2, 6, 18, 2, 2).unwrap(), 9501);
        assert_eq!(quantize(1, 1, 0, 0, 3, 2, 0).unwrap(), 100);
        assert!(quantize(i128::MAX, i128::MAX, 0, 0, 38, 0, 2).is_err());
        assert!(quantize(100, 1, 0, 0, 2, 0, 2).is_err());
    }
    fn invoke(arguments: Vec<ColumnarValue>) -> Result<ColumnarValue> {
        semantic_decimal_rate_v1().invoke_with_args(ScalarFunctionArgs {
            args: arguments,
            arg_fields: vec![],
            number_rows: 3,
            return_field: Arc::new(Field::new("converted", DataType::Decimal128(18, 2), true)),
            config_options: Arc::new(datafusion::config::ConfigOptions::default()),
        })
    }
    #[test]
    fn scalar_array_broadcast_and_bypass_validation_are_checked() {
        let controls = || {
            vec![
                ColumnarValue::Scalar(ScalarValue::Int64(Some(18))),
                ColumnarValue::Scalar(ScalarValue::Int64(Some(2))),
                ColumnarValue::Scalar(ScalarValue::Int64(Some(2))),
            ]
        };
        let mut args = vec![
            ColumnarValue::Scalar(ScalarValue::Decimal128(Some(100), 3, 0)),
            ColumnarValue::Array(Arc::new(
                Decimal128Array::from(vec![1_000_000, 950_050, 900_000])
                    .with_precision_and_scale(18, 6)
                    .unwrap(),
            )),
        ];
        args.extend(controls());
        let ColumnarValue::Array(result) = invoke(args).unwrap() else {
            panic!("broadcast result")
        };
        assert_eq!(
            result
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .unwrap()
                .values()
                .as_ref(),
            &[10000, 9501, 9000]
        );
        let mut invalid = vec![
            ColumnarValue::Scalar(ScalarValue::Decimal128(Some(100), 3, -1)),
            ColumnarValue::Scalar(ScalarValue::Decimal128(Some(1), 3, 0)),
        ];
        invalid.extend(controls());
        assert!(invoke(invalid).is_err());
        let mut nullrate = vec![
            ColumnarValue::Scalar(ScalarValue::Decimal128(Some(100), 3, 0)),
            ColumnarValue::Scalar(ScalarValue::Decimal128(None, 18, 6)),
        ];
        nullrate.extend(controls());
        let error = invoke(nullrate).unwrap_err();
        assert!(
            matches!(error,datafusion::error::DataFusionError::External(error) if error.downcast_ref::<SemanticDataCondition>()==Some(&SemanticDataCondition::RateValueMissing))
        );
        let mut mismatched = vec![
            ColumnarValue::Array(Arc::new(
                Decimal128Array::from(vec![100, 200])
                    .with_precision_and_scale(3, 0)
                    .unwrap(),
            )),
            ColumnarValue::Array(Arc::new(
                Decimal128Array::from(vec![1])
                    .with_precision_and_scale(3, 0)
                    .unwrap(),
            )),
        ];
        mismatched.extend(controls());
        assert!(invoke(mismatched).is_err());
    }
    #[test]
    fn input_coefficients_batch_lengths_and_nonnullable_output_are_enforced() {
        let run = |amount: ColumnarValue, rate: ColumnarValue, rows: usize, nullable: bool| {
            semantic_decimal_rate_v1().invoke_with_args(ScalarFunctionArgs {
                args: vec![
                    amount,
                    rate,
                    ColumnarValue::Scalar(ScalarValue::Int64(Some(18))),
                    ColumnarValue::Scalar(ScalarValue::Int64(Some(2))),
                    ColumnarValue::Scalar(ScalarValue::Int64(Some(2))),
                ],
                arg_fields: vec![],
                number_rows: rows,
                return_field: Arc::new(Field::new("value", DataType::Decimal128(18, 2), nullable)),
                config_options: Arc::new(datafusion::config::ConfigOptions::default()),
            })
        };
        let amount = || ColumnarValue::Scalar(ScalarValue::Decimal128(Some(1), 1, 0));
        let rate = || ColumnarValue::Scalar(ScalarValue::Decimal128(Some(1), 1, 0));
        assert!(
            run(
                ColumnarValue::Scalar(ScalarValue::Decimal128(Some(1000), 1, 0)),
                rate(),
                1,
                true
            )
            .is_err()
        );
        assert!(
            run(
                amount(),
                ColumnarValue::Scalar(ScalarValue::Decimal128(Some(1000), 1, 0)),
                1,
                true
            )
            .is_err()
        );
        assert!(
            run(
                ColumnarValue::Scalar(ScalarValue::Decimal128(None, 1, 0)),
                rate(),
                1,
                false
            )
            .is_err()
        );
        let array = || {
            ColumnarValue::Array(Arc::new(
                Decimal128Array::from(vec![1, 2])
                    .with_precision_and_scale(1, 0)
                    .unwrap(),
            ))
        };
        assert!(run(array(), rate(), 3, true).is_err());
        assert!(run(amount(), array(), 3, true).is_err());
    }
}
