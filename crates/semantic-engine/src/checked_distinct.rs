//! Mergeable exact distinct count for one Int64 identity. Scalar counts are
//! never merged; each partial state carries its sorted identity set.
use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Int64Array, ListArray},
        datatypes::{DataType, Field, FieldRef},
    },
    common::ScalarValue,
    error::{DataFusionError, Result},
    logical_expr::{
        Accumulator, AggregateUDF, AggregateUDFImpl, Signature, Volatility,
        function::{AccumulatorArgs, StateFieldsArgs},
    },
};

/// Deterministic per-group bound, independent of partition layout.
pub const EXACT_DISTINCT_MAX_IDENTITIES_V1: usize = 1_000_000;

fn error(message: &str) -> DataFusionError {
    DataFusionError::Execution(message.into())
}

pub fn semantic_exact_count_i64_v1() -> Arc<AggregateUDF> {
    static FUNCTION: OnceLock<Arc<AggregateUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(AggregateUDF::from(ExactDistinctCount {
                signature: Signature::user_defined(Volatility::Immutable),
            }))
        })
        .clone()
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct ExactDistinctCount {
    signature: Signature,
}
impl AggregateUDFImpl for ExactDistinctCount {
    fn name(&self) -> &str {
        "semantic_exact_count_i64_v1"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn coerce_types(&self, args: &[DataType]) -> Result<Vec<DataType>> {
        match args {
            [DataType::Int64] => Ok(args.to_vec()),
            _ => Err(DataFusionError::Plan(
                "semantic_exact_count_i64_v1 requires exactly one Int64 identity".into(),
            )),
        }
    }
    fn return_type(&self, args: &[DataType]) -> Result<DataType> {
        self.coerce_types(args)?;
        Ok(DataType::Int64)
    }
    fn accumulator(&self, _: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(ExactDistinctState::new(
            EXACT_DISTINCT_MAX_IDENTITIES_V1,
        )))
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new_list(
            format!("{}[identities]", args.name),
            Field::new_list_field(DataType::Int64, false),
            false,
        ))])
    }
}

#[derive(Debug)]
struct ExactDistinctState {
    values: BTreeSet<i64>,
    limit: usize,
}
impl ExactDistinctState {
    fn new(limit: usize) -> Self {
        Self {
            values: BTreeSet::new(),
            limit,
        }
    }
    fn insert(&mut self, value: i64) -> Result<()> {
        if !self.values.contains(&value) && self.values.len() >= self.limit {
            return Err(error("exact distinct identity limit exceeded"));
        }
        self.values.insert(value);
        Ok(())
    }
}
impl Accumulator for ExactDistinctState {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let [values] = values else {
            return Err(error("exact distinct input arity"));
        };
        let values = values
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| error("exact distinct identity type"))?;
        for index in 0..values.len() {
            if values.is_valid(index) {
                self.insert(values.value(index))?;
            }
        }
        Ok(())
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Int64(Some(
            i64::try_from(self.values.len()).map_err(|_| error("exact distinct count overflow"))?,
        )))
    }
    fn size(&self) -> usize {
        size_of::<Self>() + self.values.len() * 48
    }
    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::List(ScalarValue::new_list(
            &self
                .values
                .iter()
                .copied()
                .map(|v| ScalarValue::Int64(Some(v)))
                .collect::<Vec<_>>(),
            &DataType::Int64,
            false,
        ))])
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let [states] = states else {
            return Err(error("exact distinct state arity"));
        };
        let lists = states
            .as_any()
            .downcast_ref::<ListArray>()
            .ok_or_else(|| error("exact distinct state type"))?;
        for index in 0..lists.len() {
            if lists.is_null(index) {
                return Err(error("exact distinct state is null"));
            }
            self.update_batch(&[lists.value(index)])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_is_deterministic_and_duplicates_do_not_consume_capacity() {
        let mut state = ExactDistinctState::new(2);
        state.insert(2).unwrap();
        state.insert(1).unwrap();
        state.insert(2).unwrap();
        assert!(state.insert(3).is_err());
        assert_eq!(state.evaluate().unwrap(), ScalarValue::Int64(Some(2)));
        let encoded = state.state().unwrap();
        assert_eq!(encoded.len(), 1);
    }
}
