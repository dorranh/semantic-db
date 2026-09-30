# Semantic compiler: full implementation and parallel delivery plan

Prepared 2026-09-29 against `32eef0bbbbdf5c9c6e91d40679875fcc41079048`.
This is an implementation plan, not a record of completed work. Source inspection
and planning were read-only; no Cargo build, tests, live model evaluation, or
database checks were run while preparing it.

The inputs are the [architecture](semantic-compiler-architecture.md),
[MVP handoff](compiler-mvp-handoff.md), [supported profile](compiler-supported-profile.md),
[progress record](compiler-implementation-progress.md), and
[MVP release audit](compiler-mvp-release-audit.md). The checkout was clean at the
initial inspection. Re-pin the baseline and inspect concurrent changes before
starting implementation.

## 1. Delivery boundary and starting point

Implement the remaining architecture in executable vertical slices. Preserve
the semantic correctness established by P0, not its APIs or data structures. The handoff's completion
update supersedes its earlier A–E gap descriptions; architecture §17.1 describes
an older starting point. Neither is a reason to rebuild working functionality.

The historical evidence is 157 passing selected offline tests, nine PostgreSQL
library tests, strict selected-package Clippy, formatting, and diff checks. It is
not current validation, full-workspace coverage, live connector equivalence, or
evidence of model interpretation quality.

### Greenfield strategy: optimize for implementation speed

The user's 2026-09-29 direction supersedes compatibility-preservation suggestions
in the older handoff/architecture. This compiler is greenfield:

- Break compiler Rust APIs, proposal/response schemas, IRs and record formats
  whenever that simplifies a correct design. Update in-repository callers,
  prompts, fixtures and examples in the same integration batch.
- Do not build legacy adapters, dual schema support, deprecation periods,
  persisted-artifact migrations or old-client compatibility suites. Bump the
  relevant identity and reject/recompile old captures and artifacts; clear
  affected caches. Revision checks remain useful for correctness, not a promise
  to support older revisions.
- Prefer replacing an awkward implementation directly over maintaining two
  compiler paths. Share or unify row/graph IRs when that reduces work. The
  existing separate SQL compatibility product path need not be redesigned as
  part of this feature, and typed failures must never fall back to it silently.
- Contract checkpoints are short coordination notes and compilable types, not
  API stabilization projects or approval ceremonies. Revisit them as evidence
  changes; one owner updates all affected code.
- P00/P01 are a short inventory and fixture seed, not a requirement to finish a
  comprehensive audit/evaluation framework before shipping P02. Grow the ledger
  and fixtures alongside implementation. Reuse existing tests and helpers.
- Use the two implementation lanes wherever ownership permits, combine small
  adjacent packages, and omit abstractions with no immediate consumer. Run
  targeted correctness checks per batch and broader checks at meaningful
  milestones. Documentation/report consolidation happens at those milestones.

Correct expected results, fail-closed semantic checks, runtime authorization,
bounded execution and reproducible verification remain mandatory. Compatibility
with existing unrelated runtime/connector behavior still matters; greenfield
compiler changes do not authorize reverting another feature's work.

Source inspection confirms these extension points:

| Area | Existing foundation | Remaining boundary |
| --- | --- | --- |
| Catalog | Snapshots, revision separation, incremental publication/search, source fidelity, fact/capability states | Rich entity/grain/type contracts, executable concepts/functions/conversions, typed view applicability and broader dependency tracking |
| Binding | Governed aggregates/ratios, policies, exact applicability, role-specific checked lookups | General semantic compatibility, supported multi-hop/temporal/allocation profiles, advanced metric state |
| Query graphs | `GraphOperation::{Rows, Set, Compose}`, graph evidence and replay | Calculations and filters after composition; graph caching; shared analyses |
| Relational lowering | Project-owned row plan and checked graph nodes, SQL AST and direct adapters | Versioned pass contracts, owned analyses, lineage and rule-level preservation evidence |
| Cache | Engine-owned `CompilationSession`, bounded/coalesced row artifacts, full-snapshot keys | Graph artifacts, context/analysis reuse, positive/negative/alternative lookup dependencies |
| Runtime | Versioned MVP profile, conservative PostgreSQL subset, scope checks, deferred backend | New capability implementations, full target/binding validity, production deferred-source wiring |
| Observation | Compilation records, requirement dispositions, digests, tracing, metrics, bounded captures | Structured diagnostic locations/actions, decision/pass records, comprehensive lifecycle accounting and stage replay |
| Evaluation | Independent MVP expected rows/diagnostics and deterministic provider tests | Held-out interpretation/retrieval evaluation, repeated scale/load measurements, shadow rollout and measured gates |

The architecture has three kinds of remaining obligations:

1. **Required architecture contracts:** correctness boundaries, source fidelity,
   versioning, bounded lifecycle, evidence, verification, and evaluation. These
   must have implementation and evidence before claiming completion.
2. **Executable capability profiles:** conversions, advanced metrics, temporal
   relationships, allocation, calendars, and richer expressions. Implement the
   concrete profiles below; reject variants beyond those profiles explicitly.
   An empty interface or a rejection-only test does not complete an affirmative
   capability deliverable.
3. **Conditional extensions:** embeddings/reranking, additional SQL emitters,
   external interchange, assumption-based acceptance, live value lookup, and
   durable identity migration. Resolve each through an explicit decision gate.
   A justified deferral is recorded separately from implemented work.

General SQL equivalence/predicate implication, a new optimizer, natural-language
writes, autonomous profiling, and a universal conversion engine remain outside
the design. OpenTelemetry remains excluded by the handoff, including its exporter
and GenAI convention mapping: implement the observation responsibilities with
ordinary Rust `tracing` and compiler-owned records. Do not silently interpret
architecture §15 as overriding that explicit constraint.

## 2. Team, ownership, and the single-verifier rule

Use the four available agent slots as follows. These are roles, not four new
user-facing chats.

