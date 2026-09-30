use chrono::{Datelike, Months, NaiveDate, TimeZone, Utc};
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema, TimeUnit},
    common::ScalarValue,
};
use std::collections::BTreeSet;

use super::{CompileDiagnostic, diagnostic};

/// A hard ceiling is part of the executable profile, not a caller-controlled
/// memory budget. A narrower request limit may reduce it further.
pub(super) const MAX_UTC_MONTHS: usize = 120;

/// Inclusive start and exclusive end, both first-of-month UTC midnights.
/// The generated rows are independent of observed fact rows.
pub(super) fn utc_months(
    start_us: i64,
    end_us: i64,
    requested_limit: usize,
) -> Result<Vec<i64>, CompileDiagnostic> {
    let invalid = || {
        diagnostic(
            "calendar_spine_range",
            "Calendar fill needs a nonempty half-open range of UTC month boundaries",
        )
    };
    if start_us >= end_us {
        return Err(invalid());
    }
    let start = Utc
        .timestamp_micros(start_us)
        .single()
        .ok_or_else(invalid)?;
    let end = Utc.timestamp_micros(end_us).single().ok_or_else(invalid)?;
    if !is_month_start(start.naive_utc().date(), start_us)
        || !is_month_start(end.naive_utc().date(), end_us)
    {
        return Err(invalid());
    }
    let limit = requested_limit.min(MAX_UTC_MONTHS);
    let mut months = Vec::new();
    let mut date = start.naive_utc().date();
    while date < end.naive_utc().date() {
        if months.len() >= limit {
            return Err(diagnostic(
                "calendar_spine_limit",
                "Calendar fill exceeds the bounded UTC-month profile",
            ));
        }
        let month = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(invalid)?
            .and_utc()
            .timestamp_micros();
        months.push(month);
        date = date
            .checked_add_months(Months::new(1))
            .ok_or_else(invalid)?;
    }
    Ok(months)
}

fn is_month_start(date: NaiveDate, timestamp_us: i64) -> bool {
    date.day() == 1
        && date
            .and_hms_opt(0, 0, 0)
            .map(|instant| instant.and_utc().timestamp_micros())
            == Some(timestamp_us)
}

/// A compiler-owned Values relation validates every cell against its Arrow
/// schema before either SQL emission or direct planning can consume it.
#[derive(Debug, Clone)]
pub(super) struct TypedValues {
    pub schema: Schema,
    pub rows: Vec<Vec<ScalarValue>>,
}

impl TypedValues {
    pub fn new(
        schema: Schema,
        rows: Vec<Vec<ScalarValue>>,
        max_rows: usize,
    ) -> Result<Self, CompileDiagnostic> {
        let mut names = BTreeSet::new();
        if schema.fields().is_empty()
            || schema.fields().len() > 16
            || schema.fields().iter().any(|field| {
                field.name().is_empty() || !names.insert(field.name().as_str())
            })
            || rows.is_empty()
            || rows.len() > max_rows
            || rows.len() > MAX_UTC_MONTHS
            || schema.fields().iter().any(|field| {
                !matches!(field.data_type(), DataType::Int64)
                    && !matches!(field.data_type(), DataType::Timestamp(TimeUnit::Microsecond, Some(zone)) if zone.as_ref() == "UTC")
            })
        {
            return Err(diagnostic(
                "values_contract",
                "Typed Values needs bounded nonempty rows and Int64 or UTC-microsecond fields",
            ));
        }
        for row in &rows {
            if row.len() != schema.fields().len() {
                return Err(diagnostic(
                    "values_contract",
                    "Typed Values row width does not match its schema",
                ));
            }
            for (cell, field) in row.iter().zip(schema.fields()) {
                if cell.data_type() != *field.data_type()
                    || (!field.is_nullable() && cell.is_null())
                {
                    return Err(diagnostic(
                        "values_contract",
                        "Typed Values cell does not match its declared field",
                    ));
                }
            }
        }
        Ok(Self { schema, rows })
    }

    pub fn utc_month_spine(months: &[i64]) -> Result<Self, CompileDiagnostic> {
        Self::new(
            Schema::new(vec![Field::new(
                "__semantic_spine_month",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            )]),
            months
                .iter()
                .map(|month| {
                    vec![ScalarValue::TimestampMicrosecond(
                        Some(*month),
                        Some("UTC".into()),
                    )]
                })
                .collect(),
            MAX_UTC_MONTHS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_spine_is_bounded_and_includes_unobserved_february() {
        let start = 1_767_225_600_000_000; // 2026-01-01 UTC
        let end = 1_775_001_600_000_000; // 2026-04-01 UTC
        let months = utc_months(start, end, 3).unwrap();
        assert_eq!(
            months,
            [start, 1_769_904_000_000_000, 1_772_323_200_000_000]
        );
        let values = TypedValues::utc_month_spine(&months).unwrap();
        assert_eq!(values.rows.len(), 3);
        assert_eq!(values.schema.fields().len(), 1);
    }

    #[test]
    fn rejects_unaligned_reversed_and_unbounded_ranges() {
        let start = 1_767_225_600_000_000;
        let end = 1_775_001_600_000_000;
        assert_eq!(
            utc_months(start + 1, end, 3).unwrap_err().code,
            "calendar_spine_range"
        );
        assert_eq!(
            utc_months(end, start, 3).unwrap_err().code,
            "calendar_spine_range"
        );
        assert_eq!(
            utc_months(start, end, 2).unwrap_err().code,
            "calendar_spine_limit"
        );
    }

    #[test]
    fn typed_values_rejects_wrong_width_type_and_null() {
        let schema = Schema::new(vec![Field::new("n", DataType::Int64, false)]);
        for bad in [
            vec![],
            vec![ScalarValue::Utf8(Some("1".into()))],
            vec![ScalarValue::Int64(None)],
        ] {
            assert_eq!(
                TypedValues::new(schema.clone(), vec![bad], 1)
                    .unwrap_err()
                    .code,
                "values_contract"
            );
        }
    }
}
