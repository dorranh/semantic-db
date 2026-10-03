# Typed semantic compiler: implemented profiles

Implemented incrementally on 2026-09-28 against the
[compiler architecture](semantic-compiler-architecture.md). This is not completion
of delivery stages A–D. The public typed API is opt-in; existing CLI/server SQL
compilation responses are unchanged. No OpenTelemetry integration is introduced.

## Entry points

- `semantic_compiler::typed::compile_rows(&engine, query, options)` accepts an
  untrusted `semantic_plan::typed::RowQuery` without requiring a model provider.
- `Compiler::compile_typed(&engine, request, options)` asks the configured provider
  for the same row-query proposal and validates it through the same pipeline.
- `Compiler::compile_sql_compatibility` explicitly names the original model SQL
  mode. `Compiler::compile` remains its backward-compatible alias. Typed failure
  never falls back to compatibility SQL.

Both new entry points return `TypedCompilation { version: 1, outcome, record }`.
Outcomes distinguish compiled, clarification, unsupported, unresolved, rejected,
and provider failure. The old `GroundingOutcome` wire contract is unchanged.

A structured example, after registering an `items` relation with Int64 `id` and
Boolean `active` fields:

```rust
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_plan::typed::*;

let query = RowQuery {
    version: ROW_QUERY_VERSION,
    input: RelationInput { relation: "items".into(), instance: "r".into() },
    requirements: vec![
        Requirement {
            id: "ids".into(),
            source_text: "List IDs".into(),
            operation: RowOperation::Project {
                field: FieldRef { instance: "r".into(), field: "id".into() },
                alias: "id".into(),
            },
        },
        Requirement {
            id: "active".into(),
            source_text: "active items".into(),
            operation: RowOperation::Filter {
                predicate: RowPredicate::Compare {
                    field: FieldRef { instance: "r".into(), field: "active".into() },
                    operator: Comparison::Eq,
                    value: Literal::Boolean(true),
                },
            },
        },
    ],
    unresolved: vec![],
};
let compilation = compile_rows(&engine, query, CompileOptions::default()).await;
if let TypedOutcome::Compiled { query } = compilation.outcome {
    // Compilation only plans. This separate call explicitly executes rows and
    // carries the engine's query limits/cancellation through parameter binding.
    let execution = query.execute(&engine, Default::default()).await?;
    let batches = execution.collect().await?;
}
```

`query.sql()` provides a parameterized SQL artifact. Its values are separate from
its statement; do not submit the statement alone when it contains placeholders.
`query.plan_direct(&engine)` rebuilds a DataFusion DataFrame for inspection and
backend comparison. Use `query.execute` for normal execution through the engine's
runtime-budget path. Both reject a changed catalog snapshot.

## Implemented guarantees

Catalog registration builds immutable relation revisions and exact-case field
indexes. Snapshots share indexed relation entries and cache their manifest hash;
registration invalidates only the current snapshot handle, preserving readers of
older snapshots. Metadata, source bindings, physical schema, authored view SQL,
semantics and origin metadata participate in revision hashes. Physical source
identifiers are hashed but not included in model context. Field-name collisions
fail lookup rather than selecting one occurrence arbitrarily.

Catalog roots use a persistent, canonical Merkle treap. Snapshot pinning is O(1);
updates copy and hash the changed tree paths. `Catalog::apply_changes` validates
batches atomically and reports the transitive affected closure, including old
edges. `CatalogStore` supports optimistic publication and pinned readers. Initial graph validation traverses the catalog with explicit work bounds. Warm
publication reuses persistent reverse dependencies, revalidates affected objects
and checks changed view edges for cycles. Object revisions distinguish
semantic content, physical bindings, and full provenance.

`DeferredBackend` optionally registers schema-only providers and resolves only
selected sources at physical scan time. Resolution concurrency and its shared
LRU cache are bounded; provider schemas are checked for drift. Existing eager
loading remains available.

The current row profile uses one explicitly named relation occurrence, including
an authored view. It supports projections, Boolean predicates, comparisons,
null tests, sorting with explicit null placement, and a single nonnegative limit.
All fields have exact occurrence and field references. Unknown fields, cross-scope
references, unresolved choices, duplicate requirement IDs, duplicate output
aliases, empty Boolean groups, and conflicting limits are rejected.

Comparisons require exact physical types: Boolean, Int16/Int32/Int64, Utf8,
Decimal128 with explicit precision/scale, Date32, or Timestamp with explicit
unit and UTC or timezone-free semantics. Decimal coefficients are strings and
never pass through binary floating point. Boolean comparison supports equality
and inequality only. Relative whole-calendar filters use the separate explicit context profile
described below; other civil-time conversions remain unsupported. The comparison profile uses DataFusion 55, SQL null logic,
and binary UTF-8 comparisons. Projection and null tests accept other Arrow types.

`Group` and `Aggregate` support explicit grouping, COUNT, SUM, MIN and MAX,
including distinct values. Counts ignore null field values and return zero for
empty input; the other supported aggregates return null. `OrderOutput` uses
requirement IDs as output slots. Ungrouped projections in aggregate queries fail
binding. Ordinary row sorting precedes projection; limiting follows ordering.