| Role | Responsibility | Restrictions |
| --- | --- | --- |
| Coordinator (root) | Scope, requirement ledger, dependency order, file leases, contract decisions, integration, release report | Does not run Cargo while the verifier owns the build lane; owns shared integration edits |
| Implementer A | One bounded work package with an explicit file lease | Writes implementation and focused tests; no Cargo/build/test/lint processes |
| Implementer B | An independent bounded package, fixtures, or integration adapter | Same restriction; never edits A's leased files |
| Verifier V | Persistent build marshal, independent oracle review, targeted and release checks, evidence ledger | Sole runner of Cargo, Rust compilation, tests, Clippy, benchmarks, and build-triggering tooling; does not repair production code in place |

Do not allocate a fifth active agent. Reuse A and B between waves. Keep V's
context focused on contracts, changed files, expected results, and prior failures.
Implementation agents can review each other's work, but only V certifies a gate.
If V must be replaced, transfer its queue, baseline identity, feature/target
configuration, outstanding failures, and logs before granting the new build lease.

### File leases and integration

Use one shared checkout and its existing target directory by default. This avoids
duplicating large DataFusion builds. A file has one writer at a time, even when
two packages affect different functions in it. Directory-level ownership is
preferable for new modules and test binaries.

The coordinator initially owns these integration hotspots:

- `crates/semantic-plan/src/{typed,graph}.rs` and shared catalog contracts.
- `crates/semantic-compiler/src/typed/{mod,bind,lower,graph,context}.rs` and `prompt.txt`.
- Workspace manifests, lockfile, shared test helpers, and existing large test files.
- Public HTTP/CLI response contracts and generated progress/audit summaries.

Temporarily lease a hotspot to exactly one implementation package when needed.
Prefer separate modules such as `graph/calculate.rs`, `analysis.rs`, or
`lookup_dependencies.rs` and separate integration test binaries. These are
proposed files, not claims that they already exist. The coordinator connects
exports and call sites after the package interfaces have been agreed. Do not
perform a broad module refactor merely to manufacture parallelism.

Contract checkpoints briefly specify the types, versions, diagnostics, semantic defaults,
capability checks, evidence identities, and affected cache/replay keys. Land the
small contract change before dependent workers start. These interfaces may change
again; update their consumers together. Never have agents invent
incompatible versions of the same enum in parallel.

### Verification transaction

1. Authors submit a verification request containing package ID, changed files,
   prerequisites, fixture expectations, proposed commands, new test targets,
   feature requirements, and remaining limitations. Changes are **ready for
   verification**, not complete.
2. The coordinator integrates compatible ready packages and announces a checkout
   freeze. All agents acknowledge that writes and formatters have stopped. Pause
   background format-on-save/build tools as well. Other active work in this
   checkout must be accounted for before the freeze is valid.
3. V records HEAD, tracked diff and untracked source/test content identities,
   toolchain, features, target configuration, and the command queue. HEAD alone
   does not identify uncommitted work. Use a sorted content manifest/digest of
   build inputs, excluding build outputs and verification logs.
4. V runs one sequential command queue. Batch compatible ready changes so shared
   dependencies build once. Cargo's target lock is not a source-tree lock.
5. V verifies the source manifest is unchanged and returns command exit status,
   passed/failed/ignored counts, expected-versus-actual findings, log paths, and
   exact source identity. Mutation during a build invalidates that result.
6. The coordinator releases the freeze and routes failures to the file owner.
   Rerun the failed/affected gate after a fix, followed by the wave gate. No agent
   edits while the corresponding build is still running.

During a freeze, A and B can inspect code, design the next slice, or review
fixtures in messages. They cannot write even unrelated Rust files in the shared
workspace. If sustained build times justify independent authoring, explicitly
arrange isolated worktrees and one stable integration checkout for V; authors
still do not build. Reuse managed worktrees, account for their existing work, and
integrate serially. Do not share one target directory among concurrent Cargo
processes or create a target directory per agent.

No `cargo clean` as a routine recovery step. Avoid `cargo build`, `cargo check`,
and `cargo test --no-run` immediately before a test command that performs the same
compilation. V may run touched-file `rustfmt --edition 2024 --config skip_children=true`
before taking the verification manifest; final formatting is a check. Authors may
format leased files only outside a freeze.

## 3. Stages and parallel schedule

Waves are a default dispatch order, not estimates in days or rigid barriers.
Move a ready independent package forward when a lane is free. A dependent worker
starts after the specific interface/semantic prerequisite it consumes is ready,
not after every unrelated package in an earlier wave. Some waves contain several
small verification batches; package IDs need not correspond one-to-one to PRs.

| Wave | Implementer A | Implementer B | Coordinator checkpoint / V acceptance |
| --- | --- | --- | --- |
| 0 — Pin and inventory | P00 initial requirement inventory | P01 fixture seed using existing helpers | Reproduce baseline; record gaps and environment limits; continue the detailed audit alongside later work |
| 1 — First useful graph increment | P02 graph calculations | P03 catalog semantic types | Freeze graph-slot/type bridge first; then independent compiler/catalog ownership; baseline + each new slice |
| 2 — Trust boundaries | P04 pass/analysis contracts | P05 intent/outcome/parameter contracts | Sequence shared plan enums before parallel internals; version and malformed-input gates |
| 3 — Meaning and discovery | P06 applicability, concepts and views | P07 selection/completeness | Contract for new fact families first; binder and context/search owners remain separate |
| 4 — Runtime and basic reuse | P08 deferred-source integration | P09 graph cache/lifecycle | Engine/source ownership versus compiler cache ownership; coordinator owns HTTP integration |
| 5 — Advanced semantics, first pair | P10 metric state and richer windows | P11 multi-hop and temporal relationships | Shared grain/function/analysis contracts first; new lowering modules owned separately |
| 6 — Advanced semantics, second pair | P12 conversions/calendars | P13 allocation and fan-out | P10/P11 accepted; serialize binder/lowerer integration; independent modules and fixtures |
| 7 — Evidence and backend closure | P14 decision records/replay | P15 backend/execution validity | Share versioned decision/capability envelopes first; engine and observation modules then separate |
| 8 — Precise reuse and ingestion | P16 dependency/context/analysis caches | P17 importer and public API completion | Lookup-dependency contract first; import adapter and cache ownership independent |
| 9 — Measured quality | P18 held-out evaluation/shadow | P19 scale/load/observation benchmarks | Both use P01; V runs model/database/load jobs serially as needed, never simultaneously with timed benchmarks |
| 10 — Release | Fix assigned gate failures; conditional P20 only if justified | Independent coverage review and release fixtures | P21 architecture audit, workspace/feature gates, rollout/rollback decision |

