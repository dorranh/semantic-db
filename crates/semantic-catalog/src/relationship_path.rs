//! Authored relationship paths with distinct relation occurrences. A path is
//! selected by exact roles and checked edges, never by shortest-path ranking.
use crate::{Cardinality, CatalogSnapshot, DataType, FactResolution, ObjectRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

pub const RELATIONSHIP_PATH_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipPath {
    pub version: u32,
    pub start_relation: String,
    pub start_occurrence: String,
    pub usage: PathUsage,
    pub steps: Vec<PathStep>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathUsage {
    LookupUnique,
    Existence,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathStep {
    pub from_occurrence: String,
    pub to_occurrence: String,
    pub right_relation: String,
    pub relationship: String,
    pub role: String,
    pub as_of: Option<AsOfJoin>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsOfJoin {
    /// A field on the left occurrence, compared to [valid_from, valid_to).
    pub fact_time: String,
    pub valid_from: String,
    pub valid_to: String,
    pub timezone: Option<String>,
    pub missing: PathMissing,
    /// An as-of lookup must check at most one matching interval in the same
    /// query execution. A stale catalog scan cannot discharge this obligation.
    pub same_query_uniqueness: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathMissing {
    Null,
    Exclude,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPath {
    pub definitions: Vec<ObjectRef>,
    pub required_relations: BTreeSet<String>,
    pub obligations: Vec<PathObligation>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathObligation {
    pub relationship: ObjectRef,
    pub right_relation: ObjectRef,
    pub kind: PathObligationKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathObligationKind {
    UniqueRightKeys,
    UniqueAsOfInterval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RelationshipPathError {
    #[error("unsupported relationship path version")]
    Version,
    #[error("relationship path exceeds its hop budget")]
    Limit,
    #[error("relationship path occurrence identity or chain is invalid")]
    Identity,
    #[error("relationship path leaves its authorized relation scope")]
    Scope,
    #[error("relationship path relation is missing")]
    Relation,
    #[error("authored relationship is missing")]
    Relationship,
    #[error("relationship role or endpoint differs from its authored definition")]
    Role,
    #[error("relationship key types or cardinality are unsupported")]
    KeyType,
    #[error("relationship key reference systems differ or are only partially authored")]
    KeyReferenceSystem,
    #[error("lookup path lacks a same-query uniqueness obligation")]
    Uniqueness,
    #[error("as-of fields require matching exact temporal types")]
    TimeType,
    #[error("as-of timezone is unsupported")]
    Timezone,
    #[error("as-of interval fields or endpoint rule are invalid")]
    Interval,
}

impl RelationshipPath {
    pub fn validate(
        &self,
        snapshot: &CatalogSnapshot,
        allowed_relations: Option<&BTreeSet<String>>,
        max_hops: usize,
    ) -> Result<ValidatedPath, RelationshipPathError> {
        if self.version != RELATIONSHIP_PATH_VERSION {
            return Err(RelationshipPathError::Version);
        }
        if self.steps.is_empty() || self.steps.len() > max_hops {
            return Err(RelationshipPathError::Limit);
        }
        if self.start_relation.trim().is_empty() || self.start_occurrence.trim().is_empty() {
            return Err(RelationshipPathError::Identity);
        }
        let mut occurrence = self.start_occurrence.as_str();
        let mut relation = self.start_relation.as_str();
        let mut occurrences = BTreeSet::from([occurrence]);
        let mut required = BTreeSet::from([relation.to_owned()]);
        let mut definitions = Vec::new();
        let mut obligations = Vec::new();
        for step in &self.steps {
            if step.from_occurrence != occurrence
                || step.to_occurrence.trim().is_empty()
                || !occurrences.insert(step.to_occurrence.as_str())
                || step.relationship.trim().is_empty()
                || step.role.trim().is_empty()
            {
                return Err(RelationshipPathError::Identity);
            }
            if allowed_relations.is_some_and(|scope| {
                !scope.contains(relation) || !scope.contains(&step.right_relation)
            }) {
                return Err(RelationshipPathError::Scope);
            }
            let left = snapshot
                .relation(relation)
                .ok_or(RelationshipPathError::Relation)?;
            let right = snapshot
                .relation(&step.right_relation)
                .ok_or(RelationshipPathError::Relation)?;
            let relationship = left
                .definition()
                .semantics
                .as_ref()
                .and_then(|semantics| semantics.relationships.get(&step.relationship))
                .ok_or(RelationshipPathError::Relationship)?;
            if relationship.id.trim().is_empty()
                || relationship.role != step.role
                || relationship.right_relation != step.right_relation
            {
                return Err(RelationshipPathError::Role);
            }
            if relationship.key_pairs.is_empty() {
                return Err(RelationshipPathError::KeyType);
            }
            for key in &relationship.key_pairs {
                let left_type = left
                    .field(&key.left_field)
                    .ok_or(RelationshipPathError::KeyType)?
                    .data_type();
                let right_type = right
                    .field(&key.right_field)
                    .ok_or(RelationshipPathError::KeyType)?
                    .data_type();
                if left_type != right_type || !exact_key_type(left_type) {
                    return Err(RelationshipPathError::KeyType);
                }
                let left_system = left
                    .definition()
                    .semantics
                    .as_ref()
                    .and_then(|semantics| semantics.fields.get(&key.left_field))
                    .and_then(|field| field.reference_system.as_ref());
                let right_system = right
                    .definition()
                    .semantics
                    .as_ref()
                    .and_then(|semantics| semantics.fields.get(&key.right_field))
                    .and_then(|field| field.reference_system.as_ref());
                if !crate::reference_systems_compatible(left_system, right_system) {
                    return Err(RelationshipPathError::KeyReferenceSystem);
                }
            }
            let reference = left
                .definition_reference("relationship", &step.relationship)
                .ok_or(RelationshipPathError::Relationship)?
                .clone();
            if self.usage == PathUsage::LookupUnique {
                if matches!(
                    &relationship.cardinality,
                    FactResolution::Known {
                        value: Cardinality::OneToMany | Cardinality::ManyToMany,
                        ..
                    }
                ) {
                    return Err(RelationshipPathError::Uniqueness);
                }
                obligations.push(PathObligation {
                    relationship: reference.clone(),
                    right_relation: right.reference().clone(),
                    kind: PathObligationKind::UniqueRightKeys,
                });
            }
            if let Some(as_of) = &step.as_of {
                as_of.validate(left, right, self.usage)?;
                if as_of.same_query_uniqueness {
                    obligations.push(PathObligation {
                        relationship: reference.clone(),
                        right_relation: right.reference().clone(),
                        kind: PathObligationKind::UniqueAsOfInterval,
                    });
                }
            }
            definitions.push(reference);
            required.insert(step.right_relation.clone());
            occurrence = &step.to_occurrence;
            relation = &step.right_relation;
        }
        Ok(ValidatedPath {
            definitions,
            required_relations: required,
            obligations,
        })
    }
}

impl AsOfJoin {
    fn validate(
        &self,
        left: &crate::SnapshotRelation,
        right: &crate::SnapshotRelation,
        usage: PathUsage,
    ) -> Result<(), RelationshipPathError> {
        if self.fact_time.trim().is_empty()
            || self.valid_from.trim().is_empty()
            || self.valid_to.trim().is_empty()
            || self.valid_from == self.valid_to
        {
            return Err(RelationshipPathError::Interval);
        }
        if usage == PathUsage::LookupUnique && !self.same_query_uniqueness {
            return Err(RelationshipPathError::Uniqueness);
        }
        let fact = left
            .field(&self.fact_time)
            .ok_or(RelationshipPathError::TimeType)?
            .data_type();
        let start = right
            .field(&self.valid_from)
            .ok_or(RelationshipPathError::TimeType)?
            .data_type();
        let end = right
            .field(&self.valid_to)
            .ok_or(RelationshipPathError::TimeType)?
            .data_type();
        if fact != start || fact != end {
            return Err(RelationshipPathError::TimeType);
        }
        match fact {
            DataType::Date32 if self.timezone.is_none() => Ok(()),
            DataType::Timestamp(_, Some(zone))
                if zone.as_ref() == "UTC" && self.timezone.as_deref() == Some("UTC") =>
            {
                Ok(())
            }
            DataType::Timestamp(_, None) if self.timezone.is_none() => Ok(()),
            DataType::Timestamp(_, _) | DataType::Date32 => Err(RelationshipPathError::Timezone),
            _ => Err(RelationshipPathError::TimeType),
        }
    }
}
fn exact_key_type(ty: &DataType) -> bool {
    matches!(
        ty,
        DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt64
            | DataType::Utf8
            | DataType::Date32
            | DataType::Timestamp(_, _)
            | DataType::Decimal128(_, _)
    )
}