Application-authored `MetricDefinition` contracts add pinned identity, compatible
dimensions, source grain, result type, unit, metric-local filters and empty-input
behavior. `Metric` selects the definition exactly. `RowPolicy` predicates always
apply before aggregation in this compiler. These policies are not a replacement
for authorization in the explicit SQL APIs or source database.

`Related` expresses existence or absence through a named relationship, explicit
role and distinct occurrence ID. Semi/anti joins preserve left multiplicity even
with duplicate matches. Key null behavior is authored. Related predicates and
endpoint policies apply in the related occurrence. No cardinality declaration
is needed for this semantics-preserving formulation. Ordinary fan-out joins,
allocations and multi-fact composition are not yet enabled.

Only validation can construct `BoundQuery`. Bound and relational artifacts have
private fields and serialization without deserialization; persisted data cannot
bypass validation. Each operation belongs to a requirement. Lowering verifies
that every requirement survives exactly once and that node linkage is valid.
The interpreter's requirement extraction and authored-prose interpretation remain
fallible; these checks do not prove that the request was understood completely.

Lowering emits versioned scan, filter, semi/anti join, aggregate, sort, project and fetch operations. The SQL emitter
builds identifiers, expressions and typed placeholders as SQL AST nodes. Dynamic
values are never interpolated. The direct backend constructs DataFusion
expressions over a scan admitted through the engine's registered-relation gate.
Compilation validates both backends and compares their Arrow output schemas
without creating physical scans or executing rows. Direct scans select only fields
used by projections, predicates and ordering, avoiding a wide SELECT-star
intermediate. SQL emission isolates source sort keys from shadowing output aliases
using internal slots when needed. DataFusion still owns schema resolution and
planning; indexed binding is not a general backend planning-cost guarantee.

Full context remains the default. `SelectionMode::Retrieved` performs bounded
exact-name, alias and lexical discovery across relations, fields and metrics;
`Auto` falls back from an oversized full bundle to retrieval. Governing metadata,
known definition dependencies and alternatives are hydrated or produce an
explicit incomplete outcome. View and relationship endpoint closure currently
hydrates full endpoint schemas conservatively. Manifests retain snapshot/index
revisions, inclusion reasons, field completeness and search cutoffs. Negative
lookups retain fingerprints. A subset never proves catalog-wide absence.

The model can request search, hydration or inventory pages. Newly referenced
fields trigger expansion and another interpretation round before binding.
Expansion, repair, total-call and cumulative input/output budgets are independent.
There is no SQL fallback. Retrieval quality still requires live-model evaluation
before changing the default.

Ossie documents now expose their source archive and normalized source tree with
byte spans and JSON pointers. Exact number spelling, decoded prose, aliases and
origins, missing/null/false/empty distinctions and explicit tags survive. Authored
`is_time` absence remains absent. Generic fact resolution retains authority,
verification scope and conflicting contributors without silently promoting prose
or model proposals. Unknown Ossie executable extensions, metrics and relationships
still fail import until a sound format-specific adapter exists; application-owned
governed definitions use the typed catalog API.

## Records and limits

Every typed attempt returns a compiler-owned record, independently of tracing
subscription. It contains the mode, snapshot ID, final outcome, stage codes,
stage/service timings, indexed lookup/node counts, context sizes, model calls,
and cumulative model input/output bytes across repair history. Bytes are not
reported as token counts. Provider envelopes expose actual usage when available,
including cached input and reasoning tokens. Every attempt retains provider
status and identity, including refusal/truncation and interrupted calls. Missing
usage is unknown; aggregate accounting separately counts missing reports.
An accepted artifact has a deterministic SHA-256 digest excluding timestamps and
trace state. Replaying a saved proposal against the same catalog and compiler
version reproduces the artifact digest; no historical data snapshot is supplied.

Normal records and tracing events exclude request text, model response text,
parameter values, connection details and raw backend errors. The returned query
artifact intentionally contains the proposal, literals and SQL needed by its
caller. Applications control artifact storage and retention; it is not silently
exported by tracing. There is no debug retention store or telemetry exporter.

`CompileOptions` bounds query input bytes, expression nodes/depth, full-context
fields/bytes, response bytes, SQL bytes and total elapsed time. Depth has a hard
ceiling of 64 and nodes a ceiling of 8192. Cancellation interrupts async model and
planning waits; CPU loops check cancellation/deadline as they visit nodes and
fields. Backend calls are subject to an outer timeout, but cancellation cannot
preempt an indivisible synchronous DataFusion operation. A custom provider is
responsible for bounding allocation before returning its String; the built-in
provider already bounds its HTTP response body.

Rust `tracing` provides an async compilation span, stage spans, and structured
stage/completion events. Applications choose their subscriber. No OpenTelemetry dependency,
exporter, collector, or service is required.

## Verification and next stages

