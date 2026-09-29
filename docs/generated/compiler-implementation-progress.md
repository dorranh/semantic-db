# Compiler implementation progress

Objective: implement the contracts in [the architecture](semantic-compiler-architecture.md),
with ordinary Rust tracing and compiler-owned records. OpenTelemetry export is
out of scope per the implementation request. This checklist is an implementation
audit, not a declaration that the full design is complete.

The current executable boundary is listed in
[the supported profile](compiler-supported-profile.md).

## Delivery and evidence

- [x] Initial typed row slice: snapshot references, scoped field binding, requirement
  coverage, deterministic relational lowering, SQL AST emission, direct DataFusion
  planning, cancellation/work limits, records, compatibility mode. See
  [the first-slice guide](compiler-typed-rows.md). Baseline: 32 tests and Clippy pass.
- [ ] Catalog fact/source model: source archives/maps, fact authority/conflicts,
  capability states, missing/null distinction, governed definitions and typed edges.
- [ ] Catalog lifecycle: atomic publication, incremental reverse dependencies,
  source/semantic/binding revisions, bounded retention, demand-driven providers.
- [ ] Exact/alias/lexical field and relation search, bounded context hydration,
  dependency/alternative closure, manifests, recovery and independent expansion budgets.
- [ ] Full intent/semantic query contracts: request context/spans, value origins,
  relationship occurrences, grouping, temporal interpretation, requirement ledger.
- [ ] Executable concepts, metrics, relationships and policy binding with authority,
  grain, fan-out, coverage and acceptance-profile validation.
- [ ] Semantic lowering: metrics, joins, temporal conversions, multi-fact composition,
  ratios, windows, set operations, verification and deterministic decision provenance.
- [ ] Backend capability profiles, exact numeric/time types, parameters, function
  registry and supported dialect emission with differential execution tests.
- [ ] Versioned public API/CLI integration and importer support for executable profiles.
- [ ] Provider usage/status envelopes, request correlation, replay/capture policy,
  stage fingerprints, aggregate accounting and export-independent records.
- [ ] Bounded compilation/index/analysis caches with positive and negative lookup
  dependencies, change-aware invalidation and retention.
- [ ] Evaluation fixtures, mutation/property cases, sufficient-context comparisons,
  synthetic scale matrix, benchmarks, measured release gates and shadow mode.

Optional embeddings, extra SQL dialects and external IR interchange require the
measurement/consumer justification stated in the architecture; they must not be
represented as implemented merely by adding empty interfaces. Any unsupported
semantic capability must remain explicit until its implementation and tests exist.

## Working constraints

The Postgres connector landed as d53b348 during the initial slice. Additional
engine parameter/read/write edits are in flight. Preserve those changes and
integrate against the current checkout without reverting, staging or committing
other work. Generated documentation stays in docs/generated.

## Implemented since the first slice

- Persistent canonical catalog roots; constant-time snapshot pins; separate full,
  semantic and binding revisions; atomic bounded update validation, transitive
  affected reports and optimistic publication. Initial validation is full; warm updates now reuse a persistent dependency analysis.
- Optional deferred providers with bounded concurrency, LRU retention, shared
  resolutions and schema-drift rejection before execution.
- Exact/alias/lexical global field and metric search; full/retrieved/auto modes;
  scoped context manifests, authoritative dependency closure, bounded expansion
  and reconsideration after new context. Cold indexes build once; warm publications incrementally replace changed postings.
- Source archives and normalized YAML trees with exact scalar spelling, source
  spans, alias origins and absence preservation. Fact authority/conflict types
  and executable/descriptive/blocked capabilities are explicit.
- Exact signed integer, decimal, date and absolute UTC/timezone-free timestamp
  literals; group/count/sum/min/max/distinct queries; slot-based aggregate order.
- Application-authored governed aggregate metrics, compatible dimensions,
  source grain, result/empty contracts, metric-local filters and row policies.
- Authored relationship roles with nullable-key contracts; existence and absence
  lower to semi/anti joins with related-occurrence filters and policies.
- Provider completion envelopes and actual per-attempt token accounting,
  including refusal/truncation and explicit missing usage. Legacy text providers
  remain compatible and report usage as unknown.

These are implemented subsets of the unchecked broader contracts above. In
particular, general joins/allocation and temporal relationship evidence, broader
window/temporal profiles, broader analysis caches and measured release gates remain. Supported ratio/window/calendar
profiles and API/CLI integration are described below.

Validation uses offline local fixtures and deterministic backend comparisons.
Restricted localhost/database integration access must be recorded separately and
must not stop implementation. No OpenTelemetry is used. Clippy across catalog, compiler, engine, Ossie, server
and CLI passed with warnings denied after capture/accounting and HTTP integration.

