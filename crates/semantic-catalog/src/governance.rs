//! Executable authored contracts. Prose never populates these definitions implicitly.
use crate::{DataType, ObjectRef, Presence, SourceRef, Unit};
use semantic_plan::typed::{AggregateFunction, CalendarUnit, Comparison, Literal, RowPredicate};
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
    pub enum_domain: Option<crate::EnumDomain>,
    pub source_refs: Vec<SourceRef>,
}

/// An executable relation-scoped business predicate. Its field names are
/// authored against this relation; binding supplies the query occurrence.
/// CompareParameter names infer their exact type from the authored field and
/// receive typed values from ConceptFilter arguments at compile time.
/// Prose, aliases and examples never become executable conditions implicitly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptDefinition {
    pub id: String,
    pub description: String,
    pub aliases: Vec<String>,
    /// Other same-relation concept keys that compete for this concept's
    /// name and aliases. This declares ambiguity, not logical equivalence.
    #[serde(default)]
    pub alternatives: Vec<String>,
    pub predicate: RowPredicate<String>,
    pub source_refs: Vec<SourceRef>,
}
impl ConceptDefinition {
    pub fn reference(&self) -> ObjectRef {
        ObjectRef {
            id: self.id.clone(),
            revision: crate::canonical_digest(
                &serde_json::to_value(self).expect("concept definition serializes"),
            ),
        }
    }
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
    pub source_grain: crate::SourceGrain,
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
    /// Versioned sufficient state. The executor must implement this exact
    /// merge/finalize contract before accepting the metric for execution.
    #[serde(default)]
    pub state: Option<crate::MetricStateContract>,
    /// Applied inside this aggregate, never to other metrics in the same query.
    pub row_filters: Vec<GovernedFilter>,
    pub result_type: DataType,
    pub unit: Presence<Unit>,
    /// Exact temporal applicability for this executable profile. `Null` means
    /// explicitly unrestricted, while `Missing` remains unknown.
    #[serde(default)]
    pub temporal: Presence<MetricTemporalApplicability>,
    /// This profile returns zero for empty COUNT and null for empty SUM/MIN/MAX.
    pub empty_behavior: EmptyBehavior,
    pub source_refs: Vec<SourceRef>,
}

/// A restricted metric must be queried through one matching calendar filter.
/// Coverage is a half-open interval and uses exact Date32 or UTC Timestamp
/// literals; no timezone/unit conversion or predicate implication is attempted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricTemporalApplicability {
    pub field: String,
    pub grain: CalendarUnit,
    pub coverage_start: Literal,
    pub coverage_end: Literal,
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
    pub unit: Presence<Unit>,
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
