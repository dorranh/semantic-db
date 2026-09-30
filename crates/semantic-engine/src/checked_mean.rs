//! Exact Int64 mean. Mergeable state is a checked wide sum plus count; finalized
//! subgroup means are never averaged. Decimal output truncates toward zero.
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Decimal256Array, Int64Array, UInt64Array},
        datatypes::{DataType, Field, FieldRef, i256},
    },
    common::ScalarValue,
    error::{DataFusionError, Result},
    logical_expr::{
        Accumulator, AggregateUDF, AggregateUDFImpl, Signature, Volatility,
        function::{AccumulatorArgs, StateFieldsArgs},
    },
};
use std::sync::{Arc, OnceLock};

fn error() -> DataFusionError {
    DataFusionError::Execution("semantic mean overflow or invalid numeric state".into())
}

pub fn semantic_mean_i64_v1() -> Arc<AggregateUDF> {
    static FUNCTION: OnceLock<Arc<AggregateUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(AggregateUDF::from(CheckedMean {
                signature: Signature::user_defined(Volatility::Immutable),
            }))
        })
        .clone()
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct CheckedMean {
    signature: Signature,
}
impl AggregateUDFImpl for CheckedMean {
    fn name(&self) -> &str {
        "semantic_mean_i64_v1"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn coerce_types(&self, args: &[DataType]) -> Result<Vec<DataType>> {
        match args {
            [DataType::Int64] => Ok(args.to_vec()),
            _ => Err(DataFusionError::Plan(
                "semantic_mean_i64_v1 requires exactly one Int64 input".into(),
            )),
        }
    }
    fn return_type(&self, args: &[DataType]) -> Result<DataType> {
        self.coerce_types(args)?;
        Ok(DataType::Decimal128(38, 18))
    }
    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        if args.is_distinct {
            return Err(DataFusionError::Plan(
                "semantic_mean_i64_v1 does not accept DISTINCT".into(),
            ));
        }
        Ok(Box::new(MeanState {
            sum: i256::ZERO,
            count: 0,
        }))
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![
            Arc::new(Field::new(
                format!("{}[wide_sum]", args.name),
                DataType::Decimal256(76, 0),
                true,
            )),
            Arc::new(Field::new(
                format!("{}[count]", args.name),
                DataType::UInt64,
                false,
            )),
        ])
    }
}

#[derive(Debug)]
struct MeanState {
    sum: i256,
    count: u64,
}
impl MeanState {
    fn add(&mut self, sum: i256, count: u64) -> Result<()> {
        let next_sum = self.sum.checked_add(sum).ok_or_else(error)?;
        let next_count = self.count.checked_add(count).ok_or_else(error)?;
        self.sum = next_sum;
        self.count = next_count;
        Ok(())
    }
}
impl Accumulator for MeanState {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let [array] = values else {
            return Err(error());
        };
        let values = array
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(error)?;
        for index in 0..values.len() {
            if values.is_valid(index) {
                self.add(i256::from_i128(i128::from(values.value(index))), 1)?;
            }
        }
        Ok(())
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        if self.count == 0 {
            return Ok(ScalarValue::Decimal128(None, 38, 18));
        }
        let scaled = self
            .sum
            .checked_mul(i256::from_i128(1_000_000_000_000_000_000))
            .ok_or_else(error)?;
        let denominator = i256::from_i128(i128::from(self.count));
        let coefficient = scaled
            .checked_div(denominator)
            .and_then(i256::to_i128)
            .ok_or_else(error)?;
        if coefficient.unsigned_abs() >= 10u128.pow(38) {
            return Err(error());
        }
        Ok(ScalarValue::Decimal128(Some(coefficient), 38, 18))
    }
    fn size(&self) -> usize {
        size_of::<Self>()
    }
    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![
            ScalarValue::Decimal256((self.count > 0).then_some(self.sum), 76, 0),
            ScalarValue::UInt64(Some(self.count)),
        ])
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let [sums, counts] = states else {
            return Err(error());
        };
        let sums = sums
            .as_any()
            .downcast_ref::<Decimal256Array>()
            .ok_or_else(error)?;
        let counts = counts
            .as_any()
            .downcast_ref::<UInt64Array>()
            .ok_or_else(error)?;
        if sums.len() != counts.len() {
            return Err(error());
        }
        for index in 0..counts.len() {
            if counts.is_null(index) {
                return Err(error());
            }
            let count = counts.value(index);
            if (count == 0) != sums.is_null(index) {
                return Err(error());
            }
            if count > 0 {
                self.add(sums.value(index), count)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_merge_checks_sum_count_and_final_precision() {
        let mut sum = MeanState {
            sum: i256::MAX,
            count: 1,
        };
        assert!(sum.add(i256::from_i128(1), 1).is_err());
        let mut count = MeanState {
            sum: i256::ZERO,
            count: u64::MAX,
        };
        assert!(count.add(i256::ZERO, 1).is_err());
        let mut finalize = MeanState {
            sum: i256::MAX,
            count: 1,
        };
        assert!(finalize.evaluate().is_err());
    }
}
