//! Descriptive metadata alongside the engine's executable DataFusion catalog.
//!
//! Arrow schemas are the row-type contract. This catalog does not execute SQL.

use std::collections::{BTreeMap, btree_map::Entry};

pub use arrow_schema::{DataType, Field, Schema, SchemaRef};
use thiserror::Error;

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
    relations: BTreeMap<String, Relation>,
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

impl IntoIterator for Catalog {
    type Item = Relation;
    type IntoIter = std::collections::btree_map::IntoValues<String, Relation>;

    fn into_iter(self) -> Self::IntoIter {
        self.relations.into_values()
    }
}
