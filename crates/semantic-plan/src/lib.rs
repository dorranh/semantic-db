//! Initial interchange contracts for a future semantic compiler.
//!
//! Intent contains meaning, not guessed physical column names. Grounded SQL is
//! still untrusted input: the engine must plan and validate it before execution.
//! No LLM, retrieval backend, or automatic grounding is implemented here yet.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticPlan {
    pub request: String,
    /// Requested entity or grain, before binding to a physical relation.
    pub target: String,
    pub predicate: Option<SemanticPredicate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SemanticPredicate {
    Concept { phrase: String },
    All { predicates: Vec<SemanticPredicate> },
    Any { predicates: Vec<SemanticPredicate> },
    Not { predicate: Box<SemanticPredicate> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundingEvidence {
    pub phrase: String,
    pub catalog_reference: String,
    pub interpretation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundedQuery {
    pub sql: String,
    pub evidence: Vec<GroundingEvidence>,
}

/// Ambiguity is explicit; unresolved concepts must not silently become SQL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GroundingOutcome {
    Grounded {
        query: GroundedQuery,
    },
    NeedsClarification {
        phrases: Vec<String>,
        question: String,
    },
    Unsupported {
        reason: String,
    },
}
