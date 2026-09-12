//! Interchange contracts for semantic interpretation and compilation.
//!
//! Intent contains meaning, not guessed physical column names. Grounded SQL is
//! still untrusted input: the engine must plan and validate it before execution.
//! Provider and compilation logic live in semantic-compiler.

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
#[serde(deny_unknown_fields)]
pub struct GroundingEvidence {
    pub phrase: String,
    pub catalog_reference: String,
    pub interpretation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundedQuery {
    pub sql: String,
    pub evidence: Vec<GroundingEvidence>,
}

/// Ambiguity is explicit; unresolved concepts must not silently become SQL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
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

/// A bounded query over one authored view. The compiler owns SQL lowering;
/// the model can neither supply SQL nor replace the view's defining predicates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewSelection {
    pub view: String,
    /// Exact phrase from the request that this view is proposed to resolve.
    pub phrase: String,
    /// Explicit output columns, in order. Empty and duplicate lists are invalid.
    pub columns: Vec<String>,
    /// Conjunction only. OR, joins, aggregates and arbitrary expressions are
    /// deliberately outside this first lowering contract.
    pub filters: Vec<ViewFilter>,
    pub order_by: Vec<ViewOrder>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewFilter {
    Compare {
        column: String,
        op: Comparison,
        value: RequestLiteral,
    },
    IsNull {
        column: String,
    },
    IsNotNull {
        column: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
}

/// Literal spelling copied from the request, with no inferred unit conversion.
/// Numbers use JSON number syntax, retaining their exact decimal spelling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "text",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RequestLiteral {
    Text(String),
    Number(String),
    Boolean(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewOrder {
    pub column: String,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewSelectionOutcome {
    Selected {
        selection: ViewSelection,
    },
    NeedsClarification {
        phrases: Vec<String>,
        question: String,
    },
    Unsupported {
        reason: String,
    },
}