This ordering deliberately delivers post-composition calculations early. P03
must provide a minimal slot semantic-metadata contract before P02 is accepted;
it need not finish every future catalog type before P02 starts.

Not every package is on one critical path. The main semantic path is
P03 → P04/P05 → P06 → P10/P11 → P12/P13 → P15 → P17 → P21.
The reuse path is P02/P04 → P09 → P16 → P19. Evaluation scaffolding starts at
P01 and accumulates cases in every wave; only its final live and release runs
wait for P18/P19. Start useful read-only design for later packages during builds.

P14, P15 and P17 are final closure packages, not permission to defer records,
backend checks or current public callers until the end. Each semantic slice adds
the minimum required integration and evidence immediately: local capability and
function registration, SQL/direct correctness, safe pushdown or local fallback,
artifact/cache invalidation and working current callers. P08 can start after its
P03 binding contract; P14 work can start after P04/P05. Capture an initial host
performance baseline using existing scale scaffolding early, and run a small
held-out model pilot once the relevant protocol is usable, if its environment is
available. This exposes design problems before the final measurement waves.

## 4. Work packages and acceptance contracts

Every package also satisfies the common checklist in §5. Paths below are
repository-relative ownership boundaries; source additions are proposals.

### P00 — Baseline and requirement ledger

**References:** handoff A–E completion update; architecture §§1–19. **Dependencies:** none.

Audit each normative architecture clause into a generated ledger with stable ID,
section, existing implementation, gap, owning package, prerequisite, positive and
negative evidence, public entry point, limitation, and status. Use `implemented /
partial / planned / conditional / excluded / blocked`, never a percentage based
on test counts. Check source and tests before marking an obligation implemented.
Reconcile stale narrative with the supported profile without rewriting history.

**Owns:** `docs/generated/compiler-full-requirement-audit.md` and verification log
index; V owns command evidence. **Gate:** cleanly identified starting revision,
baseline outcomes reproduced or discrepancies explained, initial architecture
section-to-package mapping and conditional decisions visible. Expand the
clause-level evidence ledger alongside features; complete it at P21. An inherited correctness failure
blocks dependent feature work until repaired.

### P01 — Independent fixtures and evaluation scaffolding

**References:** handoff D/G; architecture §16. **Dependencies:** P00 scope inventory.

Extend the existing MVP fixtures with a versioned case format: catalog/source
rows, exact question, request clock/scope, required and competing facts, acceptable
interpretations, expected requirements, result rows or diagnostics, and profile.
Separate deterministic proposal tests, retrieval-label tests, and live model
evaluations. Preserve dataset splits by domain and phrasing; do not tune on the
held-out set. A small runner can live beside compiler tests; reuse existing
facilities before creating a separate eval crate.

**Owns:** new `crates/semantic-compiler/tests/fixtures/full_*` assets and new test
support modules, without concurrent edits to `mvp_acceptance.rs`.
**Initial gate:** seed P02's distinguishing graph-calculation fixtures using the
existing runner/helpers, with independently reviewed expected results. Add each
later package's cases when that package starts, using at least two distinguishing
data arrangements for ambiguous semantic cases. Full coverage is a P21 gate,
not a prerequisite for P02. One scripted provider proves orchestration only.

### P02 — Calculations and filtering after graph composition

**References:** handoff F; architecture §§7–10. **Dependencies:** P01, minimal P03 slot contract.

Add a bounded typed graph projection/calculation node whose inputs are upstream
slot IDs. Start with exact Int64-component ratio semantics already implemented,
explicit zero/null behavior, derived output identity/type, and pass-through slots.
Add a typed downstream filter because the handoff acceptance requires filtering
derived outputs. Keep final ordering/limit after the calculation/filter stages.
Carry source grain, unit/entity metadata, lineage and evidence; unknown operand
meaning must not become a fabricated dimensionless unit. For known units, use
the P03 checked quotient rule or reject until it is available.

**Owns:** proposed `typed/graph/calculate.rs` and dedicated graph-calculation tests;
coordinator integrates `semantic-plan/src/graph.rs`, `graph.rs`, graph intent,
replay, prompt and wire version changes.
**Gate:** independently aggregate revenue and spend, align full composite keys,
then divide. Assert absent versus present-null, zero denominators, overflow,
scalar composition, duplicate aliases, cross-node scope, derived filtering/sort,
removed-evidence rejection and SQL/direct/replay parity. Never sum ratios or
average averages. Reject unsupported operands and graph cycles explicitly.

### P03 — Semantic types, identity, grain, and fact contracts

**References:** handoff A/G; architecture §§3–5, 8.3–8.4, 14. **Dependencies:** P00.

Introduce typed entity identity and scoped grain tuples; semantic value metadata
for unit/currency, enum, calendar/timezone, comparison profile, and reference
system; preserve missing/null/known/conflicting facts and their authority. Add
typed function signatures and conversion references only with an initial real
consumer, such as P02 checked ratio metadata. Keep physical Arrow types separate.
Record nullability, key/functional-dependency evidence and applicability scope.
Replace existing string units and field-name grains directly and update all
in-repo definitions; do not silently claim stronger evidence. Authored IDs survive renames; path-derived identities
remain rename-sensitive until an explicit migration facility is chosen.

