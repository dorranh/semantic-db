//! Versioned sufficient states for exact metric merging. These contracts do not
//! grant a compiler permission to merge overlapping fact populations.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

pub const METRIC_STATE_VERSION: u32 = 2;
pub const METRIC_DECIMAL_SCALE: u32 = 18;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricStateContract {
    pub version: u32,
    pub state: MetricStateKind,
    /// Exact dimensions across which this authored state may be combined.
    pub merge_dimensions: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetricStateKind {
    /// Finalize a mean only after combining its checked sum and row count.
    SumCountAverage,
    /// Finalize a weighted mean only after combining weighted sum and weight.
    WeightedAverage {
        weight_field: String,
        zero: ZeroWeight,
    },
    /// Exact byte identities are unioned; scalar distinct counts are never summed.
    ExactDistinct { identity_fields: Vec<String> },
    /// Select one dated value. Time is deliberately non-additive.
    SnapshotBalance {
        time_field: String,
        tie_break_fields: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroWeight {
    Null,
    Zero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopulationRelation {
    ProvenDisjoint,
    MayOverlap,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    AddComponents,
    UnionIdentities,
    SelectLatest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MetricStateError {
    #[error("unsupported metric state contract version")]
    Version,
    #[error("metric state fields must be nonempty and unique")]
    Identity,
    #[error("requested dimensions exceed the authored merge contract")]
    Dimension,
    #[error("additive state merge requires proved disjoint populations")]
    Overlap,
    #[error("snapshot balances cannot be summed across time")]
    TimeAdditivity,
    #[error("metric state arithmetic overflow")]
    Overflow,
    #[error("equal snapshot order keys have conflicting values")]
    ConflictingSnapshot,
}

impl MetricStateContract {
    pub fn validate(&self) -> Result<(), MetricStateError> {
        if self.version != METRIC_STATE_VERSION {
            return Err(MetricStateError::Version);
        }
        if self
            .merge_dimensions
            .iter()
            .any(|name| name.trim().is_empty())
        {
            return Err(MetricStateError::Identity);
        }
        match &self.state {
            MetricStateKind::WeightedAverage { weight_field, .. }
                if weight_field.trim().is_empty() =>
            {
                return Err(MetricStateError::Identity);
            }
            MetricStateKind::ExactDistinct { identity_fields } => validate_fields(identity_fields)?,
            MetricStateKind::SnapshotBalance {
                time_field,
                tie_break_fields,
            } => {
                if time_field.trim().is_empty() || self.merge_dimensions.contains(time_field) {
                    return Err(MetricStateError::TimeAdditivity);
                }
                validate_fields(tie_break_fields)?;
                if tie_break_fields.iter().any(|field| field == time_field) {
                    return Err(MetricStateError::Identity);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The returned mode determines which state operation is legal. It is not
    /// permission to merge already finalized scalar results.
    pub fn check_merge(
        &self,
        dimensions: &BTreeSet<String>,
        populations: PopulationRelation,
        sum_across_time: bool,
    ) -> Result<MergeMode, MetricStateError> {
        self.validate()?;
        if !dimensions.is_subset(&self.merge_dimensions) {
            return Err(MetricStateError::Dimension);
        }
        match &self.state {
            MetricStateKind::SumCountAverage | MetricStateKind::WeightedAverage { .. } => {
                if populations != PopulationRelation::ProvenDisjoint {
                    return Err(MetricStateError::Overlap);
                }
                Ok(MergeMode::AddComponents)
            }
            MetricStateKind::ExactDistinct { .. } => Ok(MergeMode::UnionIdentities),
            MetricStateKind::SnapshotBalance { .. } => {
                if sum_across_time {
                    return Err(MetricStateError::TimeAdditivity);
                }
                Ok(MergeMode::SelectLatest)
            }
        }
    }
}
fn validate_fields(fields: &[String]) -> Result<(), MetricStateError> {
    let mut seen = BTreeSet::new();
    if fields.is_empty()
        || fields
            .iter()
            .any(|field| field.trim().is_empty() || !seen.insert(field))
    {
        return Err(MetricStateError::Identity);
    }
    Ok(())
}

/// Coefficient at scale 18. No binary floating-point intermediate is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactMean(pub i128);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SumCountState {
    pub sum: i128,
    pub count: u64,
}
impl SumCountState {
    pub fn merge(self, other: Self) -> Result<Self, MetricStateError> {
        Ok(Self {
            sum: self
                .sum
                .checked_add(other.sum)
                .ok_or(MetricStateError::Overflow)?,
            count: self
                .count
                .checked_add(other.count)
                .ok_or(MetricStateError::Overflow)?,
        })
    }
    pub fn finalize(self) -> Result<Option<ExactMean>, MetricStateError> {
        if self.count == 0 {
            return Ok(None);
        }
        let scaled = self
            .sum
            .checked_mul(10_i128.pow(METRIC_DECIMAL_SCALE))
            .ok_or(MetricStateError::Overflow)?;
        Ok(Some(ExactMean(scaled / i128::from(self.count))))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightedState {
    pub weighted_sum: i128,
    pub weight_sum: i128,
}
impl WeightedState {
    pub fn merge(self, other: Self) -> Result<Self, MetricStateError> {
        Ok(Self {
            weighted_sum: self
                .weighted_sum
                .checked_add(other.weighted_sum)
                .ok_or(MetricStateError::Overflow)?,
            weight_sum: self
                .weight_sum
                .checked_add(other.weight_sum)
                .ok_or(MetricStateError::Overflow)?,
        })
    }
    pub fn finalize(self, zero: ZeroWeight) -> Result<Option<ExactMean>, MetricStateError> {
        if self.weight_sum == 0 {
            return Ok((zero == ZeroWeight::Zero).then_some(ExactMean(0)));
        }
        let scaled = self
            .weighted_sum
            .checked_mul(10_i128.pow(METRIC_DECIMAL_SCALE))
            .ok_or(MetricStateError::Overflow)?;
        Ok(Some(ExactMean(scaled / self.weight_sum)))
    }
}

/// Exact canonical identity bytes; callers must use one stable field encoding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExactDistinctState(pub BTreeSet<Vec<u8>>);
impl ExactDistinctState {
    pub fn merge(mut self, other: Self) -> Self {
        self.0.extend(other.0);
        self
    }
    pub fn finalize_count(&self) -> Result<u64, MetricStateError> {
        u64::try_from(self.0.len()).map_err(|_| MetricStateError::Overflow)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotBalanceState {
    pub observed_at: i64,
    pub tie_break: Vec<String>,
    pub value: Option<i128>,
}
impl SnapshotBalanceState {
    pub fn select_latest(self, other: Self) -> Result<Self, MetricStateError> {
        let left = (self.observed_at, &self.tie_break);
        let right = (other.observed_at, &other.tie_break);
        match left.cmp(&right) {
            std::cmp::Ordering::Greater => Ok(self),
            std::cmp::Ordering::Less => Ok(other),
            std::cmp::Ordering::Equal if self.value == other.value => Ok(self),
            std::cmp::Ordering::Equal => Err(MetricStateError::ConflictingSnapshot),
        }
    }
}