The deterministic tests cover independently expected results through direct and
emitted-SQL execution; Boolean/null behavior, filters, ordering, zero limits,
duplicates, case sensitivity, literal injection, quoted identifiers, views,
stale snapshots, type/scope rejection, requirement coverage, replay digests,
context/work limits, provider failures, deadline and cancellation. A synthetic
100-relation catalog plus a 10,000-field relation checks indexed binding counts
and uses providers whose `scan` panics to enforce compilation without row access.
Scripted model tests validate the protocol and orchestration, not live-model
interpretation quality.

Remaining work is tracked in [the implementation audit](compiler-implementation-progress.md).
The full architecture is not complete. Snapshot identity identifies metadata,
not a historical source-data snapshot. Existing engine and connector execution
contracts continue to apply.

Local differential tests cover typed row/aggregate/relationship execution,
governed metric/policy scopes, exact decimal/time parameters, publication and
source fidelity. Scripted model tests exercise retrieval, repair, cancellation
and provider accounting without a live model. See the implementation audit for
current commands and remaining evaluation gates.

## Service and CLI integration

`compile_semantic` and the `SemanticQuery`/`SemanticOperation` aliases are the
preferred names for the expanded structured contract. The original `compile_rows`
and `RowQuery` names remain source-compatible.

```sh
sdb ask --compiler-mode typed-full --compile-only 'Count active items'
sdb ask --compiler-mode typed-retrieved 'List active item IDs'
```

`--compiler-mode sql-compatibility` remains the default. Typed compile-only output
is the versioned JSON compilation envelope, including separate SQL parameters.
Typed execution preserves selected read-consistency and cache policies.

HTTP exposes `POST /v1/compile/semantic` with
`{"question":"Count items","context":"full"}` (context also accepts `retrieved`
or `auto`), and `POST /v1/compile/query` with a structured SemanticQuery. Structured
compilation requires no model credentials. Both routes only compile; they never
collect query rows. The original `/compile` contract remains unchanged.

`CompiledQuery::capture_replay(max_bytes)` creates an explicitly retained bundle
containing the proposal, literals, snapshot, pipeline revision and artifact digest.
`ReplayBundle::replay` revalidates against that snapshot and compiler pipeline;
a serialized wrapper never bypasses binding. Capture limits fail independently
of the already-compiled query. This does not replay a model call or preserve
historical source rows.

## Additional executable profiles

- `Ratio` and governed `ratio_metrics` aggregate integer components, then use
  `semantic_ratio_i64_v1`: Decimal128(38,18), truncation toward zero, null
  propagation, and explicit null/zero handling for a zero denominator.
- `Window` supports rank/dense-rank and count/sum/min/max over explicit partitions,
  order keys and entire-partition or through-current-peer frames. Grouped-query
  windows bind output requirement slots; row windows bind source fields. Windows
  execute after policies, filtering and grouping, and before final sort/fetch.
- `CalendarFilter` resolves whole day, ISO-week, month or year intervals from
  caller-supplied `RequestContext`. Intervals are half-open, Gregorian and local
  to the explicit IANA timezone. DST days may be 23/25 hours. Ambiguous/nonexistent
  boundaries and naive timestamp fields are rejected. Fiscal calendars and
  rolling elapsed-duration periods are not implied by these operations.
- Governed metrics accept exact optional `required_unit` and ordered
  `required_source_grain` constraints. A metric with authored temporal
  applicability requires exactly one `CalendarFilter` on its governed field, the
  exact authored calendar unit, and a resolved interval contained by its
  half-open Date32 or UTC Timestamp coverage. Missing temporal facts reject a
  calendar-filtered metric; explicit null is the authored unrestricted state.
  This is a decidable profile, not unit conversion or general predicate
  implication.
- `Lookup` projects a dimension field through an authored relationship role with
  an explicit preserve-as-null or exclude-missing contract. Separate occurrences
  retain billing/shipping roles. A same-query grouped-key count guard rejects
  duplicate matches instead of multiplying root rows or picking a value. This
  currently supports row-grain projections; grouped dimension composition needs
  its own grain and metric-compatibility contract. Compilation reports checks as
  `pending_each_execution`; it never verifies data during planning.
- Typed SUM uses `semantic_sum_v1`, with a Decimal256 intermediate and checked
  final Int64/Decimal128 bounds, including DISTINCT state and windows. Final
  overflow is an execution error, never wrapped arithmetic. Empty sums are null.
  Compiler functions stay local during federation; ordinary eligible children
  may still ship. Emitted SQL targets an Engine with these functions registered.

`IntentQuery` adds exact original request text and UTF-8 byte source spans for each
mandatory requirement. The binder validates span integrity and source-text
consistency, and rejects unresolved alternatives. This evidence is retained in
accepted artifacts and explicit replay captures, separately from normal records.
It proves traceability, not completeness or correctness of language interpretation.
The model can return `status:intent` in the same call as its query proposal.

`/v1/compile/intent` accepts `{intent, request_context?}` without a model.
`/v1/compile/semantic` additionally accepts `request_context` for model-driven
calendar interpretation. Replay pins context and evidence; callers cannot replace
them while replaying. Compilation cache identity includes both.

The complete current boundary is summarized in
[the supported profile](compiler-supported-profile.md).