Additional integration now includes explicit `sdb ask --compiler-mode` choices
(`sql-compatibility`, `typed-full`, `typed-retrieved`, `typed-auto`), versioned HTTP
`/v1/compile/semantic` and `/v1/compile/query` routes, and typed execution through
engine read-consistency/cache options. Existing compatibility entry points remain.
The structured HTTP route is tested in-process without opening a socket.

Compilation records now include distinct run IDs, pipeline/build identity,
prompt/bound/relational/artifact fingerprints, requirement rule dispositions and
definition revisions. Explicit bounded `capture_replay` retains a proposal and
snapshot/pipeline identity; replay always binds again and verifies the resulting
artifact digest. Structured replay binds again without executing historical rows. The opt-in
`RecordingProvider` additionally captures bounded exact message/completion
transcripts. `TranscriptProvider` replays offline, rejects prompt drift and
incomplete captures, and checks complete consumption. Capture exhaustion does not
change compilation outcomes. `CompilerMetrics` collects bounded aggregate outcome,
work, latency and actual-token accounting independently of tracing subscribers.
Exporters read snapshots outside the compilation path.

Catalog validation now checks metric/policy field references and relationship
endpoints/types. Relationship edges participate in invalidation while remaining
separate from acyclic view expansion. YAML merge normalization retains precedence
and inherited source locations as well as aliases.

The next implemented slice adds exact integer ratios (including governed component
metrics), window ranks and aggregate windows with explicit peer-aware or whole
partition frames, and deterministic Gregorian calendar filters with a pinned host
reference instant and IANA timezone. Ratio arithmetic is a versioned local UDF;
a recording in-process remote executor verifies that aggregate pushdown remains
possible while the exact division stays local. Tests cover 1/3 precision, signs,
integer extremes, zero/null behavior, ordering peers, grouping-before-windows,
23/25-hour days, leap February, and replay context identity.

Definition revisions are now precomputed at publication, cold search construction
charges source text bytes before tokenization, and related endpoint policies have
the same identity checks as root policies. Structured compilation sessions provide
bounded admission, LRU byte/entry retention and duplicate-work coalescing. Cache
keys include request context/evidence, scope, pipeline and full snapshot. Narrow
incremental invalidation is still outstanding.

`IntentQuery` retains the exact original request and a mandatory requirement/span
ledger. UTF-8 boundaries, range, ordering, nonempty coverage and source-text
consistency are checked. The model can return this evidence in the same proposal;
`/v1/compile/intent` accepts it without a model. Normal records keep only a request
digest and whether spans were validated. Accepted artifacts and explicit replay
captures retain sensitive original text. Legacy structured queries remain usable
and explicitly have no span-validation guarantee. These checks do not establish
semantic correctness or completeness of interpretation.


Strict row-grain dimension lookups now implement role-separated billing/shipping
selection, missing-match choices and same-query uniqueness obligations. The
compiler never equates authored cardinality with enforcement. Records disclose
pending execution checks; duplicate dimension keys fail rather than fan out.
Broader joins and allocation remain outstanding; grouped lookup dimensions are now implemented below. Numeric review
also found DataFusion's wrapping SUM behavior: typed sums now use a versioned
checked aggregate with wide exact state, final overflow errors and DISTINCT
merge state. This function remains local until a backend can implement its
contract exactly.


Catalog publication now retains persistent reverse dependency analysis. Warm
updates revalidate only affected relations, preserve old edges for invalidation,
and check changed view edges for cycles. Search indexes share persistent postings;
updates retokenize changed relation text and skip unchanged posting lists. Old
snapshots remain queryable; no ancestor snapshot chain is retained for index seeds.
[Scale measurements](compiler-scale-measurements.md) include one million fields.

The Ossie adapter now imports its foreign-key relationship profile with explicit
role naming, exact equality/null behavior, scoped AI annotations and source refs.
Cardinality is retained as authored, with a warning; strict lookup execution still
checks actual multiplicity. Unsupported metric expressions/extensions still fail
explicitly rather than acquiring invented grain or aggregation contracts.

HTTP compiler routes now share bounded admission (429 on saturation); structured
query/intent routes use an owned immutable-engine compilation session for bounded
LRU reuse. `/v1/compiler/metrics` exposes aggregate compiler/cache accounting without
raw requests. In-process tests cover saturation/recovery, cache reuse and request
evidence. No listening socket or model service is needed for these tests.

Further implemented contracts: explicit after-aggregate and after-window output
predicates; unsigned rank parameters; authored additive metric rollup dimensions
with rejection of scalar distinct/ratio merges; exact Utf8 phrase-to-code
dictionaries with pinned parameter provenance and bounded expansion; and grouped
checked dimension lookups with metric-specific relationship/field/missing-match
whitelists. Catalog dictionary edits invalidate search coherently. Offline
differential tests cover each path. Query graph/composition work follows.

