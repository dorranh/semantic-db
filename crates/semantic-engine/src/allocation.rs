//! Checked scalar primitives for a same-query bridge allocation plan. The
//! compiler supplies partition counts, weight sums, duplicate checks and a
//! deterministic remainder rank from the policy-visible joined population.
use datafusion::{
    arrow::array::{Array, BooleanArray, Int64Array},
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{ColumnarValue, ScalarUDF, Volatility, create_udf},
};
use std::sync::{Arc, OnceLock};

fn failure(message: &str) -> datafusion::error::DataFusionError {
    semantic_runtime::failure(message)
}

fn args_i64(args: &[ColumnarValue], arity: usize) -> datafusion::error::Result<Vec<Vec<i64>>> {
    if args.len() != arity {
        return Err(failure("allocation function arity"));
    }
    let arrays = ColumnarValue::values_to_arrays(args)?;
    arrays
        .iter()
        .map(|array| {
            let values = array
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| failure("allocation argument type"))?;
            (0..values.len())
                .map(|index| {
                    if values.is_null(index) {
                        Err(failure("allocation argument is null"))
                    } else {
                        Ok(values.value(index))
                    }
                })
                .collect::<datafusion::error::Result<Vec<_>>>()
        })
        .collect()
}

fn evaluate_i64(
    args: &[ColumnarValue],
    arity: usize,
    kernel: impl Fn(&[i64]) -> datafusion::error::Result<i64>,
) -> datafusion::error::Result<ColumnarValue> {
    let arrays = args_i64(args, arity)?;
    let rows = arrays.first().map_or(0, Vec::len);
    let values = (0..rows)
        .map(|index| {
            let inputs = arrays.iter().map(|array| array[index]).collect::<Vec<_>>();
            kernel(&inputs)
        })
        .collect::<datafusion::error::Result<Vec<_>>>()?;
    if args
        .iter()
        .all(|arg| matches!(arg, ColumnarValue::Scalar(_)))
    {
        Ok(ColumnarValue::Scalar(ScalarValue::Int64(
            values.first().copied(),
        )))
    } else {
        Ok(ColumnarValue::Array(Arc::new(Int64Array::from(values))))
    }
}

fn evaluate_bool(
    args: &[ColumnarValue],
    arity: usize,
    kernel: impl Fn(&[i64]) -> datafusion::error::Result<bool>,
) -> datafusion::error::Result<ColumnarValue> {
    let arrays = args_i64(args, arity)?;
    let rows = arrays.first().map_or(0, Vec::len);
    let values = (0..rows)
        .map(|index| {
            let inputs = arrays.iter().map(|array| array[index]).collect::<Vec<_>>();
            kernel(&inputs)
        })
        .collect::<datafusion::error::Result<Vec<_>>>()?;
    if args
        .iter()
        .all(|arg| matches!(arg, ColumnarValue::Scalar(_)))
    {
        Ok(ColumnarValue::Scalar(ScalarValue::Boolean(
            values.first().copied(),
        )))
    } else {
        Ok(ColumnarValue::Array(Arc::new(BooleanArray::from(values))))
    }
}

fn quotient_remainder(
    amount: i64,
    weight: i64,
    denominator: i64,
) -> datafusion::error::Result<(i64, i64)> {
    if weight < 0 || denominator <= 0 || weight > denominator {
        return Err(failure("allocation weight or denominator is invalid"));
    }
    // |Int64::MIN| * Int64::MAX < 2^126, so an i128 intermediate is exact.
    let numerator = i128::from(amount).abs() * i128::from(weight);
    let denominator = i128::from(denominator);
    let magnitude = numerator / denominator;
    let signed = if amount < 0 { -magnitude } else { magnitude };
    let floor = i64::try_from(signed).map_err(|_| failure("allocation share overflow"))?;
    let remainder = i64::try_from(numerator % denominator)
        .map_err(|_| failure("allocation remainder overflow"))?;
    Ok((floor, remainder))
}

/// Signed truncation of amount * weight / denominator. The denominator is the
/// sum of eligible weights for exactly one source entity, checked separately.
pub fn semantic_allocation_floor_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_allocation_floor_v1",
                vec![DataType::Int64; 3],
                DataType::Int64,
                Volatility::Immutable,
                Arc::new(|args| {
                    evaluate_i64(args, 3, |v| {
                        quotient_remainder(v[0], v[1], v[2]).map(|p| p.0)
                    })
                }),
            ))
        })
        .clone()
}

/// Nonnegative remainder used for descending rank with the target tuple as
/// ascending tie-break. This makes output invariant to input row order.
pub fn semantic_allocation_remainder_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_allocation_remainder_v1",
                vec![DataType::Int64; 3],
                DataType::Int64,
                Volatility::Immutable,
                Arc::new(|args| {
                    evaluate_i64(args, 3, |v| {
                        quotient_remainder(v[0], v[1], v[2]).map(|p| p.1)
                    })
                }),
            ))
        })
        .clone()
}

/// Actual eligible count, expected count, actual eligible weight, expected
/// weight, and maximum multiplicity of one source/target tuple. Must be
/// evaluated after all endpoint policies in the same query execution.
pub fn semantic_assert_allocation_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_assert_allocation_v1",
                vec![DataType::Int64; 5],
                DataType::Boolean,
                Volatility::Volatile,
                Arc::new(|args| {
                    evaluate_bool(args, 5, |v| {
                        if v[0] <= 0
                            || v[1] <= 0
                            || v[0] != v[1]
                            || v[2] <= 0
                            || v[2] != v[3]
                            || v[4] != 1
                        {
                            Err(failure(
                                "allocation population or uniqueness obligation failed",
                            ))
                        } else {
                            Ok(true)
                        }
                    })
                }),
            ))
        })
        .clone()
}

/// Final signed minor-unit share. `rank` is one-based in descending remainder,
/// ascending target order; `residual` is amount minus the sum of signed floors.
/// `eligible_count` bounds both the rank and the number of residual minor units.
pub fn semantic_allocation_share_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_allocation_share_v1",
                vec![DataType::Int64; 6],
                DataType::Int64,
                Volatility::Immutable,
                Arc::new(|args| {
                    evaluate_i64(args, 6, |v| {
                        let (floor, _) = quotient_remainder(v[0], v[1], v[2])?;
                        let rank = v[3];
                        let residual = v[4];
                        let count = v[5];
                        let residual_magnitude = i64::try_from(residual.unsigned_abs())
                            .map_err(|_| failure("allocation residual overflow"))?;
                        if rank <= 0
                            || count <= 0
                            || rank > count
                            || residual_magnitude >= count
                            || (residual != 0 && residual.signum() != v[0].signum())
                        {
                            return Err(failure("allocation remainder rank or sign is invalid"));
                        }
                        let adjustment = if rank <= residual_magnitude {
                            v[0].signum()
                        } else {
                            0
                        };
                        floor
                            .checked_add(adjustment)
                            .ok_or_else(|| failure("allocation share overflow"))
                    })
                }),
            ))
        })
        .clone()
}
