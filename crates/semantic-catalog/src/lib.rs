//! Descriptive metadata alongside the engine's executable DataFusion catalog.
//!
//! Arrow schemas are the row-type contract. This catalog does not execute SQL.

use std::collections::BTreeMap;

pub use arrow_schema::{DataType, Field, Schema, SchemaRef};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod governance;
pub use governance::*;
mod semantic_type;
pub use semantic_type::*;
mod entity_key;
pub use entity_key::*;
mod applicability;
pub use applicability::*;
mod conversion;
pub use conversion::*;
mod business_calendar;
pub use business_calendar::*;
mod currency_rate;
mod exact_decimal_rate;
pub use currency_rate::*;
pub use exact_decimal_rate::{DecimalRounding, ExactDecimalRateRule, NullRatePolicy};
mod allocation;
pub use allocation::*;
mod metric_state;
pub use metric_state::*;
mod relationship_path;
pub use relationship_path::*;
mod view_lineage;
pub use view_lineage::*;
mod source;
pub use source::*;
mod canonical;
mod publication;
pub use canonical::{canonical_digest, canonical_json};
mod search;
pub use publication::{
    CatalogMutation, CatalogStore, PublicationError, PublicationLimits, PublicationReport,
};
mod snapshot;
mod tree;
pub use search::{SearchHit, SearchIndex, SearchObject, SearchOptions, SearchReport};
pub use snapshot::{CatalogSnapshot, ObjectRef, SnapshotRelation};

/// Authored hints for grounding. These are evidence, not executable rules or
/// instructions that may override the compiler's behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiContext {
    pub instructions: Option<String>,
    #[serde(default)]
    pub synonyms: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FieldSemantics {
    pub source_refs: Vec<SourceRef>,
    pub description: Option<String>,
    pub logical_type: Option<String>,
    pub unit: Option<Unit>,
    pub comparison_profile: Option<ComparisonProfile>,
    pub enum_domain: Option<EnumDomain>,
    pub reference_system: Option<ReferenceSystem>,
    pub calendar_reference: Option<CalendarReference>,
    pub label: Option<String>,
    pub is_time: Option<bool>,
    pub ai_context: Option<AiContext>,
}

/// Import identity without connection details or physical source paths.
#[derive(Debug, Clone, Serialize)]
pub struct SemanticOrigin {
    pub format: String,
    pub version: String,
    pub schema_revision: String,
    pub schema_sha256: String,
    pub document_sha256: String,
    pub adapter_version: String,
    pub model: String,
    pub dataset: String,
}

/// Descriptive semantics kept separate from Arrow's physical schema contract.
/// Keys are declarations only: registration does not scan rows to enforce them.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RelationSemantics {
    pub view_coverage: Option<ViewTemporalCoverage>,
    pub view_lineage: Option<ViewOutputLineage>,
    pub conversions: BTreeMap<String, UnitConversion>,
    pub business_calendars: BTreeMap<String, BusinessCalendarRule>,
    pub currency_rates: BTreeMap<String, CurrencyRateRule>,
    #[serde(default)]
    pub exact_decimal_rates: BTreeMap<String, ExactDecimalRateRule>,
    pub allocations: BTreeMap<String, AllocationContract>,
    pub value_mappings: BTreeMap<String, ValueMapping>,
    pub concepts: BTreeMap<String, ConceptDefinition>,
    pub metrics: BTreeMap<String, MetricDefinition>,
    pub ratio_metrics: BTreeMap<String, RatioDefinition>,
    pub relationships: BTreeMap<String, RelationshipDefinition>,
    pub row_policies: Vec<RowPolicy>,
    pub source_refs: Vec<SourceRef>,
    /// None retains the legacy application-registered executable contract.
    pub capability: Option<Capability>,
    pub facts: BTreeMap<String, FactResolution<serde_json::Value>>,
    pub model_description: Option<String>,
    pub model_ai_context: Option<AiContext>,
    pub ai_context: Option<AiContext>,
    pub fields: BTreeMap<String, FieldSemantics>,
    pub declared_primary_key: Vec<String>,
    pub declared_unique_keys: Vec<Vec<String>>,
    /// An authored entity/key contract with explicit, unpromoted evidence.
    pub entity_identity: Option<EntityIdentity>,
    pub origin: Option<SemanticOrigin>,
}