**Owns:** `semantic-catalog` contract/canonical/publication modules and new tests;
shared `semantic-plan` surfaces integrated by coordinator.
**Gate:** publication rejects incompatible references and cycles by edge kind;
exact values/provenance survive normalization and canonicalization; source-only
versus semantic/binding edits have the intended revisions. Tests distinguish
same physical type/different entity, currency, timezone and comparison meaning.
This package defines enforceable foundations; later packages supply conversions.

### P04 — Relational verification and versioned analysis ownership

**References:** handoff E/G; architecture §§9, 14–15. **Dependencies:** P02/P03 interfaces.

Add a fixed pass registry with ID/version, pre/postconditions and analyses read,
preserved or invalidated. Own type, nullability, key/grain, lineage, volatility,
policy and requirement analyses by IR node plus revision. Use immutable outputs
or explicit invalidation, with no unowned mutable semantic annotations. Give
graphs the same verifiable boundary as row plans without requiring a wholesale
rewrite into one public IR. Preserve shared definitions as DAGs or memoized
expansions; volatile expressions may not change evaluation count.
Own the common typed scalar-expression boundary here: scoped input slots, exact
literals/parameters, checked casts, registered scalar calls, conditionals,
Boolean comparisons and null tests. Add only operators with immediate consumers;
P10 supplies checked multiplication/state expressions and P12 conversions. Binding
must check signatures and semantic compatibility before lowering. A function
registry alone does not implement expression validation or execution.

**Owns:** proposed `typed/analysis.rs`, `typed/verify.rs`, pass tests; coordinator
serializes `lower.rs`/`graph.rs` integration.
**Gate:** corrupt scope/slots/types, stale outer-join nullability, omitted policy,
misplaced aggregate/window filter and removed requirement mutations fail at the
correct boundary. Debug/tests verify every pass; release verifies trust
boundaries. Diamond/dense DAG fixtures bound expansion and check cancellation.
Pass failure cannot produce an accepted artifact.

### P05 — Intent, diagnostics, defaults, and prepared parameters

**References:** architecture §§7–8, 11, 13–15. **Dependencies:** P03 contract checkpoint.

Preserve semantic roles, Boolean/quantifier scope, output grain, unresolved terms
and structured alternatives independently from candidate physical bindings.
Keep existing span evidence and one-call interpretation. Add structured diagnostic
stage, requirement/object/source references, recoverability and next action,
with a separate safe model-repair representation. Distinguish clarification,
unsupported, unresolved, catalog failure and provider failure. Defaults for clock,
timezone/calendar/locale carry authored or caller provenance. Required prepared
parameters can remain unbound in a non-executable artifact; binding values reruns
type, coverage, source-choice and other value-dependent checks before execution.
Add explicitly requested bounded offset/fetch with requirement evidence and
nonnegative range checks; carry it through row/graph lowering and both adapters.
Do not promise deterministic pagination without a sufficient requested ordering.

**Owns:** intent/temporal/diagnostic modules and new tests; coordinator owns plan
wire enums, prompt, provider capability envelope and API response changes.
**Gate:** negation and aggregate scope survive repair; clarification consumes no
guessing repair loop; budgets distinguish calls/attempts/repairs/expansion. A date
parameter crossing coverage fails/rebinds explicitly. Unknown wire versions and
semantic variants reject; old compiler clients/fixtures are updated directly,
with no backward-compatible decoder required.

### P06 — Applicability, executable concepts, and authored views

**References:** handoff A/G; architecture §§3–4, 6, 8.5, 12.1. **Dependencies:** P03–P05.

Implement a documented decidable applicability algebra: exact identities/enums,
interval inclusion, compatible grain refinement and supported conjunctions.
Unknown implication remains unknown. Add executable concept/parameterized
definition expansion and declared alternatives/equivalences, with authority,
scope, revisions and acyclic dependencies. Typed view contracts preserve exact
SQL/dialect, output schema, lineage where sound, coverage and restrictions;
opaque views remain atomic and cannot be substituted without evidence.

**Owns:** new catalog applicability/concept/view modules and compiler binding
helpers; coordinator leases `bind.rs` for integration.
**Gate:** nested active-customer definitions preserve every governing restriction;
monthly/partial-coverage views cannot answer finer/outside requests; conflicts
retain all origins; a newly applicable alternative triggers clarification.
Prose/examples cannot override executable definitions or create authoritative
predicates. No general SQL theorem prover is introduced.

### P07 — Complete context manifests and bounded discovery

**References:** architecture §§5–6, 11, 13, 16. **Dependencies:** P03 and the minimal P06 fact interfaces, not all of P06.

Hydrate new entities, concepts, functions, view restrictions, inherited facts and
alternative sets. Separate interpretation from execution dependency closure.
Manifest fields identify included facts, detail level, conflicts, satisfied or
missing dependencies, searched scopes/completion, retrieval configuration,
capability profile and token/work accounting. Search with the original request
as well as extracted clauses; domain routing cannot exclude cross-domain facts.
Pack whole mandatory fact groups and reserve expansion/output capacity. Use the
intended tokenizer when available; otherwise record estimates and uncertainty.

**Owns:** compiler `context.rs` and catalog search, with exclusive leases separate
from P06 binder work.
**Gate:** field-only clues find wide relations; missing governing facts trigger
expansion/rejection; partial search yields unresolved, never proof of absence.
Alternative, nested-view, prompt-injection, dependency-diamond, reorder and
cross-domain fixtures retain scope and bounded work. Measure query-level complete
required-fact recall separately from per-field recall.

### P08 — Production deferred sources and catalog lifecycle

**References:** handoff G; architecture §§5, 10–11, 14. **Dependencies:** P00; P03 bindings contract.

