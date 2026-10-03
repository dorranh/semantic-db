//! Typed meaning carried with a graph slot, independently of its Arrow field.

use crate::{FactResolution, Presence};
pub use semantic_plan::meaning::{EntityId, GrainKey, SourceGrain, Unit};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Authored comparison semantics for a field. A locale declaration is retained
/// for provenance but requires an explicit checked collation profile to execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComparisonProfile {
    BinaryExact,
    Locale { tag: String },
}

/// Exact authored identity for a field's key domain. Matching Arrow types do
/// not establish that two fields use the same identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSystem {
    pub id: String,
}

/// An authored vocabulary identity; equal UTF-8 code bytes need not mean the
/// same thing in two domains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnumDomain {
    pub id: String,
}

pub fn enum_domains_compatible(left: Option<&EnumDomain>, right: Option<&EnumDomain>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            !left.id.trim().is_empty() && !right.id.trim().is_empty() && left == right
        }
        _ => false,
    }
}

/// Authored calendar semantics are separate from Arrow's timestamp storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CalendarSystem {
    Gregorian,
    Fiscal { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarReference {
    pub system: CalendarSystem,
    pub timezone: String,
}

pub fn valid_calendar_reference(reference: &CalendarReference) -> bool {
    reference.timezone.parse::<chrono_tz::Tz>().is_ok()
        && match &reference.system {
            CalendarSystem::Gregorian => true,
            CalendarSystem::Fiscal { id } => !id.trim().is_empty(),
        }
}

/// Unknown on both sides preserves the existing physical-key profile. Once
/// either side is authored, both sides must carry the same valid identity.
pub fn reference_systems_compatible(
    left: Option<&ReferenceSystem>,
    right: Option<&ReferenceSystem>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            !left.id.trim().is_empty() && !right.id.trim().is_empty() && left == right
        }
        _ => false,
    }
}

/// Enforce a bounded, nonempty authored unit tree before publication.
pub fn valid_unit(unit: &Unit) -> bool {
    fn valid(unit: &Unit, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        match unit {
            Unit::Dimensionless => true,
            Unit::Named { id } => !id.trim().is_empty(),
            Unit::Currency { code } => {
                code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_uppercase())
            }
            Unit::Quotient {
                numerator,
                denominator,
            } => valid(numerator, depth + 1) && valid(denominator, depth + 1),
        }
    }
    valid(unit, 0)
}

/// The minimal metadata bridge for graph slots. Existing fact resolution keeps
/// authority and source evidence; `Unknown` is distinct from explicit `Null`.
/// A conflict must be resolved before the slot participates in arithmetic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotMeaning {
    pub unit: FactResolution<Presence<Unit>>,
    pub source_grain: FactResolution<Presence<SourceGrain>>,
    pub entity: FactResolution<Presence<EntityId>>,
}

impl Default for SlotMeaning {
    fn default() -> Self {
        Self {
            unit: FactResolution::Unknown,
            source_grain: FactResolution::Unknown,
            entity: FactResolution::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UnitQuotientError {
    #[error("an operand has an explicit null unit")]
    NullOperand,
    #[error("an operand has conflicting unit facts")]
    ConflictingOperand,
}

/// Compute the unit of a ratio. An unknown operand leaves the result unknown;
/// it never implies dimensionless. The returned value is derived, so it does
/// not fabricate an authored `Fact` or authority for the result.
pub fn checked_unit_quotient(
    numerator: &FactResolution<Presence<Unit>>,
    denominator: &FactResolution<Presence<Unit>>,
) -> Result<Presence<Unit>, UnitQuotientError> {
    fn known_unit(
        value: &FactResolution<Presence<Unit>>,
    ) -> Result<Option<&Unit>, UnitQuotientError> {
        match value {
            FactResolution::Unknown
            | FactResolution::Known {
                value: Presence::Missing,
                ..
            } => Ok(None),
            FactResolution::Known {
                value: Presence::Null,
                ..
            } => Err(UnitQuotientError::NullOperand),
            FactResolution::Known {
                value: Presence::Value(unit),
                ..
            } => Ok(Some(unit)),
            FactResolution::Conflicting { .. } => Err(UnitQuotientError::ConflictingOperand),
        }
    }

    let numerator = known_unit(numerator)?;
    let denominator = known_unit(denominator)?;
    Ok(match (numerator, denominator) {
        (Some(left), Some(right)) if left == right => Presence::Value(Unit::Dimensionless),
        (Some(left), Some(right)) => Presence::Value(Unit::Quotient {
            numerator: Box::new(left.clone()),
            denominator: Box::new(right.clone()),
        }),
        _ => Presence::Missing,
    })
}