The query-graph implementation now supports UNION/INTERSECT/EXCEPT with explicit
ALL/DISTINCT semantics and independently aggregated fact composition. It validates
complete group-key alignment against authored relationship roles and exposes
group-domain, null alignment and missing-group decisions. A duplicate-sensitive
differential fixture found DataFusion 55 membership lowering unsuitable for
INTERSECT/EXCEPT ALL; occurrence matching implements the required bag semantics in
both adapters. All 43 typed tests pass at this checkpoint, including model graph interpretation,
access scope, generated SQL scope isolation and exponential-expansion rejection. See
[query graph contracts](compiler-query-graphs.md) for current integration limits.

Clippy across catalog, compiler, engine, Ossie, server and CLI passes with warnings
denied after graph integration. The broad offline integration suite passes after the user-requested workspace
Cargo clean: 136 tests across 19 binaries, with no failures or ignored tests.
A subsequent focused graph-replay test also passes. The user's connector/runtime
changes remain intact; the only compiler integration added to the in-flight engine
parameter file is UInt64-to-unsigned-BIGINT type mapping for rank comparisons.

The next continuation completed graph-wide request evidence and a narrow metric
applicability profile. `GraphIntentQuery` now covers every node, leaf requirement,
set/composition output, final order and final limit with collision-free scoped
identities and exact UTF-8 span validation. Evidence is threaded through model
proposals, structured HTTP, records, artifacts and graph replay; legacy graphs
remain explicitly unvalidated. Governed metric uses can require an exact unit and
source grain. Authored temporal restrictions bind one real calendar filter to an
exact calendar grain and a half-open Date32/UTC Timestamp coverage interval;
unknown or incompatible applicability fails closed. Publication validates and
revisions these contracts, and context hydration includes the governed time
field. The pipeline revision at that checkpoint was 7.

Validation at that continuation checkpoint: the broad offline gate passes 143 tests
across the selected catalog/compiler/engine/Ossie/server/CLI libraries and test
binaries, including 48 typed compiler, 8 catalog publication, 6 retrieval, 3
in-process server typed and 5 CLI command tests. Formatting, diff checks and
strict Clippy across all targets of those six packages pass. At that checkpoint,
full-design gaps included backend capability profiles, broader semantic
applicability/alternatives, held-out evaluation, post-composition calculations
and graph caching. The MVP completion checkpoint below supersedes this interim
status while retaining the historical evidence.

## MVP completion checkpoint

The recommended narrow MVP profile is complete as of 2026-09-29. Binding now
preserves exact competing metric/ratio names and aliases across the authorized
catalog scope. Exact unit, source-grain and temporal contracts may disambiguate a
single candidate; otherwise binding returns stable applicability, grounding,
identity, scope or ambiguity diagnostics. Durable identities must be unique, and
window rollup validation consumes the canonical resolved metric.

The engine exposes a versioned MVP execution profile for checked integer sums,
exact ratios, null comparison and ordering, binary text behavior, UTC timestamps,
aggregate/window behavior, typed parameters and local-only compiler functions.
The PostgreSQL connector exposes a versioned conservative subset and keeps
unproved text collation and arithmetic local. Compiler artifacts, records, cache
keys and replay pin the execution profile; the compiler pipeline is revision 8.
Artifacts compiled with a restricted relation scope require a current scope at
every direct, SQL and read execution boundary.

The records/privacy audit now uses opaque digests for model/user requirement IDs
in normal records, counts graph nodes, edges and outputs, and fingerprints graph
relational state. Exact evidence remains in explicit artifacts and bounded replay
captures. The release suite adds independently stored expected rows and diagnostic
codes for governed policy, relationship roles and duplicate keys, bag sets,
missing/null/composite/scalar fact composition, output filter stage, calendar/DST,
applicability mutation, ambiguity, scope and context bounds.

Final offline evidence: 157 tests pass across 20 selected catalog/compiler/engine/
Ossie/server/CLI binaries, including 51 typed compiler tests and 10 MVP acceptance
tests. The backend lane separately passes 9 PostgreSQL library tests. Strict
all-target Clippy passes across those packages plus PostgreSQL; `cargo fmt --all
-- --check` and `git diff --check` pass. No live database or model-service test is
claimed, and OpenTelemetry remains out of scope.

This closes the P0 MVP gate only. The unchecked broad architecture items above
remain intentionally broader than this profile. The next functional work is P1:
post-composition calculations, broader semantic/applicability types, graph and
dependency-aware caches, importer/deferred-source expansion where demanded,
held-out model evaluation and repeatable production performance measurements.