Wire the existing deferred backend through real project/source construction,
embedding API, CLI and server configuration. Recorded physical contracts permit
offline semantic publication; missing physical evidence remains explicitly
unbound. Resolve only selected base/view dependencies, retaining deterministic
topological ordering. Audit existing queue-based view registration before
replacing anything. Bound metadata concurrency, coalesce cold resolutions, check
schema drift, and reclaim unreferenced provider/snapshot generations.

**Owns:** `semantic-engine/src/deferred.rs`, source/db construction and dedicated
tests; coordinator owns app configuration integration.
**Gate:** fake backends through actual config entry points record zero unrelated
provider resolutions; changed schema fails before execution; shared cold work,
cancellation, failed resolution retry and eviction are bounded. Publication/read
races never mix revisions. Metadata I/O is reported separately from row execution.

### P09 — Graph cache and conservative lifecycle closure

**References:** handoff G; architecture §§11, 14–15. **Dependencies:** P02/P04.

Extend `CompilationSession` to accepted graph artifacts using the same scoped,
engine-owned, bounded retention/admission model. Begin with full-snapshot
invalidation. Keys include graph/evidence/context, scope, acceptance and execution
profiles, pipeline and parameter-dependent inputs. Cache hits generate fresh
records and preserve current execution checks; capture policy/run IDs are not
semantic keys. Audit coalescing for cancellation of leaders/followers, admission
fairness, failure cleanup and oversized artifacts. Hits must still enforce
applicable per-request output/resource limits.

**Owns:** `typed/cache.rs` and dedicated cache tests; coordinator owns session/HTTP
wiring. **Gate:** row/graph/intent miss-hit parity, no cross-scope/engine reuse,
revoked authorization rejection, one shared cold compile, bounded retained bytes
and active entries, no hanging followers, and correct hit accounting. Do not
describe artifact serialization bytes as measured allocator/RSS consumption.

### P10 — Metric merge/finalize state and richer windows

**References:** handoff G; architecture §§4.2, 8.4, 9. **Dependencies:** P03/P04/P06.

Reuse and extend existing checked Decimal128 aggregation for architecture §12.2;
do not rebuild `checked_sum.rs`. Add versioned sufficient-state contracts:
weighted average from checked
weighted sum plus weight, average from sum/count, exact distinct recomputation or
explicit exact merge state, and a semi-additive balance with a declared time
selection rule. Add cumulative metrics and required supported window frames,
with deterministic tie, null, empty and rounding behavior. Bind additivity by
dimension and time; state schema/function versions participate in dependencies.
Reject incompatible state merges and unsupported frame nesting.
Implement the needed checked multiplication and conditional expressions through
P04's shared expression contract, including exact result typing, overflow and
null/zero semantics; model-authored SQL fragments remain prohibited.

**Owns:** new metric-state/lowering modules and engine aggregate implementations;
coordinator sequences catalog, binder and window integration.
**Gate:** partitioning/merge invariance where promised; unequal group sizes expose
average-of-averages errors; overlapping populations expose summed-distinct errors;
balance fixtures expose summing across dates. SQL/direct expectations include
empty/all-null groups, overflow, ties and window-filter placement. Approximate
sketches require a separate explicit approximation profile.

### P11 — Scoped relationship paths and temporal joins

**References:** handoff G; architecture §§4.2, 8.2–8.4, 12.3–12.4. **Dependencies:** P03/P04/P06.

Extend relation occurrence identities and typed relationship paths for bounded
multi-hop lookup/existence and repeated/self roles. Paths are selected by authored
semantics, never shortest-path cost. Add a narrow slowly changing dimension/as-of
profile: equality business keys plus half-open validity interval, explicit time
role/timezone, missing-match rule and at-most-one-match evidence/obligation.
Track directional multiplicity, functional dependencies and policy scope under
the actual predicate/comparison profile. Reject unsupported correlation.

**Owns:** new relationship/path/temporal lowering modules; coordinator integrates
shared relationship definitions and binder call sites.
**Gate:** overlapping validity intervals fail same-query uniqueness; gaps and
interval endpoints honor row preservation; billing/shipping/self roles cannot
collapse. Multi-hop fan-out, nullable composite keys, policy placement and bounded
path explosion have distinguishing fixtures. Constraints checked in a separate,
potentially stale query do not discharge execution obligations.

### P12 — Governed conversions and business calendars

**References:** handoff G; architecture §§7–9. **Dependencies:** P03/P06/P10/P11 as applicable.

Deliver successive slices: calendar grouping/bucketing with an explicit timezone
and observed-group domain; exact rational unit scaling with checked
overflow/rounding; authored business/fiscal calendar mappings with a pinned
revision; currency conversion through an authored rate relation with currency
pair, rate date/time basis, rounding, uniqueness and missing-rate behavior.
The rate lookup is part of authorized query execution, never a hidden live
service call during compilation. Broader numeric operands require registered
checked functions before their profile is enabled. Geographic reference systems
remain incompatible unless an actual checked transformation profile is selected.
Add an explicitly requested calendar spine/fill profile for missing periods,
with bounded generation and declared fill values; never infer zero-filled months
from a request that asks only for observed groups. Calendar filtering alone does
not complete grouping or fill support.
Use a typed, bounded `Values(schema, rows)` relational operator for calendar-spine
inputs where appropriate, validating every cell and its schema. This implements
the architecture's Values boundary without introducing a raw SQL escape hatch.

**Owns:** conversion/calendar modules and fixtures, plus exclusively leased engine
functions. **Gate:** currency/time/unit mismatches reject without a rule; rate
gaps/duplicates, zero/negative values, precision boundaries, fiscal year and DST
cases produce independent expected results. Reproduce architecture §12.2 with
checked decimal net revenue by UTC month and current billing region, including
unmatched customers and omitted empty months; separately test requested empty
period filling and reject substitution of current for historical region.
Changing a rule/rate definition or
calendar revision invalidates applicable plans; historical data reproduction
still requires data snapshots.

### P13 — Allocation and safe fan-out formulations

**References:** handoff G; architecture §§4.2, 8.4, 9.2–9.3, 12.3. **Dependencies:** P10/P11.

