# In-process exact-rate adapter plan

Status: design only, pending parent authorization after the frozen rowwise slice is verified and committed. This stage excludes the public receipt store, PG marker, new HTTP execution route and negative artifact migration. The exact lookup/oracle semantics are defined in `exact-decimal-rate-lookup-outcome-design.md`.

## Small closed engine boundary

Introduce an engine-owned immutable `BoundQueryFunctions` adapter with private fields and no Deserialize implementation. It contains only reviewed exact-rate aggregate instances, not arbitrary caller-provided UDFs or SQL. The compiler constructs entries after binding the published rate profile and exact selector. Each entry contains validated physical Decimal128 rate type, the runtime-owned typed guard origin and an opaque compiler-owned SQL alias derived from the sealed descriptor digest. Cloning shares immutable definitions; every physical execution creates fresh accumulator state.

Proposed signatures (names may follow repository conventions):

```rust
// semantic-engine: closed construction, never deserialized from model output.
pub struct BoundQueryFunctions { /* private exact-rate instances */ }

impl BoundQueryFunctions {
    pub fn exact_rate(
        rate_type: DataType,
        origin: SemanticDataOrigin,
    ) -> Result<Self>;
    pub fn combine(&self, other: &Self) -> Result<Self>;
    pub fn digest(&self) -> &str;
    pub fn aggregate_alias(&self, guard: &SemanticRevisionRef) -> Option<&str>;
}

impl Engine {
    pub async fn plan_generated_sql_bound(
        &self, sql: &str, functions: &BoundQueryFunctions,
    ) -> Result<DataFrame>;
    pub async fn execute_parameters_bound(
        &self, sql: &str, parameters: Vec<ScalarValue>,
        options: QueryOptions, functions: &BoundQueryFunctions,
    ) -> Result<QueryExecution>;
    pub async fn execute_read_bound(
        &self, sql: &str, parameters: Vec<ScalarValue>,
        options: ReadOptions, functions: &BoundQueryFunctions,
    ) -> Result<ReadExecution>;
}
```

The native compiler lowering can use the aggregate instance's checked expression factory directly rather than needing SQL registration. That expression factory must be narrow (one exact rate argument), with the same bound object used by SQL registration. If exposing `aggregate_alias` and native expression construction requires a wrapper, use one engine-owned `BoundExactRateAggregate` returned by the set; do not expose mutable registry state.

Constructor validation checks physical decimal bounds, nonempty bounded revision references, valid selector/descriptor digests and alias uniqueness. The combined set rejects conflicting descriptor/alias identity rather than selecting the last entry. Its own number/byte bounds prevent an accidental unbounded registry; compiler work budgets are still enforced independently. Debug prints counts/digests, not selectors or request text.

The aggregate capability remains one canonical closed function, `semantic_unique_decimal_rate_v1`, marked LocalOnly. Request aliases are lookup names for specific immutable implementations, not new general executable capabilities. The independently reviewed DataFusion 55 implementation uses a fixed canonical `name()` with a unique `aliases()` entry per sealed instance. SessionState::register_udaf installs every alias to that instance Arc and then installs the canonical name, so each bound registration overwrites the canonical entry while preserving distinct alias entries. Register all bound instances first, then mandatorily restore a generic origin-free canonical implementation with no bound aliases. Both the restored canonical and every bound alias must be tested inside that same scoped session. Static capability lookup therefore sees the canonical LocalOnly name. The function implementation's equality/hash include its origin and descriptor, so optimizers cannot merge differently attributed guards. The reviewer verified this registration order against DataFusion 55; implementation regressions must enforce it. No prefix-based permission rule is introduced.

Neither alias text nor a static raw SQL function call grants origin. Ordinary Engine SQL methods use only origin-free functions. Only the bound adapter creates origin-bearing instances from compiler-owned bindings. The artifact's checked guard manifest is required for any later strong oracle match; library callers constructing their own engine adapter are not thereby claiming a compiler receipt.

## Session and cache invariants

Refactor a small internal planning helper to accept optional immutable bound functions. Keep the ordinary unbound entry points and behavior unchanged. The helper clones the existing SessionState, then adds only the request-local functions. It must not construct a default Engine/SessionState that loses registered catalog relations, views, configuration, analyzers, execution profile, federation optimizer/query planner or runtime environment.

`plan_generated_sql_bound` uses the same query-only SQLOptions and registered-unqualified-relation gate as `plan_generated_sql`. It unconditionally bypasses `generated_plan_cache` whenever bindings are nonempty. That existing cache is keyed by binding generation and SQL only; using it for bound instances would permit a cached function to retain a different guard origin. Bypass is the initial bounded implementation, including SQL without `$`; a later cache optimization must include the full sealed descriptor digest and preserve instance ownership. No silent reuse of the old key is permitted.

`execute_parameters_bound` creates one QueryContext from the supplied QueryOptions, applies existing parameter type hints/substitution, and routes through the existing `execution_frame` plus `execute_frame` path. Thread the optional function set into `execution_frame` so it is installed in both the ordinary session and the materialization session. Preserve materialization policies, dependency/source revisions, cache admission/age/refresh/bypass decisions and catalog/table registration. Preserve federation planner and checked arithmetic analyzer; source and snapshot reads still use the same budgeted physical task QueryContext.

`execute_read_bound` threads the same optional binding set through `ReadOptions` explanation/planning and the existing read-session execution path, preserving read consistency, after-commit constraints, commit receipt validation, snapshot provider rebinding, cache and cancellation. It must not silently downgrade Snapshot to eventual reads or route a bound read around admission. If a read mode cannot be supported coherently, fail explicitly before source reads and treat it as unfinished work; do not ignore the set. The initial implementation should support the existing compiled-artifact execute_read methods by threading the same helper rather than adding a parallel execution engine.

