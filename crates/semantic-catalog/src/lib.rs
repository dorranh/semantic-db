//! Descriptive metadata alongside the engine's executable DataFusion catalog.
//!
//! Arrow schemas are the row-type contract. This catalog does not execute SQL.

use std::collections::{BTreeMap, btree_map::Entry};

pub use arrow_schema::SchemaRef;
use thiserror::Error;

/// How a relation is defined; materialization is a separate, future policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationKind {
    Base {
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

#[derive(Debug, Default)]
pub struct Catalog {
    relations: BTreeMap<String, Relation>,
}

impl Catalog {
    pub fn relation(&self, name: &str) -> Option<&Relation> {
        self.relations.get(name)
    }

    pub fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.relations.values()
    }

    /// Reject replacement so callers cannot silently invalidate view lineage.
    pub fn register(&mut self, relation: Relation) -> Result<(), CatalogError> {
        match self.relations.entry(relation.name.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(relation);
                Ok(())
            }
            Entry::Occupied(entry) => Err(CatalogError::DuplicateRelation(entry.key().clone())),
        }
    }
}
