//! Semantic meaning shared by authored catalog facts and typed proposals.

use serde::{Deserialize, Serialize};

/// Semantic units are independent of Arrow's physical numeric type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Unit {
    Dimensionless,
    Named {
        id: String,
    },
    Currency {
        code: String,
    },
    Quotient {
        numerator: Box<Unit>,
        denominator: Box<Unit>,
    },
}

/// Authored business identity. A physical field name is not an entity identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub String);

/// A field identity is scoped to its source relation, not to an output alias.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrainKey {
    pub relation: String,
    pub field: String,
}

/// The identity represented by a source row and its scoped key tuple.
/// This declaration alone does not prove uniqueness or a functional dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceGrain {
    pub entity: Option<EntityId>,
    pub keys: Vec<GrainKey>,
}
