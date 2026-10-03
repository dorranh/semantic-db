# Semantic compiler supported profile

Status: implemented MVP profile at its release checkpoint. The compiler has
since gained additional bounded profiles; their current implementation and
verification status are tracked in the
[full requirement audit](compiler-full-requirement-audit.md). This document
describes the earlier MVP boundary and its offline tests, not a claim of general
SQL or natural-language completeness. The compatibility SQL compiler is a
separate mode.

The typed compiler uses the strict acceptance profile. A query is accepted only
when its references, exact physical types, governed definitions, output scope,
required policies and supported applicability rules bind against one immutable
catalog snapshot. Compilation plans queries but does not read result rows.

## Operation matrix

| Operation | Accepted contract | Deterministic checks | Runtime obligation / limitation |
| --- | --- | --- | --- |
| Row projection, predicate, order, limit | One relation occurrence; exact field names and typed literals; Boolean predicate tree; explicit null order | Scope, exact physical types, comparison operators, requirement IDs, one final limit and read-only planning | SQL null logic and engine binary Utf8 ordering; no implicit casts, collation conversion or computed model expressions |
| Group and aggregate | Explicit groups; count/sum/min/max; supported exact scalar inputs; optional distinct | Every dimension is grouped, aggregate/output-stage scope, exact result type and checked SUM lowering | Empty count is zero; other empty aggregates are null; signed/decimal SUM overflow fails execution |
| Governed metric | Exact authored catalog name, alias or catalog-scope-unique durable identity; source grain, result/empty contract, compatible row and lookup dimensions, metric-local filters and policies | Every exact competing name/alias remains a candidate. Exact requested unit, source grain and temporal applicability may select one; zero applicable candidates rejects and multiple applicable candidates return `ambiguous_metric` rather than using search rank | Temporal applicability supports Date32 and UTC absolute Timestamp only. Missing temporal metadata rejects calendar-filtered metric use; explicit null means unrestricted. No unit conversion, general predicate implication or approximate semantic equivalence |
| Governed ratio | Two governed Int64 aggregate components with the same source grain; authored zero and unit behavior | Component metric checks, exact optional requested ratio unit/source grain, aggregate-before-divide | Decimal128(38,18), truncation toward zero, null propagation and authored zero-denominator behavior |
| Calendar filter | Whole Gregorian day, ISO week, month or year from caller-pinned instant and IANA timezone | Nonzero bounded period, Date32 or UTC Timestamp field, half-open boundaries, DST ambiguity rejection | No fiscal calendar, rolling duration, naive timestamp or invented request clock |
| Window | rank/dense-rank/count/sum/min/max; entire partition or through-current-peer frame | Explicit ordering where required, no nesting, row-versus-group input scope, peer semantics, additive governed rollup contract | No arbitrary frames or distinct window aggregates; finalized ratios and distinct states are not scalar-summed |
| Existence / absence | Authored relationship name and role with scoped right occurrence and optional predicate | Complete authored keys, exact endpoint types, null-key contract, endpoint policies | Semi/anti semantics preserve left multiplicity; not an ordinary join or allocation |
| Dimension lookup | Authored role, exact right field, explicit missing-as-null or exclude; row or grouped usage | Metric-specific grouped-dimension whitelist and key/type checks | Same-query uniqueness guard executes on every query; duplicate matches fail rather than fan out |
| Set graph | UNION/INTERSECT/EXCEPT with explicit ALL/DISTINCT and named aligned slots | Exact column types, complete reachable acyclic graph and bounded expansion | Null-safe tuple membership; bag INTERSECT/EXCEPT use occurrence matching |
| Fact composition | Independently aggregated facts, complete key alignment, authored relationship role, explicit group domain/null/missing behavior | Complete proven group keys and lineage; global scalar special case | No unaggregated fact join, allocation, temporal relationship or implicit metric rollup |
| Graph ordering / limit | Final root slots with explicit direction/null order and optional unsigned limit | Slot existence/type and final-stage placement | Applied only after the root graph operation |

## Intent and evidence boundary

`IntentQuery` retains exact request text and one nonempty UTF-8 span mapping for
each row requirement. `GraphIntentQuery` additionally gives typed scoped
identities to every node, leaf requirement, set/composition output, final order
position and final limit. Graph record IDs escape user identifiers so distinct
scopes cannot collide. Missing, duplicate, orphan, out-of-range, overlapping or
source-text-inconsistent evidence rejects. Accepted artifacts and explicit replay
captures retain the evidence; normal records retain only its digest and validation
status.

These checks prove internal traceability only. They do not prove that an
interpreter extracted every natural-language clause. Legacy structured row and
graph requests remain accepted with an explicit `request_spans_validated=false`
guarantee.

## Global execution and lifecycle contracts

- Every compilation pins one catalog snapshot. Planning/execution and replay
  reject snapshot drift. Artifacts, records, cache keys and replay pin the
  versioned execution profile as well as the compiler pipeline and evidence
  envelope; the current pipeline is revision 8.
- A compile-time relation allow-list is not perpetual authorization. Artifacts
  compiled under a restricted scope require a current allow-list at every SQL,
  direct-plan or read execution boundary, and reject if any bound relation is
  no longer authorized.
- The MVP engine profile fixes checked integer SUM, exact integer ratio, SQL
  three-valued null comparison, explicit null ordering, binary UTF-8, UTC
  instants, aggregate/window and typed positional-parameter behavior. Compiler
  functions remain local. PostgreSQL pushdown is limited to its versioned
  declared equivalent subset; text collation and unchecked arithmetic remain
  local.
- Catalog publication validates governed field references, metric temporal
  coverage types/bounds, relationship endpoints and definition identities.
  Applicability edits change semantic/object revisions and therefore cache and
  replay identity.
- Access scope applies to retrieval, hydration, structured binding and replay.
  Row policies are unavoidable. Normal records do not include raw requests,
  literals or model responses.
- Work, recursion, bytes, model calls, context expansion, graph expansion,
  admission, cache retention, cancellation and deadlines are bounded.
- Normal records digest model/user requirement identities, account for graph
  nodes, edges and checked outputs, and fingerprint graph relational state.
  Exact request text and identities remain confined to explicit artifacts and
  bounded captures.
- Ordinary Rust tracing and compiler-owned records are supported. OpenTelemetry
  is intentionally out of scope.

## Explicitly unsupported in this profile

General unit/currency conversion, arbitrary temporal joins, slowly changing
dimension validity, allocation, many-to-many fact multiplication, SQL predicate
implication, general collation equivalence, model-authored expressions/SQL,
recursive graphs, post-composition calculations, graph caching and proof of
natural-language completeness remain unsupported or future work. A bounded
retrieval miss never establishes catalog-wide absence.

Primary deterministic evidence is in `crates/semantic-compiler/tests/typed.rs`,
`crates/semantic-compiler/tests/mvp_acceptance.rs`,
`crates/semantic-compiler/tests/retrieval.rs`,
`crates/semantic-catalog/tests/publication.rs`, and
`apps/semantic-server/tests/typed.rs`. The checked-in acceptance expectations are
in `crates/semantic-compiler/tests/fixtures/mvp_acceptance.json`; the release
evidence index is [the MVP release audit](compiler-mvp-release-audit.md). Live
model and database coverage is a separate optional release artifact.