Add an authored bridge/allocation profile specifying source entity/grain, target
dimensions, weight measure, eligible population, denominator, null/zero behavior,
rounding and conservation obligations. Use pre-aggregation only under a checked
equivalence rule. Define runtime verification for required weight completeness
and uniqueness under the same read boundary; if unavailable, reject that profile.
Keep entity-level semi/anti formulations for questions that need no allocation.

**Owns:** allocation contracts/helpers and independent tests, with serialized
binder/lowerer edits. **Gate:** an order spanning two categories cannot duplicate
its total; unequal weights conserve the defined amount within the declared exact
rounding contract. Missing/duplicate bridge rows, incomplete weights, overlapping
memberships and policy-filtered denominators reject or follow explicit semantics.
No generic many-to-many join is accepted merely because SQL can execute it.

### P14 — Decision provenance, stage replay, privacy, and observation

**References:** handoff E/G; architecture §§9.4, 13–15. **Dependencies:** P04/P05; extend as new passes land.

Implement versioned rule decisions linked to requirements, scoped facts/source
refs, input/output artifacts, alternatives, preconditions and analyses reused or
invalidated. Complete minimal terminal records and runtime-obligation states;
compile records remain pending until a correlated execution discharges checks.
Extend bounded capture with optional before/after IR and first-divergence stage
replay; expired/redacted/version-incompatible captures report exact limitations.
Record queue/stage/pass work, metadata, cache invalidations and dropped capture
without high-cardinality metric labels or secrets. Use async-safe tracing parents,
fresh execution correlation and bounded optional local output queues.

**Owns:** observation/capture/replay/metrics modules and focused tracing tests;
coordinator owns record wire changes.
**Gate:** disabled/normal/debug modes have identical semantic outcomes/digests;
slow/failing consumers cannot block compilation indefinitely; concurrent spans
retain parents; usage remains unknown when unavailable. Sensitive IDs, request
text, literals, physical paths and credentials do not leak through diagnostics or
Debug formatting. Hashing low-entropy literals is not anonymization. Retained
stage replay matches decisions/IR under recorded versions and current scope.

### P15 — Backend capability and execution-validity completion

**References:** handoff C/G; architecture §§8.3, 10, 14. **Dependencies:** P04 and each advanced capability enabled.

Expand the existing execution profile and function registry for actual operators
from P10–P13. Version argument/result/null/overflow/rounding/volatility contracts
and connector implementations. Check current authorization, source/schema/binding
revision, target mappings, comparison/function/capability versions and required
read consistency at SQL/direct/read boundaries. Preserve local fallback only when
the local engine implements the exact profile. Complete SQL artifact parameter
schema, target identity, expected output and validation metadata. Engine SQL is
the default; standalone remote SQL requires complete target bindings.

**Owns:** engine profile/functions/federation and PostgreSQL capability modules;
coordinator owns compiler artifact integration.
**Gate:** fake-remote tests check safe pushdown/fallback for every added operation;
independent SQL/direct rows include collation, nulls, decimal and timestamp cases.
Stale schema/target/function/profile and revoked scope fail explicitly. Add a
separate live PostgreSQL conformance lane for claimed remote equivalence. Existing
ClickHouse compatibility receives regression coverage; expanding its typed
semantic capability profile is a conditional P20 deliverable.

### P16 — Dependency-aware reuse and context/analysis caches

**References:** handoff G; architecture §§5–6, 9.4, 14. **Dependencies:** P04/P07/P09/P14.

Track scoped positive and negative lookups: name resolution, alternative sets,
effective annotations, policy/vocabulary and definition dependencies. Compare
dependency-result revisions across publications; preserve downstream reuse only
when the derived result is unchanged. Ranked searches retain conservative
index/namespace-generation dependencies. Add bounded rendered-context and
analysis caches with single-flight construction; add normalized-definition reuse
where measured normalization costs justify it. Interpretation caching, if enabled,
keys exact request/conversation/clock/context/prompt/provider configuration.
Never key meaning by embedding similarity. Distinguish pinned artifact replay,
re-lowering, and latest-catalog re-interpretation.

**Owns:** dependency/cache modules and new publication/cache race tests; coordinator
integrates search and binding lookup hooks.
**Gate:** unrelated changes reuse safely; a newly inserted name, formerly missing
object, synonym, policy or competitor invalidates relevant results. Parameter
coverage, tenant/scope, renderer/function/acceptance revisions invalidate correctly.
Concurrent publish/read/evict races remain coherent; retained generations and
active locks are bounded. Compare measured work saved with bookkeeping cost before
replacing conservative full-snapshot behavior as the default.

### P17 — Executable import profiles and public integration closure

**References:** handoff G; architecture §§3–5, 13–14, 17. **Dependencies:** implemented contracts P03–P15.

Extend Ossie and application-authored import profiles only for definitions whose
grain, aggregation, authority and provenance are available. Preserve exact source
expression plus checked AST; unknown extensions block the affected executable
scope. Import advanced metric/concept/relationship variants incrementally with
positive executable fixtures. Keep explicit spec/adapter/normalization versions.
Expose new typed row/graph/intent outcomes, parameters, diagnostics, capabilities,
deferred options and replay through embedding API, HTTP and CLI. Maintain explicit
SQL compatibility mode as a separately labeled product path. Update all current
compiler callers directly; document rejection/recompile behavior for old compiler
artifacts rather than building migration support.

**Owns:** `semantic-ossie`, `semantic-db`, app adapters and new entry-point tests;
coordinator controls shared response/schema/prompt integration.
**Gate:** import → publish → retrieve/bind → SQL/direct expected rows → public
response works for each advertised profile. Unsupported opaque expressions do not
become executable through ignored fields. Current clients and prompts have
matching fixtures; obsolete versions fail explicitly. Minimal/no-default-feature and all-feature
builds do not hide broken reexports or optional dependencies.

### P18 — Held-out quality evaluation and shadow rollout