/// How a relation is defined; materialization is a separate, future policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationKind {
    Base {
        /// Backend-owned identifier, such as `warehouse:orders`. Keep credentials
        /// in the backend, not in descriptive metadata.
        source: String,
    },
    View {
        sql: String,
        dependencies: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Relation {
    pub name: String,
    pub schema: SchemaRef,
    pub kind: RelationKind,
    pub description: Option<String>,
    pub owner: Option<String>,
    /// What a single output row represents; not a uniqueness constraint.
    pub grain: Option<String>,
    pub semantics: Option<RelationSemantics>,
}

impl Relation {
    pub fn base(name: impl Into<String>, schema: SchemaRef, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema,
            kind: RelationKind::Base {
                source: source.into(),
            },
            description: None,
            owner: None,
            grain: None,
            semantics: None,
        }
    }

    /// Declare a view's output contract. The engine derives dependencies from SQL
    /// and validates this schema against the planned output when loading it.
    pub fn view(name: impl Into<String>, schema: SchemaRef, sql: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema,
            kind: RelationKind::View {
                sql: sql.into(),
                dependencies: Vec::new(),
            },
            description: None,
            owner: None,
            grain: None,
            semantics: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }

    pub fn with_grain(mut self, grain: impl Into<String>) -> Self {
        self.grain = Some(grain.into());
        self
    }
}

/// A curated definition available to a future grounding implementation.
#[derive(Debug, Clone)]
pub struct Concept {
    pub name: String,
    pub description: String,
    pub definition: String,
    /// Stable evidence reference (for example, a versioned catalog URI).
    pub evidence: String,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("relation already exists: {0}")]
    DuplicateRelation(String),
}

#[derive(Debug, Default, Clone)]
pub struct Catalog {
    root: tree::Root,
    analysis: std::sync::OnceLock<std::sync::Arc<publication::DependencyAnalysis>>,
    snapshot: std::sync::OnceLock<std::sync::Arc<CatalogSnapshot>>,
}

impl Catalog {
    /// Build a session snapshot from definitions owned by any application catalog.
    pub fn from_relations(
        relations: impl IntoIterator<Item = Relation>,
    ) -> Result<Self, CatalogError> {
        let mut catalog = Self::default();
        for relation in relations {
            catalog.register(relation)?;
        }
        Ok(catalog)
    }

    pub fn relation(&self, name: &str) -> Option<&Relation> {
        self.root.get(name).map(|entry| entry.definition())
    }

    pub fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.root.iter().map(|entry| entry.definition())
    }

    /// Publish immutable metadata once per catalog generation. Later readers
    /// share the same handle; registration leaves already pinned snapshots intact.
    pub fn snapshot(&self) -> std::sync::Arc<CatalogSnapshot> {
        self.snapshot
            .get_or_init(|| std::sync::Arc::new(CatalogSnapshot::new(self.root.clone())))
            .clone()
    }

    /// Reject replacement so callers cannot silently invalidate view lineage.
    pub fn register(&mut self, relation: Relation) -> Result<(), CatalogError> {
        if self.relation(&relation.name).is_some() {
            return Err(CatalogError::DuplicateRelation(relation.name));
        }
        self.root = self.root.insert(
            relation.name.clone(),
            std::sync::Arc::new(SnapshotRelation::new(relation)),
        );
        self.snapshot.take();
        self.analysis.take();
        Ok(())
    }
}

impl IntoIterator for Catalog {
    type Item = Relation;
    type IntoIter = std::vec::IntoIter<Relation>;

    fn into_iter(self) -> Self::IntoIter {
        self.relations().cloned().collect::<Vec<_>>().into_iter()
    }
}
