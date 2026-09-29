//! Version 1 row-query proposals. These types are untrusted, including when
//! deserialized from retained artifacts. Only the compiler can bind them.
use serde::{Deserialize, Serialize};

pub const ROW_QUERY_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowQuery {
    pub version: u32,
    pub input: RelationInput,
    /// Every operation belongs to a request requirement. The compiler preserves
    /// all of them; this does not prove the interpreter extracted every phrase.
    pub requirements: Vec<Requirement>,
    pub unresolved: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationInput {
    pub relation: String,
    pub instance: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldRef {
    pub instance: String,
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub id: String,
    pub source_text: String,
    pub operation: RowOperation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RowOperation {
    /// Filter resolved output slots at an explicit relational stage.
    FilterOutput {
        stage: OutputFilterStage,
        predicate: OutputPredicate,
    },
    /// Select one dimension value through an authored role. Strict compilation
    /// adds a same-query uniqueness obligation; duplicate matches fail execution.
    Lookup {
        relationship: String,
        role: String,
        instance: String,
        field: String,
        alias: String,
        missing: MissingMatch,
        #[serde(default)]
        usage: LookupUsage,
    },
    /// A half-open interval of whole calendar periods, anchored by host context.
    CalendarFilter {
        field: FieldRef,
        period: CalendarPeriod,
    },
    /// Evaluated after row policies/filters and any explicit aggregation, before
    /// final ordering/fetch. Window outputs cannot be inputs of other windows.
    Window {
        window: WindowSpec,
        alias: String,
    },
    /// Aggregate each component before division. Integer components produce an
    /// exact decimal quantized to 18 places with truncation toward zero.
    Ratio {
        numerator: AggregateOperand,
        denominator: AggregateOperand,
        zero: ZeroDivision,
        alias: String,
    },
    Related {
        relationship: String,
        role: String,
        instance: String,
        mode: ExistenceMode,
        predicate: Option<RowPredicate>,
    },
    Metric {
        name: String,
        alias: String,
    },
    /// Explicit grouping also projects the grouping value.
    Group {
        field: FieldRef,
        alias: String,
    },
    /// An explicit request calculation, distinct from a governed named metric.
    Aggregate {
        function: AggregateFunction,
        field: Option<FieldRef>,
        distinct: bool,
        alias: String,
    },
    /// Slot is the requirement ID of a grouping or aggregate output.
    OrderOutput {
        slot: String,
        direction: Direction,
        nulls: NullOrder,
    },
    Project {
        field: FieldRef,
        alias: String,
    },
    Filter {
        predicate: RowPredicate,
    },
    Order {
        field: FieldRef,
        direction: Direction,
        nulls: NullOrder,
    },
    Limit {
        count: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateOperand {
    pub function: AggregateFunction,
    pub field: Option<FieldRef>,
    pub distinct: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroDivision {
    Null,
    Zero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistenceMode {
    Exists,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateFunction {
    Count,
    Sum,
    Min,
    Max,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RowPredicate<F = FieldRef> {
    /// Exact authored phrase-to-code grounding on the referenced field.
    CompareMapped {
        field: F,
        operator: Comparison,
        mapping: String,
        phrase: String,
    },
    Compare {
        field: F,
        operator: Comparison,
        value: Literal,
    },
    IsNull {
        field: F,
        negated: bool,
    },
    All {
        predicates: Vec<RowPredicate<F>>,
    },
    Any {
        predicates: Vec<RowPredicate<F>>,
    },
    Not {
        predicate: Box<RowPredicate<F>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputRef {
    pub slot: String,
}
pub type OutputPredicate = RowPredicate<OutputRef>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFilterStage {
    AfterAggregate,
    AfterWindow,
}

/// Exact typed values. Decimal coefficients never pass through binary floats.
/// Temporal counts use an explicit epoch/unit; interpreting natural-language dates
/// and resolving calendars/timezones is a separate binding operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Literal {
    Boolean(bool),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    #[serde(rename = "uint64")]
    UInt64(u64),
    Utf8(String),
    Decimal128 {
        coefficient: String,
        precision: u8,
        scale: u8,
    },
    /// Days since 1970-01-01 in the proleptic Gregorian calendar.
    Date32(i32),
    Timestamp {
        ticks: i64,
        unit: TimestampUnit,
        timezone: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampUnit {
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Asc,
    Desc,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullOrder {
    First,
    Last,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypedProposal {
    Graph {
        query: crate::graph::GraphQuery,
    },
    Intent {
        query: RowQuery,
        evidence: RequestEvidence,
    },
    NeedContext {
        requests: Vec<ContextRequest>,
    },
    Query {
        query: RowQuery,
    },
    NeedsClarification {
        phrases: Vec<String>,
        question: String,
    },
    Unsupported {
        reason: String,
    },
    Unresolved {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextRequest {
    Search {
        terms: String,
        relation: Option<String>,
    },
    Hydrate {
        relation: String,
        fields: Vec<String>,
    },
    Inventory {
        relation: String,
        offset: usize,
        count: usize,
    },
}

/// Preferred name for the versioned semantic contract. RowQuery is retained for
/// source compatibility with the first row-only implementation.
pub type SemanticQuery = RowQuery;
pub type SemanticOperation = RowOperation;

/// A field at row grain or an explicit grouping/aggregate requirement output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowInput {
    Field { field: FieldRef },
    Output { slot: String },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowFunction {
    Rank,
    DenseRank,
    Count,
    Sum,
    Min,
    Max,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowFrame {
    EntirePartition,
    /// RANGE UNBOUNDED PRECEDING ... CURRENT ROW includes every ordering peer.
    ThroughCurrentPeer,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowOrder {
    pub input: WindowInput,
    pub direction: Direction,
    pub nulls: NullOrder,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowSpec {
    pub function: WindowFunction,
    pub input: Option<WindowInput>,
    pub partition_by: Vec<WindowInput>,
    pub order_by: Vec<WindowOrder>,
    pub frame: WindowFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarUnit {
    Day,
    IsoWeek,
    Month,
    Year,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarPeriod {
    pub unit: CalendarUnit,
    /// Offset from the start of the period containing the reference instant.
    pub offset: i32,
    pub count: u32,
}

/// UTF-8 byte offsets into the exact original request, half-open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestSpan {
    pub start: usize,
    pub end: usize,
}
/// Retains interpretation evidence without claiming natural-language completeness.
/// Every query requirement is mandatory and must have substantive source spans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEvidence {
    pub version: u32,
    pub request_id: String,
    pub original_request: String,
    pub requirement_spans: std::collections::BTreeMap<String, Vec<RequestSpan>>,
    /// Competing interpretations must be resolved before deterministic binding.
    pub unresolved_alternatives: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentQuery {
    pub query: RowQuery,
    pub evidence: RequestEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingMatch {
    Null,
    Exclude,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LookupUsage {
    #[default]
    Project,
    Group,
}
