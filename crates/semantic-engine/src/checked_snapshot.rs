//! Dated snapshot balance selection. The state chooses the latest observation
//! by UTC microsecond instant and authored Int64 tie key; it never sums values.
use std::sync::{Arc, OnceLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Int64Array, TimestampMicrosecondArray},
        datatypes::{DataType, Field, FieldRef, TimeUnit},
    },
    common::ScalarValue,
    error::{DataFusionError, Result},
    logical_expr::{
        Accumulator, AggregateUDF, AggregateUDFImpl, Signature, Volatility,
        function::{AccumulatorArgs, StateFieldsArgs},
    },
};

fn error(message: &str) -> DataFusionError {
    DataFusionError::Execution(message.into())
}

pub fn semantic_snapshot_balance_i64_v1() -> Arc<AggregateUDF> {
    static FUNCTION: OnceLock<Arc<AggregateUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(AggregateUDF::from(SnapshotBalance {
                signature: Signature::user_defined(Volatility::Immutable),
            }))
        })
        .clone()
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct SnapshotBalance {
    signature: Signature,
}
impl AggregateUDFImpl for SnapshotBalance {
    fn name(&self) -> &str {
        "semantic_snapshot_balance_i64_v1"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn coerce_types(&self, args: &[DataType]) -> Result<Vec<DataType>> {
        match args {
            [DataType::Int64, DataType::Timestamp(TimeUnit::Microsecond, Some(zone)), DataType::Int64]
                if zone.as_ref() == "UTC" => Ok(args.to_vec()),
            _ => Err(DataFusionError::Plan(
                "semantic_snapshot_balance_i64_v1 requires Int64 value, UTC microsecond timestamp, and Int64 tie key".into(),
            )),
        }
    }
    fn return_type(&self, args: &[DataType]) -> Result<DataType> {
        self.coerce_types(args)?;
        Ok(DataType::Int64)
    }
    fn accumulator(&self, _: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(SnapshotState::default()))
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(["observed_at", "tie_key", "value"]
            .into_iter()
            .map(|part| {
                Arc::new(Field::new(
                    format!("{}[{part}]", args.name),
                    DataType::Int64,
                    true,
                ))
            })
            .collect())
    }
}

#[derive(Debug, Default)]
struct SnapshotState {
    latest: Option<(i64, i64, Option<i64>)>,
}
impl SnapshotState {
    fn select(&mut self, observed_at: i64, tie_key: i64, value: Option<i64>) -> Result<()> {
        match self.latest {
            None => self.latest = Some((observed_at, tie_key, value)),
            Some((time, tie, previous)) => match (observed_at, tie_key).cmp(&(time, tie)) {
                std::cmp::Ordering::Greater => self.latest = Some((observed_at, tie_key, value)),
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal if previous == value => {}
                std::cmp::Ordering::Equal => {
                    return Err(error(
                        "snapshot balance has conflicting equal-order observations",
                    ));
                }
            },
        }
        Ok(())
    }
}
impl Accumulator for SnapshotState {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let [values, times, ties] = values else {
            return Err(error("snapshot balance input arity"));
        };
        let values = values
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| error("snapshot balance value type"))?;
        let times = times
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .ok_or_else(|| error("snapshot balance time type"))?;
        let ties = ties
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| error("snapshot balance tie type"))?;
        if values.len() != times.len() || values.len() != ties.len() {
            return Err(error("snapshot balance input lengths"));
        }
        for index in 0..values.len() {
            if times.is_null(index) || ties.is_null(index) {
                return Err(error("snapshot balance time or tie is null"));
            }
            self.select(
                times.value(index),
                ties.value(index),
                values.is_valid(index).then(|| values.value(index)),
            )?;
        }
        Ok(())
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Int64(
            self.latest.and_then(|(_, _, value)| value),
        ))
    }
    fn size(&self) -> usize {
        size_of::<Self>()
    }
    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![
            ScalarValue::Int64(self.latest.map(|(time, _, _)| time)),
            ScalarValue::Int64(self.latest.map(|(_, tie, _)| tie)),
            ScalarValue::Int64(self.latest.and_then(|(_, _, value)| value)),
        ])
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let [times, ties, values] = states else {
            return Err(error("snapshot balance state arity"));
        };
        let times = times
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| error("snapshot balance state time type"))?;
        let ties = ties
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| error("snapshot balance state tie type"))?;
        let values = values
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| error("snapshot balance state value type"))?;
        if times.len() != ties.len() || times.len() != values.len() {
            return Err(error("snapshot balance state lengths"));
        }
        for index in 0..times.len() {
            if times.is_null(index) {
                if ties.is_valid(index) || values.is_valid(index) {
                    return Err(error("snapshot balance empty state is malformed"));
                }
                continue;
            }
            if ties.is_null(index) {
                return Err(error("snapshot balance tie state is null"));
            }
            self.select(
                times.value(index),
                ties.value(index),
                values.is_valid(index).then(|| values.value(index)),
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_tie_and_conflict_are_deterministic() {
        let mut state = SnapshotState::default();
        state.select(10, 1, Some(100)).unwrap();
        state.select(9, 99, Some(999)).unwrap();
        state.select(10, 2, Some(200)).unwrap();
        state.select(10, 2, Some(200)).unwrap();
        assert_eq!(state.evaluate().unwrap(), ScalarValue::Int64(Some(200)));
        assert!(state.select(10, 2, Some(201)).is_err());
        state.select(11, 0, None).unwrap();
        assert_eq!(state.evaluate().unwrap(), ScalarValue::Int64(None));
    }
}
