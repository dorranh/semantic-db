//! Versioned compiler functions with identical direct-plan and SQL semantics.
use datafusion::{
    arrow::{
        array::{Array, BooleanArray, Decimal128Array, Int64Array},
        datatypes::DataType,
    },
    common::ScalarValue,
    logical_expr::{ColumnarValue, ScalarUDF, Volatility, create_udf},
};
use std::sync::{Arc, OnceLock};

/// Exact integer ratio quantized to 18 decimal places, truncating toward zero.
/// Null arguments propagate. The third argument selects zero (true) or null
/// (false) for a zero denominator. No binary floating-point intermediate exists.
/// Every possible i64 numerator times 10^18 fits in i128.
pub fn semantic_ratio_i64_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_ratio_i64_v1",
                vec![DataType::Int64, DataType::Int64, DataType::Boolean],
                DataType::Decimal128(38, 18),
                Volatility::Immutable,
                Arc::new(|args| {
                    if args.len() != 3 {
                        return Err(semantic_runtime::failure(
                            "ratio requires exactly three arguments",
                        ));
                    }
                    let arrays = ColumnarValue::values_to_arrays(args)?;
                    let numerator = arrays[0]
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| semantic_runtime::failure("ratio numerator type"))?;
                    let denominator = arrays[1]
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| semantic_runtime::failure("ratio denominator type"))?;
                    let zero = arrays[2]
                        .as_any()
                        .downcast_ref::<BooleanArray>()
                        .ok_or_else(|| semantic_runtime::failure("ratio zero behavior type"))?;
                    let values = (0..numerator.len())
                        .map(|i| {
                            if numerator.is_null(i) || denominator.is_null(i) || zero.is_null(i) {
                                None
                            } else if denominator.value(i) == 0 {
                                zero.value(i).then_some(0)
                            } else {
                                Some(
                                    i128::from(numerator.value(i)) * 1_000_000_000_000_000_000
                                        / i128::from(denominator.value(i)),
                                )
                            }
                        })
                        .collect::<Vec<_>>();
                    let array =
                        Arc::new(Decimal128Array::from(values).with_precision_and_scale(38, 18)?);
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

/// A same-execution uniqueness obligation over grouped right-hand keys. Returning
/// true permits the lookup; zero/one are safe, null/negative/multiple counts error.
/// This must stay local, including when the counted input is federated.
pub fn semantic_assert_single_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_assert_single_v1",
                vec![DataType::Int64],
                DataType::Boolean,
                Volatility::Volatile,
                Arc::new(|args| {
                    if args.len() != 1 {
                        return Err(semantic_runtime::failure("lookup count arity"));
                    }
                    let arrays = ColumnarValue::values_to_arrays(args)?;
                    let counts = arrays[0]
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| semantic_runtime::failure("lookup count type"))?;
                    if (0..counts.len())
                        .any(|i| counts.is_null(i) || !(0..=1).contains(&counts.value(i)))
                    {
                        return Err(semantic_runtime::failure(
                            "semantic lookup uniqueness obligation failed",
                        ));
                    }
                    if matches!(args[0], ColumnarValue::Scalar(_)) {
                        Ok(ColumnarValue::Scalar(ScalarValue::Boolean(Some(true))))
                    } else {
                        Ok(ColumnarValue::Array(Arc::new(BooleanArray::from(
                            vec![true; counts.len()],
                        ))))
                    }
                }),
            ))
        })
        .clone()
}
