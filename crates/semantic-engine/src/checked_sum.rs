//! Exact aggregation state. The native DataFusion SUM uses wrapping arithmetic;
//! compiler sums accumulate wider and check the declared result type at finalize.
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Decimal256Array, ListArray},
        datatypes::{DataType, Field, FieldRef, i256},
    },
    common::ScalarValue,
    error::{DataFusionError, Result},
    logical_expr::{
        Accumulator, AggregateUDF, AggregateUDFImpl, Signature, Volatility,
        function::{AccumulatorArgs, StateFieldsArgs},
    },
};
use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};
fn error() -> DataFusionError {
    DataFusionError::Execution("semantic sum overflow or invalid numeric state".into())
}
pub fn semantic_sum_v1() -> Arc<AggregateUDF> {
    static FUNCTION: OnceLock<Arc<AggregateUDF>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            Arc::new(AggregateUDF::from(CheckedSum {
                signature: Signature::user_defined(Volatility::Immutable),
            }))
        })
        .clone()
}
#[derive(Debug, PartialEq, Eq, Hash)]
struct CheckedSum {
    signature: Signature,
}
impl AggregateUDFImpl for CheckedSum {
    fn name(&self) -> &str {
        "semantic_sum_v1"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn coerce_types(&self, args: &[DataType]) -> Result<Vec<DataType>> {
        match args {
            [DataType::Int16 | DataType::Int32 | DataType::Int64] => Ok(vec![DataType::Int64]),
            [DataType::Decimal128(p, s)] if (1..=38).contains(p) && *s >= 0 && *s <= *p as i8 => {
                Ok(args.to_vec())
            }
            _ => Err(DataFusionError::Plan(
                "semantic_sum_v1 requires a signed integer or supported Decimal128".into(),
            )),
        }
    }
    fn return_type(&self, args: &[DataType]) -> Result<DataType> {
        match self.coerce_types(args)?.as_slice() {
            [DataType::Int64] => Ok(DataType::Int64),
            [DataType::Decimal128(p, s)] => {
                Ok(DataType::Decimal128(p.saturating_add(10).min(38), *s))
            }
            _ => Err(error()),
        }
    }
    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(SumState {
            result: args.return_type().clone(),
            sum: i256::ZERO,
            seen: false,
            distinct: args.is_distinct.then(BTreeSet::new),
        }))
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        let scale = scale(args.return_type());
        let field = if args.is_distinct {
            Field::new_list(
                format!("{}[distinct_values]", args.name),
                Field::new_list_field(DataType::Decimal128(38, scale), true),
                false,
            )
        } else {
            Field::new(
                format!("{}[wide_sum]", args.name),
                DataType::Decimal256(76, scale),
                true,
            )
        };
        Ok(vec![Arc::new(field)])
    }
}
fn scale(ty: &DataType) -> i8 {
    match ty {
        DataType::Decimal128(_, scale) => *scale,
        _ => 0,
    }
}
#[derive(Debug)]
struct SumState {
    result: DataType,
    sum: i256,
    seen: bool,
    distinct: Option<BTreeSet<i128>>,
}
impl SumState {
    fn add(&mut self, value: i128) -> Result<()> {
        if self
            .distinct
            .as_mut()
            .is_some_and(|values| !values.insert(value))
        {
            return Ok(());
        }
        self.sum = self
            .sum
            .checked_add(i256::from_i128(value))
            .ok_or_else(error)?;
        self.seen = true;
        Ok(())
    }
}
impl Accumulator for SumState {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let [array] = values else { return Err(error()) };
        for index in 0..array.len() {
            if array.is_null(index) {
                continue;
            }
            let value = match ScalarValue::try_from_array(array, index)? {
                ScalarValue::Int64(Some(v)) => i128::from(v),
                ScalarValue::Decimal128(Some(v), _, _) => v,
                _ => return Err(error()),
            };
            self.add(value)?;
        }
        Ok(())
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        let value = self
            .seen
            .then(|| self.sum.to_i128().ok_or_else(error))
            .transpose()?;
        match self.result {
            DataType::Int64 => Ok(ScalarValue::Int64(
                value
                    .map(|v| i64::try_from(v).map_err(|_| error()))
                    .transpose()?,
            )),
            DataType::Decimal128(precision, scale) => {
                if value.is_some_and(|v| v.unsigned_abs() >= 10u128.pow(u32::from(precision))) {
                    return Err(error());
                }
                Ok(ScalarValue::Decimal128(value, precision, scale))
            }
            _ => Err(error()),
        }
    }
    fn size(&self) -> usize {
        size_of::<Self>() + self.distinct.as_ref().map_or(0, |values| values.len() * 64)
    }
    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![if let Some(values) = &self.distinct {
            let scale = scale(&self.result);
            ScalarValue::List(ScalarValue::new_list(
                &values
                    .iter()
                    .map(|v| ScalarValue::Decimal128(Some(*v), 38, scale))
                    .collect::<Vec<_>>(),
                &DataType::Decimal128(38, scale),
                true,
            ))
        } else {
            ScalarValue::Decimal256(self.seen.then_some(self.sum), 76, scale(&self.result))
        }])
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let [array] = states else { return Err(error()) };
        if self.distinct.is_some() {
            let lists = array
                .as_any()
                .downcast_ref::<ListArray>()
                .ok_or_else(error)?;
            for index in 0..lists.len() {
                if lists.is_valid(index) {
                    self.update_batch(&[lists.value(index)])?;
                }
            }
        } else {
            let sums = array
                .as_any()
                .downcast_ref::<Decimal256Array>()
                .ok_or_else(error)?;
            for index in 0..sums.len() {
                if sums.is_valid(index) {
                    self.sum = self.sum.checked_add(sums.value(index)).ok_or_else(error)?;
                    self.seen = true;
                }
            }
        }
        Ok(())
    }
}
