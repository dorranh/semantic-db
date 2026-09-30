//! Pinned UTC month bucketing for microsecond timestamps. This does not infer
//! empty months or a business/fiscal calendar.

use std::sync::{Arc, OnceLock};

use chrono::{Datelike, NaiveDate};
use datafusion::{
    arrow::{
        array::{Array, TimestampMicrosecondArray},
        datatypes::{DataType, TimeUnit},
    },
    logical_expr::{ColumnarValue, ScalarUDF, Volatility, create_udf},
};

fn error(message: &str) -> datafusion::error::DataFusionError {
    semantic_runtime::failure(message)
}

/// UTC instant -> first instant of its UTC calendar month. Both physical input
/// and output are timestamp microseconds with an explicit UTC timezone. Nulls
/// propagate; representability failures reject execution.
pub fn semantic_utc_month_us_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_utc_month_us_v1",
                vec![DataType::Timestamp(
                    TimeUnit::Microsecond,
                    Some("UTC".into()),
                )],
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                Volatility::Immutable,
                Arc::new(|args| {
                    if args.len() != 1 {
                        return Err(error("UTC month bucketing requires one timestamp"));
                    }
                    let arrays = ColumnarValue::values_to_arrays(args)?;
                    let values = arrays[0]
                        .as_any()
                        .downcast_ref::<TimestampMicrosecondArray>()
                        .ok_or_else(|| error("UTC month timestamp type"))?;
                    let buckets = (0..values.len())
                        .map(|index| {
                            if values.is_null(index) {
                                return Ok(None);
                            }
                            let instant =
                                chrono::DateTime::from_timestamp_micros(values.value(index))
                                    .ok_or_else(|| error("UTC month timestamp range"))?;
                            let start = NaiveDate::from_ymd_opt(instant.year(), instant.month(), 1)
                                .and_then(|day| day.and_hms_micro_opt(0, 0, 0, 0))
                                .ok_or_else(|| error("UTC month boundary"))?;
                            Ok(Some(start.and_utc().timestamp_micros()))
                        })
                        .collect::<datafusion::error::Result<Vec<_>>>()?;
                    Ok(ColumnarValue::Array(Arc::new(
                        TimestampMicrosecondArray::from(buckets).with_timezone_opt(Some("UTC")),
                    )))
                }),
            ))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::ArrayRef;

    #[test]
    fn utc_month_boundaries_and_nulls() {
        let input = TimestampMicrosecondArray::from(vec![
            Some(1_769_903_999_999_999), // 2026-02-01 boundary minus one microsecond
            Some(1_769_904_000_000_000),
            Some(-1),
            None,
        ])
        .with_timezone_opt(Some("UTC"));
        let output = semantic_utc_month_us_v1()
            .invoke_with_args(datafusion::logical_expr::ScalarFunctionArgs {
                args: vec![ColumnarValue::Array(Arc::new(input) as ArrayRef)],
                arg_fields: vec![],
                number_rows: 4,
                return_field: Arc::new(datafusion::arrow::datatypes::Field::new(
                    "bucket",
                    DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                    true,
                )),
                config_options: Arc::new(datafusion::config::ConfigOptions::default()),
            })
            .unwrap();
        let ColumnarValue::Array(output) = output else {
            panic!("expected array")
        };
        let values = output
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap();
        assert_eq!(values.value(0), 1_767_225_600_000_000);
        assert_eq!(values.value(1), 1_769_904_000_000_000);
        assert_eq!(values.value(2), -2_678_400_000_000);
        assert!(values.is_null(3));
    }
}
