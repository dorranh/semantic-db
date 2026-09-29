//! Versioned, untrusted relational composition proposals. Each leaf re-enters
//! ordinary semantic binding; graph edges name output slots, never SQL aliases.
use crate::typed::{Direction, NullOrder, RowQuery};
use serde::{Deserialize, Serialize};

/// Evidence is separate from the legacy graph proposal: a structured graph can
/// still compile without claiming that its request spans have been validated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphIntentQuery {
    pub query: GraphQuery,
    pub evidence: GraphRequestEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphRequestEvidence {
    pub version: u32,
    pub request_id: String,
    pub original_request: String,
    pub requirements: Vec<GraphRequirementEvidence>,
    pub unresolved_alternatives: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphRequirementEvidence {
    pub target: GraphRequirementRef,
    pub source_spans: Vec<crate::typed::RequestSpan>,
}

/// A typed, scoped identity. Ordering positions refer to the immutable proposal,
/// not SQL aliases. Every node, leaf requirement, non-leaf output, final sort key
/// and final limit has its own disposition and (for intent) evidence entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GraphRequirementRef {
    Node { node: String },
    Leaf { node: String, requirement: String },
    Output { node: String, slot: String },
    Order { index: usize },
    Limit,
}
impl GraphRequirementRef {
    /// Collision-free record identity using JSON Pointer component escaping.
    pub fn record_id(&self) -> String {
        fn escape(value: &str) -> String {
            value.replace('~', "~0").replace('/', "~1")
        }
        match self {
            Self::Node { node } => format!("node/{}", escape(node)),
            Self::Leaf { node, requirement } => {
                format!("leaf/{}/{}", escape(node), escape(requirement))
            }
            Self::Output { node, slot } => format!("output/{}/{}", escape(node), escape(slot)),
            Self::Order { index } => format!("order/{index}"),
            Self::Limit => "limit".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphQuery {
    pub version: u32,
    pub nodes: Vec<QueryNode>,
    pub root: String,
    pub ordering: Vec<GraphOrder>,
    pub limit: Option<u32>,
    pub unresolved: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryNode {
    pub id: String,
    pub source_text: String,
    pub operation: GraphOperation,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GraphOperation {
    Rows {
        query: RowQuery,
    },
    Set {
        left: String,
        right: String,
        operator: SetOperator,
        duplicates: Duplicates,
        columns: Vec<SetColumn>,
    },
    Compose {
        left: String,
        right: String,
        relationship_relation: String,
        relationship: String,
        role: String,
        domain: GroupDomain,
        null_alignment: NullAlignment,
        keys: Vec<SetColumn>,
        outputs: Vec<CompositionOutput>,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetColumn {
    pub id: String,
    pub left: String,
    pub right: String,
    pub alias: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionOutput {
    pub id: String,
    pub side: Side,
    pub slot: String,
    pub alias: String,
    pub missing: MissingGroup,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphOrder {
    pub slot: String,
    pub direction: Direction,
    pub nulls: NullOrder,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetOperator {
    Union,
    Intersect,
    Except,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Duplicates {
    All,
    Distinct,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupDomain {
    Union,
    Intersection,
    Left,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullAlignment {
    Match,
    NeverMatch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingGroup {
    Null,
    Zero,
}
