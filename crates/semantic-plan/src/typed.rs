//! Version 1 row-query proposals. These types are untrusted, including when
//! deserialized from retained artifacts. Only the compiler can bind them.
use serde::{Deserialize, Serialize};

/// Exact decimal quantization; no floating point intermediates are permitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecimalRounding {
    Truncate,
    HalfEven,
    HalfAwayFromZero,
}
/// Requested exact decimal output representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecimalResultType {
    pub precision: u8,
    pub scale: u8,
}
/// Currency comes from each authored rate row, rather than a guessed constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RateAmountUnit {
    RateSourceCurrency,
}
/// The first exact rate profile accepts major currency units only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateAmountBasis {
    Major,
}
/// A request amount, repeated over genuine authored rate rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RateAmount {
    Literal {
        value: Literal,
        unit: RateAmountUnit,
        basis: RateAmountBasis,
    },
}

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
    /// Apply an exact authored concept predicate at row stage. Resolution is
    /// scoped to the selected relation and fails on competing exact aliases.
    ConceptFilter {
        concept: String,
        /// Exact typed values for placeholders in the authored concept.
        #[serde(default)]
        arguments: std::collections::BTreeMap<String, Literal>,
    },
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
    /// Follow exactly authored relationship roles through bounded occurrences.
    /// Each hop executes its own endpoint policies and uniqueness guard.
    PathLookup {
        hops: Vec<PathHop>,
        field: String,
        alias: String,
        missing: MissingMatch,
    },
    /// Allocate an authored source amount across one checked target dimension,
    /// then sum the exact minor-unit shares by that target.
    Allocate {
        allocation: String,
        target_aliases: Vec<String>,
        amount_alias: String,
    },
    /// Apply an exact, authored rational conversion to its declared Int64
    /// source field. The result is Decimal128(38,18).
    Convert {
        conversion: String,
        alias: String,
    },
    /// Quantize a decimal literal against every surviving authored rate row.
    /// The result uses the requested exact precision and scale in target major units.
    ConvertRate {
        rate: String,
        amount: RateAmount,
        result_type: DecimalResultType,
        rounding: DecimalRounding,
        alias: String,
    },
    /// Convert through one authored, dated rate relation under the same read.
    /// The result is Decimal128(38,18) in the rule's target currency.
    CurrencyConvert {
        rate: String,
        alias: String,
    },
    /// A half-open interval of whole calendar periods, anchored by host context.
    CalendarFilter {
        field: FieldRef,
        period: CalendarPeriod,
    },
    /// Group observed UTC microsecond instants by their UTC calendar month.
    /// This does not generate empty periods or imply a business calendar.
    CalendarGroup {
        field: FieldRef,
        grain: CalendarUnit,
        timezone: String,
        alias: String,
    },
    /// Explicitly fill absent UTC months for one observed CalendarGroup and
    /// one non-distinct COUNT(*). Bounds are first-of-month UTC microseconds;
    /// the end is exclusive. This bounded profile only permits zero fill.
    CalendarFill {
        month_slot: String,
        count_slot: String,
        start_us: i64,
        end_us: i64,
        fill: i64,
    },
    /// Resolve exactly one policy-visible authored calendar row for a Date32
    /// source value. Missing or duplicate dates fail the same query.
    BusinessCalendar {
        calendar: String,
        field: BusinessCalendarField,
        alias: String,
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
        /// Conjunctive target-scoped Filter, ConceptFilter, or Related requirements.
        #[serde(default)]
        target_requirements: Vec<Requirement>,
    },
    Metric {
        name: String,
        alias: String,
        /// Optional semantic constraints stated by the request. Omission does
        /// not invent a unit or grain; catalog temporal restrictions are still
        /// enforced against actual calendar-filter operations in the query.
        #[serde(default)]
        applicability: MetricApplicability,
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
    /// Explicit bounded offset/fetch. This is presentation pagination and does
    /// not claim stable pages unless the request supplies sufficient ordering.
    Page {
        offset: u32,
        fetch: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathHop {
    pub relationship: String,
    pub role: String,
    pub instance: String,
    /// Reserved for the checked half-open interval profile. Rejected by the
    /// current compiler until same-query interval uniqueness is executable.
    #[serde(default)]
    pub as_of: Option<PathAsOf>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathAsOf {
    pub fact_time: String,
    pub valid_from: String,
    pub valid_to: String,
    pub timezone: Option<String>,
}

/// Narrow, exact applicability requirements for a governed metric use. General
/// conversions and grain implication are intentionally outside this contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricApplicability {
    #[serde(default)]
    pub required_unit: Option<crate::meaning::Unit>,
    #[serde(default)]
    pub required_source_grain: Option<crate::meaning::SourceGrain>,
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
    Avg,
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
    /// Required typed value supplied only through a prepared binding. An
    /// ordinary row or graph compilation rejects this unbound proposal.
    CompareParameter {
        field: F,
        operator: Comparison,
        parameter: String,
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
    Float64(f64),
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
    /// An explicit proleptic Gregorian civil date, encoded as YYYY-MM-DD.
    /// Binding validates the date and converts it to Date32 without a timezone.
    GregorianDate(String),
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
    GraphIntent {
        query: crate::graph::GraphQuery,
        evidence: crate::graph::GraphRequestEvidence,
    },
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
    /// ROWS UNBOUNDED PRECEDING ... CURRENT ROW requires a strict grouped order.
    RowsThroughCurrent,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessCalendarField {
    FiscalYear,
    FiscalPeriod,
    BusinessDay,
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

/// Visit every request requirement in stable depth-first order. The visitor
/// runs before descent, allowing callers to enforce their depth/work budget.
pub fn visit_requirements<'a, E>(
    requirements: &'a [Requirement],
    visitor: &mut impl FnMut(&'a Requirement, usize, &[usize]) -> Result<(), E>,
) -> Result<(), E> {
    fn walk<'a, E>(
        requirements: &'a [Requirement],
        path: &mut Vec<usize>,
        visitor: &mut impl FnMut(&'a Requirement, usize, &[usize]) -> Result<(), E>,
    ) -> Result<(), E> {
        for (index, requirement) in requirements.iter().enumerate() {
            path.push(index);
            visitor(requirement, path.len(), path)?;
            if let RowOperation::Related {
                target_requirements,
                ..
            } = &requirement.operation
            {
                walk(target_requirements, path, visitor)?;
            }
            path.pop();
        }
        Ok(())
    }
    walk(requirements, &mut Vec::new(), visitor)
}
/// Mutable counterpart of visit_requirements, with the same stable ordering.
pub fn visit_requirements_mut<E>(
    requirements: &mut [Requirement],
    visitor: &mut impl FnMut(&mut Requirement, usize, &[usize]) -> Result<(), E>,
) -> Result<(), E> {
    fn walk<E>(
        requirements: &mut [Requirement],
        path: &mut Vec<usize>,
        visitor: &mut impl FnMut(&mut Requirement, usize, &[usize]) -> Result<(), E>,
    ) -> Result<(), E> {
        for (index, requirement) in requirements.iter_mut().enumerate() {
            path.push(index);
            visitor(requirement, path.len(), path)?;
            if let RowOperation::Related {
                target_requirements,
                ..
            } = &mut requirement.operation
            {
                walk(target_requirements, path, visitor)?;
            }
            path.pop();
        }
        Ok(())
    }
    walk(requirements, &mut Vec::new(), visitor)
}