**References:** handoff D/G; architecture §§6, 15–16. **Dependencies:** P01/P07/P14/P17.

Compare existing full projection, full compact, retrieved compact and independently
curated sufficient context under the same recorded model configuration. Use
representative slices when full context does not fit; do not call truncation a
full baseline. Score requirements/alternatives, complete required-fact recall,
correct accepted answers, incorrect accepts, unnecessary refusal/clarification,
expansion recovery and cost per correct resolution. Include prompt injection,
misleading aliases, unseen domains/phrasing and catalog mutations. Repeat cases
to report sample size, variability and uncertainty. Deterministic replay is a
separate suite from model re-evaluation.

**Owns:** evaluation runner/datasets and generated reports. V owns executions.
**Gate:** numerical release thresholds and sample protocol are recorded before
scoring the held-out release run; critical policy/scope/fan-out/ambiguity false
accepts block rollout. Run selection policies in shadow on an authorized sample,
with bounded extra calls and retention. Keep full compact fallback where it fits
and a configuration rollback. Without credentials/approved data, the harness can
be complete, but live quality and rollout gates remain unverified.

### P19 — Repeatable scale, concurrency, and performance gates

**References:** handoff G; architecture §§5, 11, 15–16. **Dependencies:** P08/P09/P14/P16.

Extend `tests/performance` with compiler workloads rather than treating its
existing ClickHouse workloads as compiler coverage. Use deterministic interpreters
to isolate host costs. Cover 100/1,000/10,000 relations, at least one million
fields overall, a 10,000-field relation queried for three fields, chains/diamonds/
dense hubs, large prose/dictionaries, cross-domain dependencies and mutations.
Measure cold/warm/invalidation and concurrent publication/readers, admission,
cancellation, slow metadata, cache pressure and slow observation consumers.

**Owns:** new compiler performance harness, manifest changes integrated centrally,
and machine-readable results under `docs/generated`.
**Gate:** record seed, hardware/toolchain/profile, repetitions/warmup and raw
observations; report p50/p95/p99 with sample limits, queue/service/expanded-request
latency, work counts, real allocation/RSS measures where available, plan size,
provider calls, cache/snapshot retention and disabled/normal/debug overhead.
Separate index construction amortization and model cost. Assert architectural
properties with work counters: selected queries avoid unrelated fields, edits
touch real dependents, publication is coherent and limits are explicit. Do not
infer CPU time from summed spans or invent production capacity from one run.

### P20 — Conditional extensions and recorded decisions

**References:** handoff G; architecture §§10.3, 17–18. **Dependencies:** measured need/consumer.

| Decision | Implementation if selected | Evidence required; otherwise disposition |
| --- | --- | --- |
| Embeddings or reranking | Union candidate sources, retain authoritative hydration, version model/index/configuration | Improve total cost/latency or sufficient-context quality under the same incorrect-accept gates; otherwise defer |
| Additional SQL emitter / broader ClickHouse profile | Explicit target mappings, typed AST, capability profile, parameter conventions | Real consumer plus independent dialect/live execution conformance; otherwise remain unsupported |
| Substrait or external relational wire contract | Tested subset, extensions and compatibility/migration policy | Independent consumer; do not serialize engine-private plans as public semantics |
| `AuthoredAssumptions` acceptance | Caller-selected permitted declaration classes, versioned assumptions and execution/cache behavior | Named use case; strict remains default and model cannot relax it; otherwise strict-only is deliberate |
| Cross-rename identity migration | Explicit authored migration map and conflict validation | Actual rename persistence requirement; otherwise document path-derived identity behavior |
| Geographic transformation / additional calendars / correlation / recursion / grouping sets | One bounded typed profile with validation/lowering/termination as applicable | Concrete accepted use case and distinguishing results; otherwise explicit capability rejection |
| Live value discovery | Separately authorized, budgeted, fresh lookup interface with provenance | Explicit product decision; never introduce hidden row profiling in the compiler |

These decisions prevent “full architecture” from becoming an unlimited promise
of every SQL or semantic operation. They also prevent quiet omission: each item
receives an implemented, deferred-with-reason, or excluded disposition in P21.

### P21 — Final architecture audit and release

**References:** all; especially architecture §§16–18. **Dependencies:** all required packages and selected P20 items.

Reconcile the clause-level ledger with implementation, test names, exact verified
source identities and limitations. Update generated supported-profile, progress,
graph/typed guides, and release audit. Mark historical checkpoints as historical.
Run broad workspace/feature checks once on the final frozen tree, plus the
specific live/evaluation/performance gates required for the claims being released.
Review every exported profile and default against its evidence. Record rollback
configuration and current supported input/artifact versions. No legacy compiler
compatibility certification is required.

**Gate:** no outstanding mandatory architecture obligation; every enabled
capability has expected-result, rejection, public-entry, revision and runtime
evidence. Conditional deferrals and the no-OpenTelemetry exception are explicit.
An unavailable live environment yields a bounded offline release statement, not
a full-architecture or production-quality completion claim.

## 5. Definition of done for every vertical slice

- Its authored semantics, accepted inputs, defaults, unsupported cases and stable
  failure outcomes are written before implementation; V reviews the expected
  rows/diagnostics independently from production lowering.
- Publication, canonical revisions, search/hydration, binding, relational checks,
  SQL/direct adaptation and runtime obligations agree for the supported profile.
- Intent/evidence and mandatory policy requirements survive every transformation;
  no typed failure falls back to unconstrained model SQL.
- Tests distinguish competing meanings using nulls, duplicates, missing groups,
  boundary values or multiple datasets. Add focused property/mutation tests where
  they test semantic invariants; avoid tests that merely restate implementation.
- Wire/pipeline/function/profile versions, replay, cache keys, authorization and
  parameter-dependent applicability are updated together.
- Cancellation, expansion/output limits and observation privacy cover the new
  work. New telemetry cannot affect meaning or become unbounded.
