use crate::{Column, Comparison, Expected, Result, TypedResult};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use datafusion::arrow::{
    array::*,
    datatypes::{DataType, Schema, TimeUnit},
    record_batch::RecordBatch,
};
use serde_json::Value;
#[derive(Debug, PartialEq)]
enum Kind {
    Int,
    Float,
    Bool,
    Text,
    Decimal(u8, i8),
    Date,
    Time(TimeUnit),
    Timestamp(TimeUnit, Option<String>),
}
fn unit(s: &str) -> Result<TimeUnit> {
    Ok(match s {
        "s" => TimeUnit::Second,
        "ms" => TimeUnit::Millisecond,
        "us" => TimeUnit::Microsecond,
        "ns" => TimeUnit::Nanosecond,
        _ => return Err("invalid temporal unit".into()),
    })
}
fn kind(s: &str) -> Result<Kind> {
    Ok(match s {
        "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64" => Kind::Int,
        "float32" | "float64" => Kind::Float,
        "boolean" => Kind::Bool,
        "utf8" => Kind::Text,
        "date32" => Kind::Date,
        _ if s.starts_with("decimal128(") && s.ends_with(')') => {
            let v: Vec<_> = s[11..s.len() - 1].split(',').collect();
            if v.len() != 2 {
                return Err("invalid decimal type".into());
            }
            let p: u8 = v[0].parse()?;
            let scale: i8 = v[1].parse()?;
            if p == 0 || p > 38 || scale < 0 || scale as u8 > p {
                return Err("invalid decimal precision/scale".into());
            }
            Kind::Decimal(p, scale)
        }
        _ if s.starts_with("time64(") && s.ends_with(')') => {
            let u = unit(&s[7..s.len() - 1])?;
            if !matches!(u, TimeUnit::Microsecond | TimeUnit::Nanosecond) {
                return Err("invalid time64 unit".into());
            }
            Kind::Time(u)
        }
        _ if s.starts_with("timestamp(") && s.ends_with(')') => {
            let v: Vec<_> = s[10..s.len() - 1].split(',').collect();
            if v.len() > 2 {
                return Err("invalid timestamp".into());
            }
            Kind::Timestamp(unit(v[0])?, v.get(1).map(|s| s.to_string()))
        }
        _ => return Err(format!("unsupported expected type {s}").into()),
    })
}
fn decimal(s: &str, p: u8, scale: i8) -> Result<i128> {
    let neg = s.starts_with('-');
    let s = s.strip_prefix('-').unwrap_or(s);
    let parts: Vec<_> = s.split('.').collect();
    if parts.len() > 2 || parts[0].is_empty() || !parts[0].bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid decimal literal".into());
    }
    let frac = parts.get(1).copied().unwrap_or("");
    if !frac.bytes().all(|b| b.is_ascii_digit()) || frac.len() > scale as usize {
        return Err("decimal exceeds declared scale".into());
    }
    let digits = format!(
        "{}{}{}",
        parts[0],
        frac,
        "0".repeat(scale as usize - frac.len())
    );
    let v: i128 = digits.parse()?;
    if v.to_string().len() > p as usize {
        return Err("decimal exceeds declared precision".into());
    }
    Ok(if neg { -v } else { v })
}
fn string(v: &Value) -> Result<&str> {
    v.as_str()
        .ok_or_else(|| "expected lossless JSON string".into())
}
fn normalize(v: &Value, k: &Kind) -> Result<Value> {
    if v.is_null() {
        return Ok(Value::Null);
    }
    Ok(match k {
        Kind::Int => Value::String(string(v)?.parse::<i128>()?.to_string()),
        Kind::Decimal(p, s) => Value::String(decimal(string(v)?, *p, *s)?.to_string()),
        Kind::Float => {
            let n = v.as_f64().ok_or("float must be finite JSON number")?;
            if !n.is_finite() {
                return Err("nonfinite float".into());
            }
            v.clone()
        }
        Kind::Bool => Value::Bool(v.as_bool().ok_or("boolean must be JSON boolean")?),
        Kind::Text => Value::String(string(v)?.into()),
        Kind::Date => Value::String(NaiveDate::parse_from_str(string(v)?, "%Y-%m-%d")?.to_string()),
        Kind::Time(_) => Value::String(
            NaiveTime::parse_from_str(string(v)?, "%H:%M:%S%.f")?
                .format("%H:%M:%S%.f")
                .to_string(),
        ),
        Kind::Timestamp(_, tz) => {
            if tz.is_some() {
                Value::String(
                    chrono::DateTime::parse_from_rfc3339(string(v)?)?
                        .with_timezone(&chrono::Utc)
                        .to_rfc3339(),
                )
            } else {
                Value::String(
                    NaiveDateTime::parse_from_str(string(v)?, "%Y-%m-%dT%H:%M:%S%.f")?
                        .format("%Y-%m-%dT%H:%M:%S%.f")
                        .to_string(),
                )
            }
        }
    })
}
pub(crate) fn validate_result(columns: &[Column], rows: &[Vec<Value>]) -> Result<()> {
    for c in columns {
        let k = kind(&c.kind)?;
        if let Kind::Timestamp(_, Some(tz)) = &k
            && tz != "UTC"
        {
            return Err("expected timestamp timezone must be UTC".into());
        }
        if let Some(t) = &c.tolerance
            && (k != Kind::Float
                || !t.absolute.is_finite()
                || !t.relative.is_finite()
                || t.absolute < 0.
                || t.relative < 0.)
        {
            return Err(
                "float tolerance requires finite nonnegative bounds on float column".into(),
            );
        }
    }
    for row in rows {
        if row.len() != columns.len() {
            return Err("expected row width differs from columns".into());
        }
        for (v, c) in row.iter().zip(columns) {
            if v.is_null() && !c.nullable {
                return Err("null in nonnullable expected column".into());
            }
            normalize(v, &kind(&c.kind)?)?;
            if !v.is_null() {
                let k = kind(&c.kind)?;
                let nanos = match &k {
                    Kind::Time(_) => {
                        Some(NaiveTime::parse_from_str(string(v)?, "%H:%M:%S%.f")?.nanosecond())
                    }
                    Kind::Timestamp(_, tz) => Some(if tz.is_some() {
                        chrono::DateTime::parse_from_rfc3339(string(v)?)?.timestamp_subsec_nanos()
                    } else {
                        NaiveDateTime::parse_from_str(string(v)?, "%Y-%m-%dT%H:%M:%S%.f")?
                            .nanosecond()
                    }),
                    _ => None,
                };
                if let Some(n) = nanos {
                    let u = match &k {
                        Kind::Time(u) | Kind::Timestamp(u, _) => u,
                        _ => unreachable!(),
                    };
                    let quantum = match u {
                        TimeUnit::Second => 1_000_000_000,
                        TimeUnit::Millisecond => 1_000_000,
                        TimeUnit::Microsecond => 1_000,
                        TimeUnit::Nanosecond => 1,
                    };
                    if n % quantum != 0 {
                        return Err("temporal value exceeds declared precision".into());
                    }
                }
                match c.kind.as_str() {
                    "uint8" => {
                        string(v)?.parse::<u8>()?;
                    }
                    "uint16" => {
                        string(v)?.parse::<u16>()?;
                    }
                    "uint32" => {
                        string(v)?.parse::<u32>()?;
                    }
                    "uint64" => {
                        string(v)?.parse::<u64>()?;
                    }
                    "int64" => {
                        string(v)?.parse::<i64>()?;
                    }
                    "int16" => {
                        string(v)?.parse::<i16>()?;
                    }
                    "int32" => {
                        string(v)?.parse::<i32>()?;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
fn compatible(a: &str, b: &str, physical: bool) -> Result<bool> {
    if physical {
        return Ok(a == b);
    }
    Ok(match (kind(a)?, kind(b)?) {
        (Kind::Int, Kind::Int)
        | (Kind::Float, Kind::Float)
        | (Kind::Decimal(..), Kind::Decimal(..))
        | (Kind::Time(_), Kind::Time(_)) => true,
        (Kind::Timestamp(_, a), Kind::Timestamp(_, b)) => a == b,
        (a, b) => a == b,
    })
}
pub fn compare(
    expected: &Expected,
    actual: &TypedResult,
    options: &Comparison,
) -> Result<Vec<String>> {
    let Expected::Result { columns, rows } = expected else {
        return Err("result compared to non-result expectation".into());
    };
    validate_result(columns, rows)?;
    validate_result(&actual.columns, &actual.rows)?;
    let mut errors = vec![];
    if columns.len() != actual.columns.len() {
        return Ok(vec![format!(
            "column count: expected {}, actual {}",
            columns.len(),
            actual.columns.len()
        )]);
    }
    for (i, (e, a)) in columns.iter().zip(&actual.columns).enumerate() {
        if !compatible(&e.kind, &a.kind, options.assert_physical_types)? {
            errors.push(format!(
                "column {i} type: expected {}, actual {}",
                e.kind, a.kind
            ))
        }
        if let Some(nullable) = e.physical_nullable
            && nullable != a.nullable
        {
            errors.push(format!(
                "column {i} provider nullability: expected {nullable}, actual {}",
                a.nullable
            ))
        }
        if options.assert_names && e.name != a.name {
            errors.push(format!(
                "column {i} name: expected {}, actual {}",
                e.name, a.name
            ))
        }
    }
    if !errors.is_empty() {
        return Ok(errors);
    }
    let equal = |e: &Vec<Value>, a: &Vec<Value>| -> Result<bool> {
        for ((ev, av), c) in e.iter().zip(a).zip(columns) {
            if ev.is_null() || av.is_null() {
                if ev != av {
                    return Ok(false);
                }
                continue;
            }
            let k = kind(&c.kind)?;

            if let Some(t) = &c.tolerance {
                let x = ev.as_f64().ok_or("invalid expected float")?;
                let y = av.as_f64().ok_or("invalid actual float")?;
                if (x - y).abs() > t.absolute.max(t.relative * x.abs().max(y.abs())) {
                    return Ok(false);
                }
            } else if matches!(k, Kind::Decimal(..)) {
                if decimal_canonical(string(ev)?)? != decimal_canonical(string(av)?)? {
                    return Ok(false);
                }
            } else if normalize(ev, &k)? != normalize(av, &k)? {
                return Ok(false);
            }
        }
        Ok(true)
    };
    if rows.len() != actual.rows.len() {
        errors.push(format!(
            "row count: expected {}, actual {}",
            rows.len(),
            actual.rows.len()
        ))
    }
    if rows.len() != actual.rows.len() {
        return Ok(errors);
    }
    if !options.ordered && columns.iter().all(|c| c.tolerance.is_none()) {
        let canonical = |row: &Vec<Value>, cols: &[Column]| -> Result<String> {
            let normalized = row
                .iter()
                .zip(cols)
                .map(|(value, column)| {
                    if value.is_null() {
                        Ok(Value::Null)
                    } else if matches!(kind(&column.kind)?, Kind::Decimal(..)) {
                        Ok(Value::String(decimal_canonical(string(value)?)?))
                    } else {
                        normalize(value, &kind(&column.kind)?)
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(serde_json::to_string(&normalized)?)
        };
        let mut expected = std::collections::BTreeMap::<String, usize>::new();
        let mut actual_counts = std::collections::BTreeMap::<String, usize>::new();
        for row in rows {
            *expected.entry(canonical(row, columns)?).or_default() += 1;
        }
        for row in &actual.rows {
            *actual_counts
                .entry(canonical(row, &actual.columns)?)
                .or_default() += 1;
        }
        if expected != actual_counts {
            errors
                .push("unordered row multiset differs (including duplicate multiplicities)".into());
        }
        return Ok(errors);
    }
    if !options.ordered && rows.len().saturating_mul(actual.rows.len()) > 1_000_000 {
        return Err("float tolerance matching work budget exceeded".into());
    }
    if options.ordered {
        for (i, (e, a)) in rows.iter().zip(&actual.rows).enumerate() {
            if !equal(e, a)? {
                errors.push(format!("row {i}: expected {e:?}, actual {a:?}"))
            }
        }
    } else {
        // Bipartite matching, rather than greedy removal: tolerance equality is not transitive.
        fn augment(
            i: usize,
            edges: &[Vec<usize>],
            seen: &mut [bool],
            owners: &mut [Option<usize>],
        ) -> bool {
            for &j in &edges[i] {
                if seen[j] {
                    continue;
                }
                seen[j] = true;
                if owners[j].is_none() || augment(owners[j].unwrap(), edges, seen, owners) {
                    owners[j] = Some(i);
                    return true;
                }
            }
            false
        }
        let edges: Vec<Vec<usize>> = rows
            .iter()
            .map(|e| {
                actual
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(j, a)| match equal(e, a) {
                        Ok(true) => Some(Ok(j)),
                        Ok(false) => None,
                        Err(e) => Some(Err(e)),
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<_>>()?;
        let mut owners = vec![None; actual.rows.len()];
        for (i, row) in rows.iter().enumerate() {
            if !augment(i, &edges, &mut vec![false; actual.rows.len()], &mut owners) {
                errors.push(format!("unmatched expected row {i}: {:?}", row))
            }
        }
    }
    Ok(errors)
}
fn type_name(t: &DataType) -> Result<String> {
    Ok(match t {
        DataType::UInt8 => "uint8".into(),
        DataType::UInt16 => "uint16".into(),
        DataType::UInt32 => "uint32".into(),
        DataType::UInt64 => "uint64".into(),
        DataType::Int16 => "int16".into(),
        DataType::Int32 => "int32".into(),
        DataType::Int64 => "int64".into(),
        DataType::Float32 => "float32".into(),
        DataType::Float64 => "float64".into(),
        DataType::Boolean => "boolean".into(),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => "utf8".into(),
        DataType::Date32 => "date32".into(),
        DataType::Decimal128(p, s) => format!("decimal128({p},{s})"),
        DataType::Time64(u) => format!("time64({})", unit_name(u)),
        DataType::Timestamp(u, tz) => format!(
            "timestamp({}{})",
            unit_name(u),
            tz.as_ref().map(|s| format!(",{s}")).unwrap_or_default()
        ),
        _ => return Err(format!("unsupported actual Arrow type {t:?}").into()),
    })
}
fn finite_float(v: f64) -> Result<Value> {
    if !v.is_finite() {
        return Err("nonfinite actual float".into());
    }
    Ok(serde_json::to_value(v)?)
}
fn unit_name(u: &TimeUnit) -> &str {
    match u {
        TimeUnit::Second => "s",
        TimeUnit::Millisecond => "ms",
        TimeUnit::Microsecond => "us",
        TimeUnit::Nanosecond => "ns",
    }
}
pub fn result_from_batches(schema: &Schema, batches: &[RecordBatch]) -> Result<TypedResult> {
    let columns = schema
        .fields()
        .iter()
        .map(|f| {
            Ok(Column {
                name: f.name().clone(),
                kind: type_name(f.data_type())?,
                nullable: f.is_nullable(),
                physical_nullable: None,
                tolerance: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut rows = vec![];
    for b in batches {
        if b.schema().as_ref() != schema {
            return Err("batch schema differs from output schema".into());
        }
        for i in 0..b.num_rows() {
            let mut row = vec![];
            for a in b.columns() {
                if a.is_null(i) {
                    row.push(Value::Null);
                    continue;
                }
                macro_rules! scalar {
                    ($ty:ty) => {
                        a.as_any()
                            .downcast_ref::<$ty>()
                            .ok_or("Arrow array type mismatch")?
                            .value(i)
                    };
                }
                let value = match a.data_type() {
                    DataType::UInt8 => Value::String(scalar!(UInt8Array).to_string()),
                    DataType::UInt16 => Value::String(scalar!(UInt16Array).to_string()),
                    DataType::UInt32 => Value::String(scalar!(UInt32Array).to_string()),
                    DataType::UInt64 => Value::String(scalar!(UInt64Array).to_string()),
                    DataType::Int16 => Value::String(scalar!(Int16Array).to_string()),
                    DataType::Int32 => Value::String(scalar!(Int32Array).to_string()),
                    DataType::Int64 => Value::String(scalar!(Int64Array).to_string()),
                    DataType::Float32 => finite_float(scalar!(Float32Array) as f64)?,
                    DataType::Float64 => finite_float(scalar!(Float64Array))?,
                    DataType::Boolean => Value::Bool(scalar!(BooleanArray)),
                    DataType::Utf8 => Value::String(scalar!(StringArray).into()),
                    DataType::LargeUtf8 => Value::String(scalar!(LargeStringArray).into()),
                    DataType::Utf8View => Value::String(scalar!(StringViewArray).into()),
                    DataType::Decimal128(_, scale) => {
                        let v = scalar!(Decimal128Array);
                        let sign = if v < 0 { "-" } else { "" };
                        let digits = v.unsigned_abs().to_string();
                        if *scale < 0 {
                            return Err("negative decimal scale unsupported".into());
                        }
                        let s = *scale as usize;
                        let padded = format!("{:0>width$}", digits, width = s + 1);
                        Value::String(if s == 0 {
                            format!("{sign}{padded}")
                        } else {
                            format!(
                                "{sign}{}.{}",
                                &padded[..padded.len() - s],
                                &padded[padded.len() - s..]
                            )
                        })
                    }
                    DataType::Date32 => {
                        let days = scalar!(Date32Array);
                        let d = NaiveDate::from_ymd_opt(1970, 1, 1)
                            .ok_or("invalid epoch")?
                            .checked_add_signed(chrono::Duration::days(days as i64))
                            .ok_or("date out of range")?;
                        Value::String(d.to_string())
                    }
                    DataType::Time64(u) => {
                        let n = match u {
                            TimeUnit::Microsecond => scalar!(Time64MicrosecondArray)
                                .checked_mul(1000)
                                .ok_or("time overflow")?,
                            TimeUnit::Nanosecond => scalar!(Time64NanosecondArray),
                            _ => return Err("unsupported time unit".into()),
                        };
                        if !(0..86_400_000_000_000).contains(&n) {
                            return Err("time outside day".into());
                        }
                        let t = NaiveTime::from_num_seconds_from_midnight_opt(
                            (n / 1_000_000_000) as u32,
                            (n % 1_000_000_000) as u32,
                        )
                        .ok_or("invalid time")?;
                        Value::String(t.format("%H:%M:%S%.f").to_string())
                    }
                    DataType::Timestamp(u, tz) => {
                        let n = match u {
                            TimeUnit::Second => scalar!(TimestampSecondArray),
                            TimeUnit::Millisecond => scalar!(TimestampMillisecondArray),
                            TimeUnit::Microsecond => scalar!(TimestampMicrosecondArray),
                            TimeUnit::Nanosecond => scalar!(TimestampNanosecondArray),
                        };
                        let factor = match u {
                            TimeUnit::Second => 1,
                            TimeUnit::Millisecond => 1000,
                            TimeUnit::Microsecond => 1_000_000,
                            TimeUnit::Nanosecond => 1_000_000_000,
                        };
                        let dt = chrono::DateTime::from_timestamp(
                            n.div_euclid(factor),
                            (n.rem_euclid(factor) * (1_000_000_000 / factor)) as u32,
                        )
                        .ok_or("timestamp out of range")?;
                        Value::String(if tz.is_some() {
                            dt.to_rfc3339()
                        } else {
                            dt.naive_utc().format("%Y-%m-%dT%H:%M:%S%.f").to_string()
                        })
                    }
                    _ => return Err("unsupported output type".into()),
                };
                row.push(value)
            }
            rows.push(row)
        }
    }
    let result = TypedResult { columns, rows };
    validate_result(&result.columns, &result.rows)?;
    Ok(result)
}

fn decimal_canonical(s: &str) -> Result<String> {
    let negative = s.starts_with("-");
    let s = s.strip_prefix("-").unwrap_or(s);
    let (whole, fraction) = s.split_once(".").unwrap_or((s, ""));
    let whole = whole.trim_start_matches("0");
    let fraction = fraction.trim_end_matches("0");
    let whole = if whole.is_empty() { "0" } else { whole };
    Ok(format!(
        "{}{}{}",
        if negative && (whole != "0" || !fraction.is_empty()) {
            "-"
        } else {
            ""
        },
        whole,
        if fraction.is_empty() {
            String::new()
        } else {
            format!(".{fraction}")
        }
    ))
}
