//! Exact rational Int64 conversion to Decimal128(38,18).

use datafusion::{
    arrow::{
        array::{Array, BooleanArray, Decimal128Array, Int64Array},
        datatypes::{DataType, i256},
    },
    common::ScalarValue,
    logical_expr::{ColumnarValue, ScalarUDF, Volatility, create_udf},
};
use std::sync::{Arc, OnceLock};

fn error(message: &str) -> datafusion::error::DataFusionError {
    semantic_runtime::failure(message)
}

/// Arguments are (value, numerator, positive denominator, half_even).
/// Null inputs propagate. Quotients truncate toward zero unless half-even
/// rounding is explicitly selected. Invalid factors and overflow fail query
/// execution, even for callers bypassing the semantic binder.
pub fn semantic_scale_i64_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_scale_i64_v1",
                vec![
                    DataType::Int64,
                    DataType::Int64,
                    DataType::Int64,
                    DataType::Boolean,
                ],
                DataType::Decimal128(38, 18),
                Volatility::Immutable,
                Arc::new(|args| {
                    if args.len() != 4 {
                        return Err(error("unit conversion requires four arguments"));
                    }
                    let arrays = ColumnarValue::values_to_arrays(args)?;
                    let values = arrays[0]
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| error("unit conversion value type"))?;
                    let numerators = arrays[1]
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| error("unit conversion numerator type"))?;
                    let denominators = arrays[2]
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| error("unit conversion denominator type"))?;
                    let roundings = arrays[3]
                        .as_any()
                        .downcast_ref::<BooleanArray>()
                        .ok_or_else(|| error("unit conversion rounding type"))?;
                    let coefficients = (0..values.len())
                        .map(|index| {
                            if arrays.iter().any(|array| array.is_null(index)) {
                                return Ok(None);
                            }
                            let numerator = numerators.value(index);
                            let denominator = denominators.value(index);
                            if numerator <= 0 || denominator <= 0 {
                                return Err(error("unit conversion factor is invalid"));
                            }
                            let scaled = i256::from_i128(i128::from(values.value(index)))
                                .checked_mul(i256::from_i128(i128::from(numerator)))
                                .and_then(|value| {
                                    value.checked_mul(i256::from_i128(1_000_000_000_000_000_000))
                                })
                                .ok_or_else(|| error("unit conversion overflow"))?;
                            let divisor = i256::from_i128(i128::from(denominator));
                            let quotient = scaled
                                .checked_div(divisor)
                                .ok_or_else(|| error("unit conversion overflow"))?;
                            let remainder = scaled
                                .checked_rem(divisor)
                                .ok_or_else(|| error("unit conversion overflow"))?;
                            let mut coefficient = quotient
                                .to_i128()
                                .ok_or_else(|| error("unit conversion overflow"))?;
                            if roundings.value(index) {
                                let rest = remainder
                                    .to_i128()
                                    .ok_or_else(|| error("unit conversion overflow"))?;
                                let doubled = rest.unsigned_abs() * 2;
                                let denominator = u128::from(denominator.unsigned_abs());
                                if doubled > denominator
                                    || (doubled == denominator
                                        && coefficient.unsigned_abs() % 2 == 1)
                                {
                                    coefficient = coefficient
                                        .checked_add(if scaled.is_negative() { -1 } else { 1 })
                                        .ok_or_else(|| error("unit conversion overflow"))?;
                                }
                            }
                            if coefficient.unsigned_abs() >= 10u128.pow(38) {
                                return Err(error("unit conversion result precision overflow"));
                            }
                            Ok(Some(coefficient))
                        })
                        .collect::<datafusion::error::Result<Vec<_>>>()?;
                    let array = Arc::new(
                        Decimal128Array::from(coefficients).with_precision_and_scale(38, 18)?,
                    );
                    if args
                        .iter()
                        .all(|arg| matches!(arg, ColumnarValue::Scalar(_)))
                    {
                        Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                            array.as_ref(),
                            0,
                        )?))
                    } else {
                        Ok(ColumnarValue::Array(array))
                    }
                }),
            ))
        })
        .clone()
}