- Public structured and model-proposal paths are covered where advertised; a
  model script is not counted as evidence of interpretation accuracy.
- V's relevant checks passed against the unchanged source identity; generated
  audit entries list tests run, unrun checks, exact limitations and follow-up IDs.

## 6. Verifier command lanes

Commands are templates to execute during implementation, not results from this
planning task. Use repository Rust 1.94 / edition 2024 and the committed lockfile.
Confirm available targets/features at P00 and maintain this list as new test
binaries land. `--offline` prohibits dependency downloads; it does not prevent a
test from opening sockets or using Docker. Classify tests by behavior as well as
Cargo flags. Save complete logs outside source inputs, with summaries in the
generated verification ledger.

### Baseline and targeted batches

V runs the recorded selected baseline once, including acceptance and PostgreSQL:

```sh
cargo test -p semantic-catalog -p semantic-compiler -p semantic-engine \
  -p semantic-ossie -p semantic-server -p semantic-cli \
  --lib --test publication --test snapshots --test search --test source \
  --test typed --test retrieval --test compilation --test deferred \
  --test query --test compiler_functions --test import --test commands \
  --test mvp_acceptance --locked --offline
cargo test -p semantic-postgres --lib --locked --offline
```

During a wave run only the relevant existing and newly registered test targets,
then affected-package Clippy at its integration checkpoint. Examples:

```sh
cargo test -p semantic-compiler --test typed --test mvp_acceptance --locked --offline
cargo test -p semantic-compiler --test retrieval --locked --offline
cargo test -p semantic-catalog --test publication --test snapshots --test search --locked --offline
cargo test -p semantic-server --test typed --locked --offline
cargo test -p semantic-cli --test commands --locked --offline
```

Append the actual new graph/types/cache/analysis test binary names to V's queue;
an old hard-coded list cannot establish coverage of new packages. `cargo test`
already builds what it tests. Use `cargo build` separately for a deliverable or
feature surface not exercised by the chosen tests, for example:

```sh
cargo build -p semantic-db --no-default-features --locked --offline
cargo build -p semantic-db --no-default-features --features compiler --locked --offline
cargo build -p semantic-cli -p semantic-server --locked --offline
```

Exercise source/postgres/clickhouse feature combinations when their paths change;
do not multiply all possible combinations without a coverage reason.

### Integration and final offline/static lane

After a coherent multi-package milestone, rerun the selected regression lane plus
all newly relevant test targets. At final release, inventory workspace tests and
run the following in an environment that permits their local sockets/services:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
cargo test --workspace --all-features --locked --offline
git diff --check
```

These align with the broad workspace/features in `.github/workflows/ci.yml`,
adding offline dependency resolution. Missing cached dependencies are an
environmental blocker for that invocation, not a semantic failure; record and
resolve acquisition separately without removing `--locked` casually. If a
dependency change is intentional, assign one manifest/lockfile owner and verify
the resulting lockfile before returning to locked commands.

`crates/semantic-compiler/tests/openai.rs` uses local fake HTTP servers. Include it
in the loopback-permitted transport lane; it is not a live-model evaluation.
Socket restrictions can prevent the complete workspace command from passing.
Record the exact unrun/failed environment-dependent targets and continue the
no-socket gate rather than relabeling the selected gate “workspace passed.”

### Live and measurement lanes

Inspect actual test requirements and credentials before running service tests.
Never enable every ignored test indiscriminately. Existing targeted PostgreSQL
TLS CI commands include:

```sh
cargo test -p semantic-postgres --all-features --locked --test tls -- --ignored
cargo test -p semantic-sources --features postgres --locked --test postgres \
  connector_integration_postgres_tls_secret_options -- --ignored
```

These verify connector/TLS behavior, not every compiler semantic claim. Add
dedicated live profile conformance tests in P15. P18 live-model runs and P19
benchmarks have separate commands/configurations recorded by their harnesses.
V serializes timed workloads to avoid contamination by other builds, model runs
or database benchmarks. Use release-profile measurements where appropriate and
report the profile. No credentials, captured private prompts or source data enter
checked-in logs without an explicit retention policy.

## 7. Dispatch templates

### Implementer assignment

```text
Implement package Pxx from docs/generated/compiler-full-implementation-plan.md.
Baseline/source identity: <revision and integrated dependency gates>.
Owned files: <exclusive list>; shared integration owner: coordinator.
Contract: <types/versions/diagnostics/defaults and allowed capability profile>.
Acceptance: <independent fixture IDs, expected outcomes, required public paths>.
Preserve P0 semantic correctness, authorization, replay and boundedness.
This compiler is greenfield: simplify APIs/IRs freely, update in-repo callers
together, invalidate obsolete artifacts, and do not add compatibility shims.
Add focused tests, but do not run Cargo, rustc, Clippy, benchmarks or build tools.
Observe the coordinator's checkout freeze; stop all writes before acknowledging.
Return changed files, contract changes, proposed verification commands, limitations
and integration notes. Do not mark the package verified or modify others' files.
```

### Persistent verifier assignment

```text
You are the sole verifier/build marshal for this implementation effort.
Run one sequential queue only after the coordinator confirms a frozen checkout.
Record source identity including untracked inputs, commands, toolchain/features,
exit status and full logs; confirm sources did not change during verification.
Review fixture expectations independently; SQL/direct agreement alone is not an
oracle. Check the package's positive, negative, mutation and public-path coverage.
Choose the smallest sufficient gate, broaden at integration/release checkpoints.
Do not edit production code, accept weakened assertions to get green tests, run
cargo clean, or treat skipped live/environmental checks as passing.
Return pass/fail/blocked evidence and actionable findings to the coordinator.
```

### Coordinator's next action

Start P00/P01 with V reserved, re-establish the current baseline, and resolve the
minimal graph-slot semantic contract. Then dispatch P02 and P03 under explicit
file leases. Carry the requirement ledger and V's queue forward across waves;
do not launch all work packages at once.
