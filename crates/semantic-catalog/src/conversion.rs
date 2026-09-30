//! Authored exact rational unit conversion. Expressions are data, never SQL.

use crate::{ObjectRef, SourceRef, Unit, canonical_digest, valid_unit};
use serde::{Deserialize, Serialize};

pub const UNIT_CONVERSION_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionRounding {
    Truncate,
    HalfEven,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitConversion {
    pub version: u32,
    pub id: String,
    pub field: String,
    pub from_unit: Unit,
    pub to_unit: Unit,
    pub numerator: i64,
    pub denominator: i64,
    pub rounding: ConversionRounding,
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConversionError {
    #[error("unsupported unit conversion version")]
    Version,
    #[error("unit conversion requires valid typed unit identities")]
    Identity,
    #[error("unit conversion rational factor is invalid")]
    Factor,
}

impl UnitConversion {
    pub fn validate(&self) -> Result<(), ConversionError> {
        fn bounded_unit(unit: &Unit, depth: usize) -> bool {
            if depth > 8 {
                return false;
            }
            match unit {
                Unit::Dimensionless | Unit::Currency { .. } => true,
                Unit::Named { id } => id.len() <= 256,
                Unit::Quotient {
                    numerator,
                    denominator,
                } => bounded_unit(numerator, depth + 1) && bounded_unit(denominator, depth + 1),
            }
        }
        if self.version != UNIT_CONVERSION_VERSION {
            return Err(ConversionError::Version);
        }
        if self.id.trim().is_empty()
            || self.field.trim().is_empty()
            || !valid_unit(&self.from_unit)
            || !valid_unit(&self.to_unit)
            || !bounded_unit(&self.from_unit, 0)
            || !bounded_unit(&self.to_unit, 0)
        {
            return Err(ConversionError::Identity);
        }
        if self.numerator <= 0 || self.denominator <= 0 {
            return Err(ConversionError::Factor);
        }
        Ok(())
    }

    pub fn reference(&self) -> ObjectRef {
        let mut canonical = self.clone();
        canonical.source_refs.clear();
        ObjectRef {
            id: self.id.clone(),
            revision: canonical_digest(
                &serde_json::to_value(canonical).expect("unit conversion serializes"),
            ),
        }
    }
}
