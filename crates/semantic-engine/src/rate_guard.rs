//! Required same-query coverage and uniqueness for a dated rate lookup.
use std::sync::{Arc, OnceLock};

use datafusion::{
    arrow::array::{Array, BooleanArray, Int64Array},
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{ColumnarValue, ScalarUDF, Volatility, create_udf},
};

pub fn semantic_assert_exactly_one_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_assert_exactly_one_v1",
                vec![DataType::Int64],
                DataType::Boolean,
                Volatility::Volatile,
                Arc::new(|args| {
                    let [count] = args else {
                        return Err(semantic_runtime::failure("rate match count arity"));
                    };
                    let arrays = ColumnarValue::values_to_arrays(args)?;
                    let counts = arrays[0]
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| semantic_runtime::failure("rate match count type"))?;
                    if (0..counts.len()).any(|i| counts.is_null(i) || counts.value(i) != 1) {
                        return Err(semantic_runtime::failure(
                            "rate coverage or uniqueness obligation failed",
                        ));
                    }
                    if matches!(count, ColumnarValue::Scalar(_)) {
                        Ok(ColumnarValue::Scalar(ScalarValue::Boolean(Some(true))))
                    } else {
                        Ok(ColumnarValue::Array(Arc::new(BooleanArray::from(vec![
                            true;
                            counts.len()
                        ]))))
                    }
                }),
            ))
        })
        .clone()
}
