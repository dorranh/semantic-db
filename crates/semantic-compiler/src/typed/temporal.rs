//! Civil calendar interpretation is a deterministic binding pass. The model
//! chooses a period expression but cannot choose or overwrite host context.
use super::{CompileDiagnostic, diagnostic};
use chrono::{Datelike, Days, Months, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use semantic_catalog::DataType;
use semantic_plan::typed::{CalendarPeriod, CalendarUnit, Literal, TimestampUnit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Calendar {
    Gregorian,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextOrigin {
    Caller,
    ConfiguredDefault { revision: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestContext {
    /// Captured once by the host; never re-read from a clock during compilation.
    pub reference_unix_millis: i64,
    pub timezone: String,
    pub calendar: Calendar,
    pub origin: ContextOrigin,
}
impl RequestContext {
    pub fn validate(&self) -> Result<(), CompileDiagnostic> {
        if self.timezone.len() > 128
            || self.timezone.parse::<Tz>().is_err()
            || chrono::DateTime::<Utc>::from_timestamp_millis(self.reference_unix_millis).is_none()
            || matches!(&self.origin, ContextOrigin::ConfiguredDefault { revision } if revision.is_empty() || revision.len() > 256)
        {
            return Err(diagnostic(
                "invalid_time_context",
                "Request context requires a valid instant, IANA timezone and bounded default revision",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct TemporalResolution {
    pub context: RequestContext,
    pub period: CalendarPeriod,
    pub local_start: String,
    pub local_end: String,
    pub start_unix_millis: i64,
    pub end_unix_millis: i64,
    pub rule: &'static str,
}
pub(super) fn resolve(
    period: &CalendarPeriod,
    context: &RequestContext,
    ty: &DataType,
) -> Result<(Literal, Literal, TemporalResolution), CompileDiagnostic> {
    context.validate()?;
    if period.count == 0 || period.count > 100_000 || period.offset.unsigned_abs() > 100_000 {
        return Err(diagnostic(
            "temporal_range",
            "Calendar range count and offset exceed the supported bounds",
        ));
    }
    let timezone: Tz = context.timezone.parse().expect("validated timezone");
    let instant = chrono::DateTime::<Utc>::from_timestamp_millis(context.reference_unix_millis)
        .expect("validated instant");
    let date = instant.with_timezone(&timezone).date_naive();
    let anchor = match period.unit {
        CalendarUnit::Day => date,
        CalendarUnit::IsoWeek => date
            .checked_sub_days(Days::new(u64::from(date.weekday().num_days_from_monday())))
            .ok_or_else(range_error)?,
        CalendarUnit::Month => {
            NaiveDate::from_ymd_opt(date.year(), date.month(), 1).ok_or_else(range_error)?
        }
        CalendarUnit::Year => NaiveDate::from_ymd_opt(date.year(), 1, 1).ok_or_else(range_error)?,
    };
    let start = shift(anchor, period.unit, i64::from(period.offset))?;
    let end = shift(start, period.unit, i64::from(period.count))?;
    let civil = |date: NaiveDate| {
        timezone.from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight"))
        .single().ok_or_else(|| diagnostic("ambiguous_calendar_boundary", "Calendar boundary is ambiguous or nonexistent in this timezone; provide explicit absolute bounds"))
    };
    let start_instant = civil(start)?;
    let end_instant = civil(end)?;
    let literal = |date: NaiveDate,
                   instant: chrono::DateTime<Tz>|
     -> Result<Literal, CompileDiagnostic> {
        match ty {
            DataType::Date32 => Ok(Literal::Date32(
                i32::try_from(
                    date.signed_duration_since(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
                        .num_days(),
                )
                .map_err(|_| range_error())?,
            )),
            DataType::Timestamp(unit, Some(zone)) if matches!(zone.as_ref(), "UTC" | "+00:00") => {
                use datafusion::arrow::datatypes::TimeUnit;
                let (ticks, unit) = match unit {
                    TimeUnit::Second => (instant.timestamp(), TimestampUnit::Second),
                    TimeUnit::Millisecond => {
                        (instant.timestamp_millis(), TimestampUnit::Millisecond)
                    }
                    TimeUnit::Microsecond => {
                        (instant.timestamp_micros(), TimestampUnit::Microsecond)
                    }
                    TimeUnit::Nanosecond => (
                        instant.timestamp_nanos_opt().ok_or_else(range_error)?,
                        TimestampUnit::Nanosecond,
                    ),
                };
                Ok(Literal::Timestamp {
                    ticks,
                    unit,
                    timezone: Some(zone.to_string()),
                })
            }
            _ => Err(diagnostic(
                "temporal_field_type",
                "Calendar filters require Date32 or UTC absolute timestamp fields",
            )),
        }
    };
    Ok((
        literal(start, start_instant)?,
        literal(end, end_instant)?,
        TemporalResolution {
            context: context.clone(),
            period: period.clone(),
            local_start: start.to_string(),
            local_end: end.to_string(),
            start_unix_millis: start_instant.timestamp_millis(),
            end_unix_millis: end_instant.timestamp_millis(),
            rule: "gregorian-local-period-half-open/v1/tzdb-chrono-tz-0.10.4",
        },
    ))
}
fn range_error() -> CompileDiagnostic {
    diagnostic(
        "temporal_range",
        "Calendar calculation exceeds the supported date range",
    )
}
fn shift(date: NaiveDate, unit: CalendarUnit, amount: i64) -> Result<NaiveDate, CompileDiagnostic> {
    let magnitude = amount.unsigned_abs();
    match unit {
        CalendarUnit::Day | CalendarUnit::IsoWeek => {
            let days = Days::new(
                magnitude
                    .checked_mul(if unit == CalendarUnit::IsoWeek { 7 } else { 1 })
                    .ok_or_else(range_error)?,
            );
            if amount < 0 {
                date.checked_sub_days(days)
            } else {
                date.checked_add_days(days)
            }
        }
        CalendarUnit::Month | CalendarUnit::Year => {
            let months = Months::new(
                u32::try_from(
                    magnitude
                        .checked_mul(if unit == CalendarUnit::Year { 12 } else { 1 })
                        .ok_or_else(range_error)?,
                )
                .map_err(|_| range_error())?,
            );
            if amount < 0 {
                date.checked_sub_months(months)
            } else {
                date.checked_add_months(months)
            }
        }
    }
    .ok_or_else(range_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(instant: &str, timezone: &str) -> RequestContext {
        RequestContext {
            reference_unix_millis: chrono::DateTime::parse_from_rfc3339(instant)
                .unwrap()
                .timestamp_millis(),
            timezone: timezone.into(),
            calendar: Calendar::Gregorian,
            origin: ContextOrigin::Caller,
        }
    }
    #[test]
    fn calendar_days_follow_dst_and_months_follow_leap_years() {
        let ty = DataType::Timestamp(
            datafusion::arrow::datatypes::TimeUnit::Millisecond,
            Some("UTC".into()),
        );
        for (instant, hours) in [("2024-03-31T12:00:00Z", 23), ("2024-10-27T12:00:00Z", 25)] {
            let (_, _, result) = resolve(
                &CalendarPeriod {
                    unit: CalendarUnit::Day,
                    offset: 0,
                    count: 1,
                },
                &context(instant, "Europe/Zurich"),
                &ty,
            )
            .unwrap();
            assert_eq!(
                result.end_unix_millis - result.start_unix_millis,
                hours * 60 * 60 * 1000
            );
        }
        let (_, _, result) = resolve(
            &CalendarPeriod {
                unit: CalendarUnit::Month,
                offset: -1,
                count: 1,
            },
            &context("2024-03-15T00:00:00Z", "UTC"),
            &DataType::Date32,
        )
        .unwrap();
        assert_eq!(
            (result.local_start.as_str(), result.local_end.as_str()),
            ("2024-02-01", "2024-03-01")
        );
        assert_eq!(
            (result.end_unix_millis - result.start_unix_millis) / 86_400_000,
            29
        );
        let (_, _, result) = resolve(
            &CalendarPeriod {
                unit: CalendarUnit::IsoWeek,
                offset: 0,
                count: 1,
            },
            &context("2024-03-31T23:30:00Z", "Europe/Zurich"),
            &DataType::Date32,
        )
        .unwrap();
        assert_eq!(
            (result.local_start.as_str(), result.local_end.as_str()),
            ("2024-04-01", "2024-04-08")
        );
    }
    #[test]
    fn ambiguous_or_missing_civil_boundaries_and_naive_timestamps_fail_closed() {
        let period = CalendarPeriod {
            unit: CalendarUnit::Day,
            offset: 0,
            count: 1,
        };
        let result = resolve(
            &period,
            &context("2018-11-04T12:00:00Z", "America/Sao_Paulo"),
            &DataType::Date32,
        );
        assert_eq!(result.unwrap_err().code, "ambiguous_calendar_boundary");
        let ty = DataType::Timestamp(datafusion::arrow::datatypes::TimeUnit::Second, None);
        assert_eq!(
            resolve(&period, &context("2024-01-01T12:00:00Z", "UTC"), &ty)
                .unwrap_err()
                .code,
            "temporal_field_type"
        );
    }
}