No engine-global register_udaf call is used. Dropping a planned frame or execution releases its scoped state. A compiled artifact retains immutable bindings for repeated execution, with independent accumulators and current effective budgets each time. Concurrent executions share definitions only.

## Compiler ownership and lowering

Add the closed terminal `convert_rate_exact` operation from the reviewed lookup design. Private bound state owns the profile reference, relation/field/policy references, canonical currency/date selector, literal amount, exact result controls, typed target currency meaning and structural guard descriptor. Derive guard identity from compiler traversal positions, not model IDs. Keep original request evidence per requirement unchanged and flatten through the existing visitors where applicable.

Lower one policy-filtered exact matching source into a global aggregate using the bound exact-rate instance. Its state counts all matching rows and retains at most one rate; final evaluation performs cardinality, null and positivity checks before returning the rate. Then call the existing decimal quantizer and project the result. SQL and native lowering obtain their function instance from the same immutable binding factory. Emitted SQL aliases identify that instance in the scoped registry; currency/date/amount remain typed artifact parameters. This is one input population and one execution, not a preliminary COUNT query.

CompiledQuery and CompiledGraph own their private `BoundQueryFunctions` and public safe guard manifests. Graph assembly combines sets with collision checks, preserving each occurrence's identity. Compilation backend schema checks use `plan_generated_sql_bound`; ordinary artifacts with no functions continue on the existing cache path. Execute/execute_authorized/execute_read/execute_read_authorized choose the matching bound engine entry point only when the private set is nonempty, after existing snapshot and current scope checks. Replay reconstructs bindings from validated proposals and pinned definitions; no deserialization of an executable function set is added. Bump pipeline, lowering and execution profile revisions for the new aggregate semantics.

SQL-only artifact serialization is not an execution receipt for these bound functions. In-process replay is supported through the ordinary typed replay bundle. Public SQL-only/PG execution of a bound lookup remains explicitly unavailable in this stage; no public success or provenance claim is made until the separately reviewed public lifecycle exists.

## Typed extraction and default report

Add runtime-owned `SemanticRevisionRef`, `SemanticDataOrigin` and `SemanticDataFailure` as described in the lookup design. The actual aggregate instance emits DataFusion external errors carrying its own origin. A shared extractor traverses concrete error/source wrappers with a bounded visit limit, recognizing the typed envelope without Display matching. A repeated/cyclic source chain or exhausted bound produces no trusted match. Rate validity checks in this instance preserve that same origin; no adapter guesses it from profile lists.

Preserve this envelope before typed compiler execution helpers convert engine errors to stable diagnostics. Add optional execution-failure metadata with serde defaults where existing CompileDiagnostic-returning APIs require compatibility. Planning-time extraction never supplies execution stage; only execute/stream adapters can do that. The evaluator recognizes errors from execute-start and collection, records successful compilation plus checked guard manifest before execution, and then records actual execution condition/origin. Resource limits and cancellation remain incomplete; unrelated failures remain generic.

Default report fields are present without `--debug-capture`: explicit compilation status, safe checked guard manifest, execution stage and typed failure. Keep raw SQL/transcripts opt-in. No default receipt is fabricated for older serialized reports. The strict expected outcome adapter may be added and unit-tested with synthetic generic artifacts, but no commerce companion metadata is migrated in this stage. Its match requires exact code/category/stage/profile/selector, successful compilation and actual guard membership; a profile merely present in compilation refs is insufficient.

## Test plan and bounded estimate

Engine tests:

- Count 0/1/2, identical duplicates, null/nonpositive rate, multiple partitions/merge order, exact result and genuine empty stream; errors occur at execution after planning succeeds.
- Same SQL text with two different bound origins executed sequentially and concurrently yields the respective actual origin; bound calls never populate/reuse the old generated plan cache.
- Two instances in one graph retain distinct identity, including the same profile with different selectors; fresh repeated accumulators and cancellation do not leak counts/origins.
- Scoped function aliases exist only in the bound frame/session; ordinary raw SQL cannot access an origin-bearing instance before or after execution. A canonical-name call inside the scoped session itself also remains origin-free after bound aliases are installed; each alias retains only its corresponding origin.
- Native and SQL exact type/nullability, policies before matching, registered-view/readonly gates, checked arithmetic analyzer and federation planner retained.
- Restrictive request/decoded/remote/timeout bounds fail genuinely; sufficient controls return the same exact result. Materialization bypass/age/refresh options and snapshot read-session rebinding are preserved.

Runtime/compiler/report tests:

- Typed envelope through DataFusion contexts/external and anyhow/source chains survives extraction; generic identical-text errors never match, and planning-phase typed errors are not execution outcomes.
- Closed selector/amount/result forms, access scope, evidence, stale profile/policy/catalog replay, parameters and target currency meaning.
- Default capture-off compiled status/manifest remains when a later stream fails; wrong profile, selector, guard/revision and missing origin are nonpassing.
- Missing/duplicate generic fixture expectations demand compile success and exact execution receipt; parser/provider/overflow/budget errors remain distinct. No dataset gold rewrite is needed for these tests.

Estimated bounded implementation: approximately 8–12 production files plus 5–7 focused test files, roughly 1,000–1,600 added/changed lines. The largest seam is threading an optional closed binding set through the existing parameter/materialization/read planning helpers; no second query engine is introduced. Suggested reviewable commits are (1) runtime envelope plus engine aggregate/scoped adapter/cache and execution tests, (2) compiler binding/lowering/replay/manifest, (3) evaluator capture-off extraction and strict generic matcher. All Cargo, formatter, linker/storage management and integration verification remain parent-owned. Public receipt/PG/HTTP changes and authored negative errata remain later independently reviewed work.
