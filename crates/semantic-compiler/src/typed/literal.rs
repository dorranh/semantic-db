use super::{CompileDiagnostic, diagnostic};
use datafusion::{common::ScalarValue, sql::sqlparser::ast};
use semantic_plan::typed::{Literal, TimestampUnit};

pub(super) fn checked_scalar(value: &Literal) -> Result<ScalarValue, CompileDiagnostic> {
    Ok(match value {
        Literal::Boolean(v) => ScalarValue::Boolean(Some(*v)),
        Literal::Int16(v) => ScalarValue::Int16(Some(*v)),
        Literal::Int32(v) => ScalarValue::Int32(Some(*v)),
        Literal::Int64(v) => ScalarValue::Int64(Some(*v)),
        Literal::Float64(v) => {
            if !v.is_finite() {
                return Err(diagnostic(
                    "float_literal",
                    "Float64 literals must be finite",
                ));
            }
            ScalarValue::Float64(Some(*v))
        }
        Literal::UInt64(v) => ScalarValue::UInt64(Some(*v)),
        Literal::Utf8(v) => ScalarValue::Utf8(Some(v.clone())),
        Literal::Date32(v) => ScalarValue::Date32(Some(*v)),
        Literal::Decimal128 {
            coefficient,
            precision,
            scale,
        } => {
            let digits = coefficient.strip_prefix('-').unwrap_or(coefficient);
            if *precision == 0
                || *precision > 38
                || scale > precision
                || digits.is_empty()
                || !digits.bytes().all(|b| b.is_ascii_digit())
                || digits.trim_start_matches('0').len() > *precision as usize
                || coefficient.len() > 40
            {
                return Err(diagnostic(
                    "decimal_literal",
                    "Decimal coefficient or precision/scale is invalid",
                ));
            }
            let coefficient = coefficient.parse::<i128>().map_err(|_| {
                diagnostic("decimal_literal", "Decimal coefficient is out of range")
            })?;
            ScalarValue::Decimal128(Some(coefficient), *precision, *scale as i8)
        }
        Literal::Timestamp {
            ticks,
            unit,
            timezone,
        } => {
            // Named-zone civil-time interpretation needs a separate calendar rule.
            // This executable profile accepts UTC instants and explicitly naive counts.
            if timezone
                .as_deref()
                .is_some_and(|zone| !matches!(zone, "UTC" | "+00:00"))
            {
                return Err(diagnostic(
                    "timestamp_timezone",
                    "Absolute timestamp literals support UTC or explicitly timezone-free counts",
                ));
            }
            let tz = timezone.as_deref().map(Into::into);
            match unit {
                TimestampUnit::Second => ScalarValue::TimestampSecond(Some(*ticks), tz),
                TimestampUnit::Millisecond => ScalarValue::TimestampMillisecond(Some(*ticks), tz),
                TimestampUnit::Microsecond => ScalarValue::TimestampMicrosecond(Some(*ticks), tz),
                TimestampUnit::Nanosecond => ScalarValue::TimestampNanosecond(Some(*ticks), tz),
            }
        }
    })
}
pub(super) fn sql_type(value: &Literal) -> ast::DataType {
    match value {
        Literal::Boolean(_) => ast::DataType::Boolean,
        Literal::Int16(_) => ast::DataType::SmallInt(None),
        Literal::Int32(_) => ast::DataType::Int(None),
        Literal::Int64(_) => ast::DataType::BigInt(None),
        Literal::Float64(_) => ast::DataType::Double(ast::ExactNumberInfo::None),
        Literal::UInt64(_) => ast::DataType::BigIntUnsigned(None),
        Literal::Utf8(_) => ast::DataType::Text,
        Literal::Date32(_) => ast::DataType::Date,
        Literal::Decimal128 {
            precision, scale, ..
        } => ast::DataType::Decimal(ast::ExactNumberInfo::PrecisionAndScale(
            *precision as u64,
            *scale as i64,
        )),
        Literal::Timestamp { unit, timezone, .. } => ast::DataType::Timestamp(
            Some(match unit {
                TimestampUnit::Second => 0,
                TimestampUnit::Millisecond => 3,
                TimestampUnit::Microsecond => 6,
                TimestampUnit::Nanosecond => 9,
            }),
            if timezone.is_some() {
                ast::TimezoneInfo::WithTimeZone
            } else {
                ast::TimezoneInfo::None
            },
        ),
    }
}
