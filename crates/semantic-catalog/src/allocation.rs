//! Exact, authored allocation over a checked bridge population. The runtime
//! observations must be read with the source row under one query boundary.
use crate::{CatalogSnapshot, DataType, ObjectRef, SourceRef, canonical_digest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

pub const ALLOCATION_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationContract {
    pub version: u32,
    pub id: String,
    pub source_relation: String,
    pub bridge_relation: String,
    pub source_entity_fields: Vec<String>,
    pub bridge_source_fields: Vec<String>,
    pub target_dimensions: Vec<String>,
    pub source_amount_field: String,
    pub amount_unit: String,
    pub weight_field: String,
    pub eligible_population: AllocationEligibility,
    pub expected_membership_count_field: String,
    pub expected_weight_total_field: String,
    pub denominator: AllocationDenominator,
    pub null_weight: NullWeight,
    pub zero_denominator: ZeroDenominator,
    pub rounding: AllocationRounding,
    pub conservation: AllocationConservation,
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AllocationEligibility {
    AllRows,
    Utf8Equals { field: String, value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationDenominator {
    SumEligibleWeightsPerSource,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullWeight {
    Reject,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroDenominator {
    Reject,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationRounding {
    LargestRemainderMinorUnit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationConservation {
    ExactPerSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationSourceObservation {
    pub amount: i128,
    pub expected_membership_count: u64,
    pub expected_weight_total: i128,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationBridgeObservation {
    pub target: Vec<String>,
    pub weight: Option<i128>,
    pub eligibility_value: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatedAmount {
    pub target: Vec<String>,
    pub amount: i128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AllocationError {
    #[error("unsupported allocation contract version")]
    Version,
    #[error("allocation identity or field list is invalid")]
    Identity,
    #[error("allocation relation or field is missing")]
    Field,
    #[error("allocation field has an unsupported physical type")]
    Type,
    #[error("allocation scope excludes a required relation")]
    Scope,
    #[error("eligible bridge target is missing or duplicated")]
    Target,
    #[error("eligible bridge weight is null or negative")]
    Weight,
    #[error("bridge population differs from the source-declared complete population")]
    IncompletePopulation,
    #[error("eligible weight denominator is zero")]
    ZeroDenominator,
    #[error("exact allocation arithmetic overflow")]
    Overflow,
    #[error("allocated amounts do not conserve the source amount")]
    Conservation,
}

impl AllocationContract {
    pub fn reference(&self) -> ObjectRef {
        let mut canonical = self.clone();
        canonical.source_refs.clear();
        ObjectRef {
            id: self.id.clone(),
            revision: canonical_digest(
                &serde_json::to_value(canonical).expect("allocation serializes"),
            ),
        }
    }

    pub fn validate(
        &self,
        snapshot: &CatalogSnapshot,
        allowed_relations: Option<&BTreeSet<String>>,
    ) -> Result<(), AllocationError> {
        if self.version != ALLOCATION_VERSION {
            return Err(AllocationError::Version);
        }
        if self.id.trim().is_empty()
            || self.source_relation.trim().is_empty()
            || self.bridge_relation.trim().is_empty()
            || self.source_entity_fields.len() != self.bridge_source_fields.len()
            || !valid_fields(&self.source_entity_fields)
            || !valid_fields(&self.bridge_source_fields)
            || !valid_fields(&self.target_dimensions)
            || self.source_amount_field.trim().is_empty()
            || self.amount_unit.trim().is_empty()
            || self.weight_field.trim().is_empty()
            || self.expected_membership_count_field.trim().is_empty()
            || self.expected_weight_total_field.trim().is_empty()
        {
            return Err(AllocationError::Identity);
        }
        if allowed_relations.is_some_and(|scope| {
            !scope.contains(&self.source_relation) || !scope.contains(&self.bridge_relation)
        }) {
            return Err(AllocationError::Scope);
        }
        let source = snapshot
            .relation(&self.source_relation)
            .ok_or(AllocationError::Field)?;
        let bridge = snapshot
            .relation(&self.bridge_relation)
            .ok_or(AllocationError::Field)?;
        let source_semantics = source
            .definition()
            .semantics
            .as_ref()
            .ok_or(AllocationError::Identity)?;
        if source_semantics.declared_primary_key != self.source_entity_fields
            && !source_semantics
                .declared_unique_keys
                .contains(&self.source_entity_fields)
        {
            return Err(AllocationError::Identity);
        }
        for (left, right) in self
            .source_entity_fields
            .iter()
            .zip(&self.bridge_source_fields)
        {
            let left = source.field(left).ok_or(AllocationError::Field)?;
            let right = bridge.field(right).ok_or(AllocationError::Field)?;
            if left.is_nullable()
                || right.is_nullable()
                || left.data_type() != right.data_type()
                || !exact_key_type(left.data_type())
            {
                return Err(AllocationError::Type);
            }
        }
        for field in &self.target_dimensions {
            let field = bridge.field(field).ok_or(AllocationError::Field)?;
            if field.is_nullable() || !exact_key_type(field.data_type()) {
                return Err(AllocationError::Type);
            }
        }
        for (relation, field) in [
            (source, &self.source_amount_field),
            (source, &self.expected_membership_count_field),
            (source, &self.expected_weight_total_field),
        ] {
            let field = relation.field(field).ok_or(AllocationError::Field)?;
            if field.is_nullable() || field.data_type() != &DataType::Int64 {
                return Err(AllocationError::Type);
            }
        }
        if bridge
            .field(&self.weight_field)
            .ok_or(AllocationError::Field)?
            .data_type()
            != &DataType::Int64
        {
            return Err(AllocationError::Type);
        }
        if let AllocationEligibility::Utf8Equals { field, value } = &self.eligible_population {
            if field.trim().is_empty() || value.is_empty() {
                return Err(AllocationError::Identity);
            }
            if bridge
                .field(field)
                .ok_or(AllocationError::Field)?
                .data_type()
                != &DataType::Utf8
            {
                return Err(AllocationError::Type);
            }
        }
        Ok(())
    }

    /// Checks one source entity's policy-visible rows. The caller must obtain
    /// the source expectations and bridge observations in the same execution.
    pub fn verify_and_allocate(
        &self,
        source: &AllocationSourceObservation,
        bridge: &[AllocationBridgeObservation],
    ) -> Result<Vec<AllocatedAmount>, AllocationError> {
        let mut seen = BTreeSet::new();
        let mut eligible = Vec::new();
        let mut total_weight = 0i128;
        for row in bridge {
            let included = match &self.eligible_population {
                AllocationEligibility::AllRows => true,
                AllocationEligibility::Utf8Equals { value, .. } => {
                    row.eligibility_value.as_ref() == Some(value)
                }
            };
            if !included {
                continue;
            }
            if row.target.len() != self.target_dimensions.len()
                || row.target.iter().any(|part| part.is_empty())
                || !seen.insert(row.target.clone())
            {
                return Err(AllocationError::Target);
            }
            let weight = row.weight.ok_or(AllocationError::Weight)?;
            if weight < 0 {
                return Err(AllocationError::Weight);
            }
            total_weight = total_weight
                .checked_add(weight)
                .ok_or(AllocationError::Overflow)?;
            eligible.push((row.target.clone(), weight as u128));
        }
        if eligible.len() as u64 != source.expected_membership_count
            || total_weight != source.expected_weight_total
            || eligible.is_empty()
        {
            return Err(AllocationError::IncompletePopulation);
        }
        if total_weight == 0 {
            return Err(AllocationError::ZeroDenominator);
        }
        let amount = source.amount.unsigned_abs();
        let denominator = total_weight as u128;
        let mut shares = Vec::with_capacity(eligible.len());
        let mut floor_sum = 0u128;
        for (target, weight) in eligible {
            let numerator = amount
                .checked_mul(weight)
                .ok_or(AllocationError::Overflow)?;
            let floor = numerator / denominator;
            floor_sum = floor_sum
                .checked_add(floor)
                .ok_or(AllocationError::Overflow)?;
            shares.push((target, floor, numerator % denominator));
        }
        let residual = amount
            .checked_sub(floor_sum)
            .ok_or(AllocationError::Conservation)?;
        shares.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        for share in shares
            .iter_mut()
            .take(usize::try_from(residual).map_err(|_| AllocationError::Overflow)?)
        {
            share.1 = share.1.checked_add(1).ok_or(AllocationError::Overflow)?;
        }
        shares.sort_by(|a, b| a.0.cmp(&b.0));
        let result = shares
            .into_iter()
            .map(|(target, magnitude, _)| {
                let signed = if source.amount < 0 {
                    if magnitude == (i128::MAX as u128) + 1 {
                        i128::MIN
                    } else {
                        -i128::try_from(magnitude).map_err(|_| AllocationError::Overflow)?
                    }
                } else {
                    i128::try_from(magnitude).map_err(|_| AllocationError::Overflow)?
                };
                Ok(AllocatedAmount {
                    target,
                    amount: signed,
                })
            })
            .collect::<Result<Vec<_>, AllocationError>>()?;
        let total = result.iter().try_fold(0i128, |sum, share| {
            sum.checked_add(share.amount)
                .ok_or(AllocationError::Overflow)
        })?;
        if total != source.amount {
            return Err(AllocationError::Conservation);
        }
        Ok(result)
    }
}

fn valid_fields(fields: &[String]) -> bool {
    let mut seen = BTreeSet::new();
    !fields.is_empty()
        && fields
            .iter()
            .all(|field| !field.trim().is_empty() && seen.insert(field))
}
fn exact_key_type(data_type: &DataType) -> bool {
    matches!(data_type, DataType::Int64 | DataType::Utf8)
}
