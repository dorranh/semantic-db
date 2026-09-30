//! Exact weighted mean over Int64 values and nonnegative Int64 weights.
//! Partial states are checked wide integer components, never finalized means.
use std::sync::{Arc, OnceLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Decimal256Array, Int64Array},
        datatypes::{DataType, Field, FieldRef, i256},
    },
    common::ScalarValue,
    error::{DataFusionError, Result},
    logical_expr::{
        Accumulator, AggregateUDF, AggregateUDFImpl, Signature, Volatility,
        function::{AccumulatorArgs, StateFieldsArgs},
    },
};

fn error() -> DataFusionError {
    DataFusionError::Execution("semantic weighted mean overflow or invalid state".into())
}

pub fn semantic_weighted_mean_i64_v1() -> Arc<AggregateUDF> {
    static FUNCTION: OnceLock<Arc<AggregateUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(AggregateUDF::from(CheckedWeightedMean {
                signature: Signature::user_defined(Volatility::Immutable),
            }))
        })
        .clone()
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct CheckedWeightedMean {
    signature: Signature,
}
impl AggregateUDFImpl for CheckedWeightedMean {
    fn name(&self) -> &str {
        "semantic_weighted_mean_i64_v1"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn coerce_types(&self, args: &[DataType]) -> Result<Vec<DataType>> {
        match args {
            [DataType::Int64, DataType::Int64] => Ok(args.to_vec()),
            _ => Err(DataFusionError::Plan(
                "semantic_weighted_mean_i64_v1 requires value and weight Int64 inputs".into(),
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
                "semantic_weighted_mean_i64_v1 does not accept DISTINCT".into(),
            ));
        }
        Ok(Box::new(WeightedState::default()))
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![
            Arc::new(Field::new(
                format!("{}[weighted_sum]", args.name),
                DataType::Decimal256(76, 0),
                true,
            )),
            Arc::new(Field::new(
                format!("{}[weight_sum]", args.name),
                DataType::Decimal256(76, 0),
                true,
            )),
        ])
    }
}

#[derive(Debug, Default)]
struct WeightedState {
    weighted_sum: i128,
    weight_sum: i128,
    seen: bool,
}
impl WeightedState {
    fn add(&mut self, weighted: i128, weight: i128) -> Result<()> {
        if weight < 0 {
            return Err(error());
        }
        self.weighted_sum = self.weighted_sum.checked_add(weighted).ok_or_else(error)?;
        self.weight_sum = self.weight_sum.checked_add(weight).ok_or_else(error)?;
        self.seen = true;
        Ok(())
    }
}
impl Accumulator for WeightedState {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let [values, weights] = values else {
            return Err(error());
        };
        let values = values
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(error)?;
        let weights = weights
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(error)?;
        if values.len() != weights.len() {
            return Err(error());
        }
        for index in 0..values.len() {
            if values.is_null(index) || weights.is_null(index) {
                continue;
            }
            let weight = i128::from(weights.value(index));
            let product = i128::from(values.value(index))
                .checked_mul(weight)
                .ok_or_else(error)?;
            self.add(product, weight)?;
        }
        Ok(())
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        if !self.seen || self.weight_sum == 0 {
            return Ok(ScalarValue::Decimal128(None, 38, 18));
        }
        let scaled = i256::from_i128(self.weighted_sum)
            .checked_mul(i256::from_i128(1_000_000_000_000_000_000))
            .ok_or_else(error)?;
        let coefficient = scaled
            .checked_div(i256::from_i128(self.weight_sum))
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
            ScalarValue::Decimal256(self.seen.then(|| i256::from_i128(self.weighted_sum)), 76, 0),
            ScalarValue::Decimal256(self.seen.then(|| i256::from_i128(self.weight_sum)), 76, 0),
        ])
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let [weighted, weights] = states else {
            return Err(error());
        };
        let weighted = weighted
            .as_any()
            .downcast_ref::<Decimal256Array>()
            .ok_or_else(error)?;
        let weights = weights
            .as_any()
            .downcast_ref::<Decimal256Array>()
            .ok_or_else(error)?;
        if weighted.len() != weights.len() {
            return Err(error());
        }
        for index in 0..weighted.len() {
            if weighted.is_null(index) != weights.is_null(index) {
                return Err(error());
            }
            if weighted.is_null(index) {
                continue;
            }
            let numerator = weighted.value(index).to_i128().ok_or_else(error)?;
            let denominator = weights.value(index).to_i128().ok_or_else(error)?;
            self.add(numerator, denominator)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_components_reject_overflow_and_negative_weight() {
        let mut state = WeightedState::default();
        assert!(state.add(0, -1).is_err());
        state.add(i128::MAX, i128::MAX).unwrap();
        assert!(state.add(1, 1).is_err());
    }
}
