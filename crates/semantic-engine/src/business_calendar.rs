//! UTC microsecond instant to authored IANA local date. The timezone comes
//! from a validated, pinned business calendar rule; session timezone is never
//! consulted. The function name/version pins chrono-tz 0.10.4 behavior.

use std::sync::{Arc, OnceLock};

use chrono::NaiveDate;
use datafusion::{
    arrow::{
        array::{Array, Date32Array, StringArray, TimestampMicrosecondArray},
        datatypes::{DataType, TimeUnit},
    },
    logical_expr::{ColumnarValue, ScalarUDF, Volatility, create_udf},
};

fn error(message: &str) -> datafusion::error::DataFusionError {
    semantic_runtime::failure(message)
}

fn local_date32(ticks: i64, zone: &str) -> datafusion::error::Result<i32> {
    let timezone: chrono_tz::Tz = zone.parse().map_err(|_| error("calendar IANA timezone"))?;
    let instant = chrono::DateTime::from_timestamp_micros(ticks)
        .ok_or_else(|| error("calendar UTC instant range"))?;
    let day = instant.with_timezone(&timezone).date_naive();
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("fixed epoch");
    day.signed_duration_since(epoch)
        .num_days()
        .try_into()
        .map_err(|_| error("calendar Date32 range"))
}

/// Input must be an Arrow UTC timestamp; the second argument must be an
/// authored IANA timezone. Null timestamps propagate; null/invalid zones fail.
pub fn semantic_local_date_us_v1() -> Arc<ScalarUDF> {
    static FUNCTION: OnceLock<Arc<ScalarUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(create_udf(
                "semantic_local_date_us_v1",
                vec![
                    DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                    DataType::Utf8,
                ],
                DataType::Date32,
                Volatility::Immutable,
                Arc::new(|args| {
                    if args.len() != 2 {
                        return Err(error("calendar local date arity"));
                    }
                    let arrays = ColumnarValue::values_to_arrays(args)?;
                    let instants = arrays[0]
                        .as_any()
                        .downcast_ref::<TimestampMicrosecondArray>()
                        .ok_or_else(|| error("calendar UTC timestamp type"))?;
                    let zones = arrays[1]
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .ok_or_else(|| error("calendar timezone type"))?;
                    let values = (0..instants.len())
                        .map(|index| {
                            if zones.is_null(index) {
                                return Err(error("calendar IANA timezone is null"));
                            }
                            if instants.is_null(index) {
                                return Ok(None);
                            }
                            local_date32(instants.value(index), zones.value(index)).map(Some)
                        })
                        .collect::<datafusion::error::Result<Vec<_>>>()?;
                    Ok(ColumnarValue::Array(Arc::new(Date32Array::from(values))))
                }),
            ))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use datafusion::arrow::array::ArrayRef;

    fn micros(instant: &str) -> i64 {
        DateTime::parse_from_rfc3339(instant)
            .unwrap()
            .timestamp_micros()
    }

    #[test]
    fn zurich_dst_boundaries_use_local_calendar_dates() {
        let before = local_date32(micros("2026-03-28T22:30:00Z"), "Europe/Zurich").unwrap();
        let spring = local_date32(micros("2026-03-28T23:30:00Z"), "Europe/Zurich").unwrap();
        let after = local_date32(micros("2026-03-29T22:30:00Z"), "Europe/Zurich").unwrap();
        assert_eq!(spring, before + 1);
        assert_eq!(after, spring + 1);
        // Both UTC instants name the repeated 02:30 local hour at fall-back.
        let first = local_date32(micros("2026-10-25T00:30:00Z"), "Europe/Zurich").unwrap();
        let second = local_date32(micros("2026-10-25T01:30:00Z"), "Europe/Zurich").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn invalid_zone_and_unrepresentable_instant_fail() {
        assert!(local_date32(0, "Mars/Base").is_err());
        assert!(local_date32(i64::MAX, "UTC").is_err());
    }

    #[test]
    fn udf_propagates_null_instants_but_rejects_null_zone() {
        let invoke = |zones: Vec<Option<&str>>| {
            semantic_local_date_us_v1().invoke_with_args(
                datafusion::logical_expr::ScalarFunctionArgs {
                    args: vec![
                        ColumnarValue::Array(Arc::new(
                            TimestampMicrosecondArray::from(vec![None, Some(0)])
                                .with_timezone("UTC"),
                        ) as ArrayRef),
                        ColumnarValue::Array(Arc::new(StringArray::from(zones)) as ArrayRef),
                    ],
                    arg_fields: vec![],
                    number_rows: 2,
                    return_field: Arc::new(datafusion::arrow::datatypes::Field::new(
                        "local_date",
                        DataType::Date32,
                        true,
                    )),
                    config_options: Arc::new(datafusion::config::ConfigOptions::default()),
                },
            )
        };
        let ColumnarValue::Array(output) = invoke(vec![Some("UTC"), Some("UTC")]).unwrap() else {
            panic!("expected array")
        };
        let dates = output.as_any().downcast_ref::<Date32Array>().unwrap();
        assert!(dates.is_null(0));
        assert_eq!(dates.value(1), 0);
        assert!(invoke(vec![Some("UTC"), None]).is_err());
    }
}
