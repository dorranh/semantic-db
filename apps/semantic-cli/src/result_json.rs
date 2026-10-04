//! Lossless schema and scalar JSON output for public commands.
use chrono::{NaiveDate, NaiveTime};
use datafusion::arrow::{
    array::*,
    datatypes::{DataType, Schema, TimeUnit},
    record_batch::RecordBatch,
};
use serde::Serialize;
use serde_json::Value;
type Result<T> = super::Result<T>;
#[derive(Serialize)]
pub struct Column {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    nullable: bool,
    tolerance: Option<()>,
}
#[derive(Serialize)]
pub struct TypedResult {
    columns: Vec<Column>,
    rows: Vec<Vec<Value>>,
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
    Ok(TypedResult { columns, rows })
}
