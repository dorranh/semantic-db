use crate::*;
use bytes::BytesMut;
use datafusion::common::ScalarValue;
use tokio_postgres::types::{FromSql, IsNull, ToSql};

/// Borrowed binary value, used to inspect lengths before allocating Arrow buffers.
pub(crate) struct Raw<'a>(pub &'a [u8]);
impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(
        _: &Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(raw))
    }
    fn accepts(_: &Type) -> bool {
        true
    }
}
pub(crate) fn admitted_size(rows: &[Row]) -> Result<usize> {
    let mut size = 1024usize;
    for row in rows {
        for i in 0..row.len() {
            let raw = row
                .try_get::<_, Option<Raw<'_>>>(i)
                .map_err(|_| failure("Postgres binary value unavailable"))?;
            // Includes array headers, validity, offsets, conversion scratch and alignment.
            size = size
                .checked_add(raw.map_or(512, |v| {
                    let multiplier = if matches!(
                        row.columns()[i].type_().kind(),
                        tokio_postgres::types::Kind::Array(_)
                    ) {
                        32
                    } else {
                        4
                    };
                    v.0.len().saturating_mul(multiplier).saturating_add(512)
                }))
                .ok_or_else(|| failure("Postgres batch size overflow"))?;
        }
    }
    Ok(size)
}
pub(crate) fn string_array(values: Vec<Option<String>>) -> StringArray {
    // Arrow's FromIterator reserves 1024 data bytes even for one short value.
    // Reserve the actual payload so small batches stay within decode admission.
    let bytes = values.iter().flatten().map(String::len).sum();
    let mut builder = StringBuilder::with_capacity(values.len(), bytes);
    builder.extend(values);
    builder.finish()
}
pub(crate) fn decimal_type(modifier: i32) -> Result<DataType> {
    if modifier < 0 {
        return Ok(DataType::Decimal128(38, 10));
    }
    let m = modifier - 4;
    let precision = (m >> 16) & 0xffff;
    let scale = (m & 0x7ff) as i16;
    let scale = if scale >= 1024 { scale - 2048 } else { scale };
    if precision == 0 || precision > 38 || scale < 0 || scale as i32 > precision {
        return Err(failure("unsupported Postgres NUMERIC precision/scale"));
    }
    Ok(DataType::Decimal128(precision as u8, scale as i8))
}
/// PostgreSQL base-10000 numeric -> exact scaled integer. Never rounds or wraps.
pub(crate) fn numeric(raw: &[u8], precision: u8, scale: i8) -> Result<i128> {
    let bad = || failure("Postgres NUMERIC cannot be represented exactly");
    if raw.len() < 8 || scale < 0 {
        return Err(bad());
    }
    let read = |i| i16::from_be_bytes([raw[i], raw[i + 1]]);
    let count = read(0);
    let weight = read(2) as i32;
    let sign = read(4) as u16;
    if count < 0 || raw.len() != 8 + count as usize * 2 || ![0, 0x4000].contains(&sign) {
        return Err(bad());
    }
    let mut value = 0i128;
    for index in 0..count as usize {
        let digit = read(8 + index * 2) as i128;
        if !(0..10000).contains(&digit) {
            return Err(bad());
        }
        if digit == 0 {
            continue;
        }
        let exponent = 4 * (weight - index as i32) + scale as i32;
        let term = if exponent >= 0 {
            digit
                .checked_mul(10i128.checked_pow(exponent as u32).ok_or_else(bad)?)
                .ok_or_else(bad)?
        } else {
            let divisor = 10i128.checked_pow((-exponent) as u32).ok_or_else(bad)?;
            if digit % divisor != 0 {
                return Err(bad());
            }
            digit / divisor
        };
        value = value.checked_add(term).ok_or_else(bad)?;
    }
    if value >= 10i128.checked_pow(precision as u32).ok_or_else(bad)? {
        return Err(bad());
    }
    Ok(if sign == 0x4000 { -value } else { value })
}
pub(crate) fn numeric_at(row: &Row, i: usize, p: u8, s: i8) -> Result<Option<i128>> {
    row.try_get::<_, Option<Raw<'_>>>(i)
        .map_err(|_| failure("Postgres numeric decoding failed"))?
        .map(|v| numeric(v.0, p, s))
        .transpose()
}
#[derive(Debug)]
pub(crate) struct BinaryValue(pub Option<Vec<u8>>, pub Type);
impl ToSql for BinaryValue {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> std::result::Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        if ty != &self.1 {
            return Err("Postgres parameter type mismatch".into());
        }
        match &self.0 {
            Some(v) => {
                out.extend_from_slice(v);
                Ok(IsNull::No)
            }
            None => Ok(IsNull::Yes),
        }
    }
    fn accepts(_: &Type) -> bool {
        true
    }
    tokio_postgres::types::to_sql_checked!();
}
pub(crate) fn encode_numeric(value: i128, scale: i8) -> Vec<u8> {
    let negative = value < 0;
    let mut digits = value.unsigned_abs().to_string();
    let scale = scale as usize;
    if digits.len() <= scale {
        digits = format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits);
    }
    let whole = digits.len() - scale;
    let pad = (4 - whole % 4) % 4;
    digits = format!(
        "{}{}{}",
        "0".repeat(pad),
        digits,
        "0".repeat((4 - scale % 4) % 4)
    );
    let groups = digits
        .as_bytes()
        .chunks(4)
        .map(|v| std::str::from_utf8(v).unwrap().parse::<i16>().unwrap())
        .collect::<Vec<_>>();
    let mut out = vec![];
    for n in [
        groups.len() as i16,
        ((whole + pad) / 4) as i16 - 1,
        if negative { 0x4000 } else { 0 },
        scale as i16,
    ] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    for n in groups {
        out.extend_from_slice(&n.to_be_bytes());
    }
    out
}
pub(crate) fn extra_array(field: &Field, rows: &[Row], i: usize) -> Result<ArrayRef> {
    let bad = || failure("Postgres value conversion failed");
    Ok(match field.data_type() {
        DataType::Decimal128(p, s) => Arc::new(
            Decimal128Array::from(
                rows.iter()
                    .map(|r| numeric_at(r, i, *p, *s))
                    .collect::<Result<Vec<_>>>()?,
            )
            .with_precision_and_scale(*p, *s)?,
        ),
        DataType::Binary => {
            let values = rows
                .iter()
                .map(|r| r.try_get::<_, Option<&[u8]>>(i))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| bad())?;
            let bytes = values.iter().flatten().map(|value| value.len()).sum();
            let mut builder = BinaryBuilder::with_capacity(values.len(), bytes);
            builder.extend(values);
            Arc::new(builder.finish())
        }
        DataType::FixedSizeBinary(16) => {
            let mut builder = FixedSizeBinaryBuilder::with_capacity(rows.len(), 16);
            for r in rows {
                match r.try_get::<_, Option<Raw<'_>>>(i).map_err(|_| bad())? {
                    Some(v) => builder.append_value(v.0)?,
                    None => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DataType::Time64(TimeUnit::Microsecond) => {
            use chrono::Timelike;
            Arc::new(Time64MicrosecondArray::from(
                rows.iter()
                    .map(|r| {
                        r.try_get::<_, Option<chrono::NaiveTime>>(i)
                            .map(|v| {
                                v.map(|t| {
                                    t.num_seconds_from_midnight() as i64 * 1_000_000
                                        + t.nanosecond() as i64 / 1000
                                })
                            })
                            .map_err(|_| bad())
                    })
                    .collect::<Result<Vec<_>>>()?,
            ))
        }
        DataType::Interval(IntervalUnit::MonthDayNano) => {
            let values = rows
                .iter()
                .map(|r| {
                    r.try_get::<_, Option<Raw<'_>>>(i)
                        .map_err(|_| bad())?
                        .map(|v| {
                            if v.0.len() != 16 {
                                return Err(bad());
                            }
                            let micros = i64::from_be_bytes(v.0[0..8].try_into().unwrap());
                            let days = i32::from_be_bytes(v.0[8..12].try_into().unwrap());
                            let months = i32::from_be_bytes(v.0[12..16].try_into().unwrap());
                            Ok(IntervalMonthDayNanoType::make_value(
                                months,
                                days,
                                micros.checked_mul(1000).ok_or_else(bad)?,
                            ))
                        })
                        .transpose()
                })
                .collect::<Result<Vec<_>>>()?;
            Arc::new(IntervalMonthDayNanoArray::from(values))
        }
        DataType::List(item) => {
            for row in rows {
                if let Some(raw) = row.try_get::<_, Option<Raw<'_>>>(i).map_err(|_| bad())? {
                    if raw.0.len() < 12 {
                        return Err(bad());
                    }
                    let dimensions = i32::from_be_bytes(raw.0[..4].try_into().unwrap());
                    if dimensions != 0
                        && (dimensions != 1
                            || raw.0.len() < 20
                            || i32::from_be_bytes(raw.0[16..20].try_into().unwrap()) != 1)
                    {
                        return Err(failure(
                            "Postgres arrays require one dimension and a lower bound of one",
                        ));
                    }
                }
            }
            let values = rows
                .iter()
                .map(|r| {
                    macro_rules! list {
                        ($t:ty,$variant:ident) => {
                            r.try_get::<_, Option<Vec<Option<$t>>>>(i)
                                .map_err(|_| bad())?
                                .map(|v| {
                                    v.into_iter().map(ScalarValue::$variant).collect::<Vec<_>>()
                                })
                        };
                    }
                    let elements = match item.data_type() {
                        DataType::Int16 => list!(i16, Int16),
                        DataType::Int32 => list!(i32, Int32),
                        DataType::Int64 => list!(i64, Int64),
                        DataType::Boolean => list!(bool, Boolean),
                        DataType::Utf8 => list!(String, Utf8),
                        _ => return Err(bad()),
                    };
                    Ok(match elements {
                        Some(v) => {
                            ScalarValue::List(ScalarValue::new_list(&v, item.data_type(), true))
                        }
                        None => ScalarValue::new_null_list(item.data_type().clone(), true, 1),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            ScalarValue::iter_to_array(values)?
        }
        _ => return Err(bad()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_roundtrip_and_boundaries() {
        for value in [
            0,
            1,
            -1,
            i64::MAX as i128,
            i64::MIN as i128,
            99999999999999999999999999999999999999i128,
        ] {
            for scale in [0, 4, 10] {
                assert_eq!(
                    numeric(&encode_numeric(value, scale), 38, scale).unwrap(),
                    value
                );
            }
        }
        assert!(numeric(&encode_numeric(123, 2), 38, 1).is_err());
        assert!(numeric(&encode_numeric(1000, 0), 3, 0).is_err());
        assert!(numeric(&[0, 0, 0, 0, 0xc0, 0, 0, 0], 38, 0).is_err());
        assert_eq!(
            decimal_type(4 + (12 << 16) + 3).unwrap(),
            DataType::Decimal128(12, 3)
        );
    }
}
