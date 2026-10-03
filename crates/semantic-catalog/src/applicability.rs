//! Decidable, exact temporal applicability for authored view contracts.
//! No SQL predicate implication or timezone conversion is inferred here.

use crate::{ObjectRef, SourceRef, canonical_digest};
use semantic_plan::typed::{CalendarUnit, Literal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewTemporalCoverage {
    pub field: String,
    /// The finest row-level calendar grain preserved by this view.
    pub grain: CalendarUnit,
    pub start: Literal,
    pub end: Literal,
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApplicabilityError {
    #[error("invalid or incomparable temporal coverage")]
    InvalidCoverage,
    #[error("requested time field differs from the authored view field")]
    Field,
    #[error("requested calendar grain is finer than or incomparable with the view grain")]
    Grain,
    #[error("requested interval is outside the authored view coverage")]
    Coverage,
}

impl ViewTemporalCoverage {
    pub fn reference(&self, relation: &str) -> ObjectRef {
        let mut canonical = self.clone();
        canonical.source_refs.clear();
        ObjectRef {
            id: format!("view_coverage/{relation}"),
            revision: canonical_digest(
                &serde_json::to_value(canonical).expect("view coverage serializes"),
            ),
        }
    }

    pub fn validate(&self) -> Result<(), ApplicabilityError> {
        if self.field.trim().is_empty() || !ordered(&self.start, &self.end) {
            return Err(ApplicabilityError::InvalidCoverage);
        }
        Ok(())
    }

    pub fn contains(
        &self,
        field: &str,
        grain: CalendarUnit,
        start: &Literal,
        end: &Literal,
    ) -> Result<(), ApplicabilityError> {
        self.validate()?;
        if field != self.field {
            return Err(ApplicabilityError::Field);
        }
        if !grain_refines(self.grain, grain) {
            return Err(ApplicabilityError::Grain);
        }
        if !ordered(start, end)
            || !less_or_equal(&self.start, start)
            || !less_or_equal(end, &self.end)
        {
            return Err(ApplicabilityError::Coverage);
        }
        Ok(())
    }
}

/// ISO weeks are deliberately incomparable with months and calendar years.
pub fn grain_refines(source: CalendarUnit, requested: CalendarUnit) -> bool {
    use CalendarUnit::*;
    matches!(
        (source, requested),
        (Day, Day | IsoWeek | Month | Year)
            | (IsoWeek, IsoWeek)
            | (Month, Month | Year)
            | (Year, Year)
    )
}

fn ordered(start: &Literal, end: &Literal) -> bool {
    matches!(compare(start, end), Some(std::cmp::Ordering::Less))
}
fn less_or_equal(left: &Literal, right: &Literal) -> bool {
    matches!(
        compare(left, right),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    )
}
fn compare(left: &Literal, right: &Literal) -> Option<std::cmp::Ordering> {
    match (left, right) {
        (Literal::Date32(a), Literal::Date32(b)) => Some(a.cmp(b)),
        (
            Literal::Timestamp {
                ticks: a,
                unit: au,
                timezone: az,
            },
            Literal::Timestamp {
                ticks: b,
                unit: bu,
                timezone: bz,
            },
        ) if au == bu && az == bz && matches!(az.as_deref(), Some("UTC" | "+00:00")) => {
            Some(a.cmp(b))
        }
        _ => None,
    }
}
