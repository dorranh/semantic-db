//! Executable authored contracts. Prose never populates these definitions implicitly.
use crate::{DataType, ObjectRef, Presence, SourceRef};
use semantic_plan::typed::{AggregateFunction, Comparison, Literal};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// An authored, exact phrase-to-code dictionary. Keys are case-sensitive and
/// neither Unicode-normalized nor inferred from current data. Multiple authored
/// phrases may name the same code. This initial profile targets Utf8 fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueMapping {
    pub id: String,
    pub field: String,
    pub description: String,
    pub codes: std::collections::BTreeMap<String, String>,
    pub source_refs: Vec<SourceRef>,
}
impl ValueMapping {
    pub fn reference(&self) -> ObjectRef {
        ObjectRef {
            id: self.id.clone(),
            revision: crate::canonical_digest(
                &serde_json::to_value(self).expect("mapping serializes"),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedFilter {
    pub field: String,
    pub operator: Comparison,
    pub value: Literal,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricDefinition {
    /// Durable identity. Map keys are display/lookup names, not identity.
    pub id: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub function: AggregateFunction,
    pub field: Option<String>,
    pub distinct: bool,
    pub source_grain: Vec<String>,
    /// An explicit whitelist. Empty means only the global aggregate is permitted.
    pub compatible_dimensions: BTreeSet<String>,
    #[serde(default)]
    pub compatible_lookup_dimensions: Vec<MetricLookupDimension>,
    /// Dimensions across which disjoint grouped scalar results may be summed.
    /// Missing means no authored additive rollup contract. Only nondistinct
    /// SUM/COUNT have a supported scalar-sum merge; distinct state needs a
    /// separate representation even if an author supplies this annotation.
    #[serde(default)]
    pub sum_rollup_dimensions: Option<BTreeSet<String>>,
    /// Applied inside this aggregate, never to other metrics in the same query.
    pub row_filters: Vec<GovernedFilter>,
    pub result_type: DataType,
    pub unit: Presence<String>,
    /// This profile returns zero for empty COUNT and null for empty SUM/MIN/MAX.
    pub empty_behavior: EmptyBehavior,
    pub source_refs: Vec<SourceRef>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmptyBehavior {
    Zero,
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricLookupDimension {
    pub relationship: String,
    pub field: String,
    pub missing: semantic_plan::typed::MissingMatch,
}
impl MetricDefinition {
    pub fn reference(&self) -> ObjectRef {
        ObjectRef {
            id: self.id.clone(),
            revision: crate::canonical_digest(
                &serde_json::to_value(self).expect("metric definition serializes"),
            ),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowPolicy {
    pub id: String,
    pub filters: Vec<GovernedFilter>,
    pub source_refs: Vec<SourceRef>,
}
impl RowPolicy {
    pub fn reference(&self) -> ObjectRef {
        ObjectRef {
            id: self.id.clone(),
            revision: crate::canonical_digest(
                &serde_json::to_value(self).expect("policy serializes"),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipKey {
    pub left_field: String,
    pub right_field: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    OneToOne,
    ManyToOne,
    OneToMany,
    ManyToMany,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipDefinition {
    #[serde(default)]
    pub ai_context: Option<crate::AiContext>,
    pub id: String,
    pub right_relation: String,
    pub role: String,
    pub key_pairs: Vec<RelationshipKey>,
    pub null_keys_match: bool,
    /// Existence/absence uses semi/anti joins and needs no multiplicity assumption.
    /// Ordinary joins must separately establish enforceable directional evidence.
    pub cardinality: crate::FactResolution<Cardinality>,
    pub source_refs: Vec<SourceRef>,
}
impl RelationshipDefinition {
    pub fn reference(&self) -> ObjectRef {
        ObjectRef {
            id: self.id.clone(),
            revision: crate::canonical_digest(
                &serde_json::to_value(self).expect("relationship serializes"),
            ),
        }
    }
}

/// A governed ratio expands its two declared aggregate metrics before division.
/// This executable profile accepts Int64 components, Decimal128(38,18) output,
/// truncation toward zero, and explicit zero behavior. Nulls always propagate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RatioDefinition {
    pub id: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub numerator: String,
    pub denominator: String,
    pub zero: semantic_plan::typed::ZeroDivision,
    pub unit: Presence<String>,
    pub source_refs: Vec<SourceRef>,
}
impl RatioDefinition {
    pub fn reference(&self) -> ObjectRef {
        ObjectRef {
            id: self.id.clone(),
            revision: crate::canonical_digest(
                &serde_json::to_value(self).expect("ratio definition serializes"),
            ),
        }
    }
}
