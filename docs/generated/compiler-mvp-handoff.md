# Semantic compiler: MVP gaps and implementation handoff

Prepared 2026-09-29 against `e0a7b88` (`wip: Working on first proper compiler implementation`). The checkout was clean before this document was added. Recheck the current diff before editing: other agents may be working here.

## Objective and how to use this document

Finish a useful, trustworthy compiler MVP with the remaining budget, then work toward the full [architecture](semantic-compiler-architecture.md). **MVP completion is not full-design completion.** This document recommends a delivery boundary; it does not replace the architecture or claim the outstanding work is already done.

Read this document, then inspect the code named under each task. Implement and test one vertical slice at a time. Prefer extending existing contracts over introducing a parallel compiler. Do not spend the budget rebuilding features listed as implemented below.

Priority meanings:

- **P0: MVP release gate.** Supported queries must have correct semantics, explicit limits, usable integration, and evidence. A documented unsupported capability is acceptable; silently accepting a query with guessed semantics is not.
- **P1: next functional increment / full-design completion.** Valuable work that can follow an MVP with an explicitly narrower supported profile.
- **P2: measurement-driven or optional work.** Do only after correctness gates, or when measurements demonstrate a need.

Recommended MVP: the existing strict typed row/aggregate/lookup/window/calendar/set/composition profiles, exposed through current CLI/HTTP paths, with complete evidence for the supported intent forms and conservative backend behavior. Keep the SQL compatibility mode explicitly separate. Do not claim universal SQL support or proof that an LLM captured every phrase in a request.

## Already implemented: preserve and extend

See [implementation progress](compiler-implementation-progress.md), [typed rows](compiler-typed-rows.md), and [query graphs](compiler-query-graphs.md). The broad unchecked items in the progress document contain substantial implemented subsets; they are not all greenfield tasks.

- Immutable catalog snapshots; source/semantic/binding revisions; atomic publication and incremental reverse dependencies; source archives and YAML source maps.
- Exact, alias, lexical, field and metric search; scoped full/retrieved/auto context, dependency hydration, bounded expansion and reconsideration.
- Typed proposal binding, mandatory requirement coverage, exact literals, governed metrics and policies, separate SQL AST and direct DataFusion lowering.
- Aggregation, exact integer-component ratios, supported windows, pinned Gregorian calendar interpretation, existence/absence, checked dimension lookups, grouped lookups, output-stage filters, additive rollup restrictions, authored phrase-to-code mappings.
- Query graphs with UNION/INTERSECT/EXCEPT ALL/DISTINCT and independently aggregated fact composition. Composition has explicit group domain, null alignment, missing-group handling and complete-key validation.
- Bounded row compilation cache and coalescing; optional deferred providers; compiler records, Rust tracing, aggregate metrics, bounded capture and replay. **Graph replay already exists. Graph caching does not.**
- Typed CLI modes and HTTP compilation routes; in-process HTTP tests; Ossie foreign-key import with explicit authored-cardinality status.

Previously recorded validation: 136 offline tests across 19 binaries, followed by a passing focused graph replay test; Clippy passed across six packages with warnings denied. These are historical checkpoint results, not validation of future edits. No live database/model-service coverage is implied.

## Priority map

| ID | Priority | Deliverable | Why it matters |
| --- | --- | --- | --- |
| A | P0 | Explicit supported semantic profile and applicability checks | Prevent plausible but wrong answers |
| B | P0 | Complete graph request evidence and requirement coverage | Prevent clauses disappearing between intent and graph execution |
| C | P0 | Backend/execution validity for that profile | Preserve meaning across local execution, federation and replay |
| D | P0 | End-to-end fixtures and release audit | Establish what actually works and what must reject |
| E | P0 audit; P1 expansion | Diagnostic/privacy/resource-bound closure | Make success and failure trustworthy without unbounded work |
| F | P1 | Calculations over composed graph results | Enable useful questions such as ratios across separate facts |
| G | P1/P2 | Cache/lifecycle and adapter completion | Finish the broader design without delaying correctness |

Suggested order: establish the small acceptance fixture set in D, implement A → B → C, close findings in E, then run D's release gate. Take F next if budget remains. If a chosen launch fixture needs F, promote that slice to P0; do not claim the fixture works without it.

## A. P0 — Semantic applicability and supported-profile boundary

**Current gap.** The catalog contains fact/capability types and useful metric/grain contracts, but that is not a complete semantic type system. `MetricDefinition.unit` is a `Presence<String>`; source grain and dimension whitelists exist. Do not confuse storing a unit/fact with enforcing it. General unit/entity/time-grain compatibility, coverage applicability, and competing-definition resolution still need closure.

**Implementation steps:**

1. Write a concise supported-profile matrix in generated documentation: operation, accepted inputs/contracts, enforced checks, runtime obligations, and unsupported cases. Derive it from the binder and tests, not the architecture's wish list.
2. Audit the supported operations for result-critical metadata that is stored but ignored. Bind applicable restrictions before producing a compiled artifact. Reject unknown/conflicting required facts or unsupported restrictions with stable diagnostics.
3. Implement a small decidable applicability profile where needed: exact authored units/identities and explicit coverage/time-grain restrictions. Do not build general SQL predicate implication. A monthly-only source must not answer a daily request; a September-only source must not answer August merely because column names match.
4. Preserve plausible competing definitions through hydration/binding. Resolve only by an explicit applicable rule; otherwise return unresolved/clarification information. Do not let search rank decide semantic authority.
5. Include new semantic contracts in publication validation, canonical revisions, context rendering, binding and replay/cache identity. Preserve the distinction between missing, explicit null, authored, enforced and conflicting evidence.

**Start here:**

- `crates/semantic-catalog/src/{lib,governance,publication,snapshot,canonical}.rs`
- `crates/semantic-compiler/src/typed/{bind,context,lower}.rs`
- `crates/semantic-catalog/tests/{publication,snapshots,search}.rs`
- `crates/semantic-compiler/tests/{typed,retrieval}.rs`
- Architecture §§4, 6, 8.

**Done when:** supported examples compile and execute correctly; wrong grain, incompatible required units/identity, out-of-coverage requests and unresolved competing definitions fail explicitly. Mutation tests show that changing a relevant contract changes/rejects the binding rather than reusing stale meaning. Unsupported allocation, conversions and temporal relationships remain explicit capabilities, not guessed rewrites.

**Budget boundary:** no general unit-conversion engine, currency-rate service, arbitrary temporal join language, or allocation engine is required for the recommended MVP. A narrow executable profile plus explicit rejection is acceptable. Preserve these as full-design work where the intended supported profile requires them.

## B. P0 — Graph intent evidence and complete requirement accounting

**Confirmed gap.** `GraphQuery` has node `source_text`, but no graph-wide original-request/span contract. `GraphOrder` has slot/direction/nulls but no requirement identity; `limit` is a bare optional integer. Graph preflight currently rejects `CompileOptions.request_evidence` with `graph_evidence` because row evidence cannot cover a graph. A graph's leaf requirements do not prove coverage of composition, set semantics, final ordering or limits.

**Implementation steps:**

1. Define a versioned graph intent/evidence envelope. Reuse the row evidence validation rules where appropriate, but give graph operations and final ordering/limit their own stable requirement identities.
2. Retain the exact original request in the accepted intent/capture. Validate UTF-8 spans, nonempty span mappings, referenced identities, no orphan mappings, and disposition of every mandatory requirement.
3. Use unambiguous scoped identifiers for leaf requirements. Do not rely on concatenating arbitrary IDs with `/` unless escaping/collision handling is defined and tested.
4. Map graph-level requirements to actual checked operations and final output behavior. A mentioned field or copied source string is not substantive evidence that the requested operation exists.
5. Thread the envelope through model proposals, hydration/repair, structured HTTP, CLI outcomes, records and graph replay. Keep legacy structured graphs usable only with an explicit “no source-span validation” guarantee, consistent with row queries.
6. Update wire/pipeline versions deliberately. Support old inputs explicitly or reject incompatible versions; never silently reinterpret them. Update `prompt.txt` together with the Rust schema.

**Start here:**

- `crates/semantic-plan/src/{typed,graph}.rs`
- `crates/semantic-compiler/src/typed/{intent,graph,mod,replay}.rs`
- `crates/semantic-compiler/src/typed/graph/replay.rs`
- `crates/semantic-compiler/src/typed/prompt.txt`
- `apps/semantic-server/src/http.rs`, `apps/semantic-server/tests/typed.rs`
- `apps/semantic-cli/src/lib.rs`, `apps/semantic-cli/tests/commands.rs`

**Done when:** one request containing a multi-fact/set operation, ordering and limit retains all required dispositions through SQL/direct planning and replay. Missing spans, removed required operations, orphan IDs, conflicting IDs and unsupported versions reject. Graph replay still rebinds under the current caller's allowed scope. Neither records nor API documentation claims natural-language completeness from span checks alone.

## C. P0 — Conservative backend and execution-validity contracts

**Current gap.** Custom ratio/assert/sum functions already have local-execution guards. `federation.rs` uses named checks for those functions; this is not a general versioned capability/comparison contract. Full-design checks include current authorization, target mappings, function/capability versions and physical schema validity. Existing snapshot and deferred-provider drift checks cover part of that boundary.

**Implementation steps:**

1. Define the supported local DataFusion profile and connector capabilities actually needed by the MVP: exact numeric/overflow behavior, null comparisons and ordering, text collation, timestamps, aggregate/window behavior, parameter types and function versions.
2. Centralize semantic legality decisions rather than scattering new function-name exceptions. A small typed profile for the current operators is sufficient; do not build an unused plugin framework.
3. Push down only when the connector declares and implements equivalent behavior. Otherwise retain the operation locally or return a capability diagnostic if local evaluation is unavailable.
4. Audit both SQL and direct-plan execution boundaries for equivalent read-only, relation, policy and function restrictions. Audit how current host authorization reaches artifact execution; do not treat an old compile-time scope as perpetual authorization.
5. Pin/check relevant profile and binding revisions in accepted artifacts, replay and any cache keys. On incompatible schema/target/profile changes, reject or recompile explicitly; never silently rebind.
6. Integrate against the current Postgres connector. Inspect its current comparison/predicate support before changing shared engine behavior. Do not expand this task into unrelated connector work.

**Start here:** `crates/semantic-engine/src/{federation,compiler_functions,checked_sum,contracts,deferred,parameters,reads}.rs`; compiler `typed/{mod,lower,graph,replay,cache}.rs`; `crates/semantic-postgres/src/`; `crates/semantic-engine/tests/{compiler_functions,deferred}.rs`.

**Done when:** recording/fake-remote tests demonstrate safe pushdown and local fallback for each sensitive supported operation. SQL and direct execution agree with independently specified expected results, including overflow/null/text cases. Incompatible execution state cannot run a previously accepted artifact unnoticed. Live Postgres checks are separate evidence: record permission/environment limitations and continue local implementation/tests.

## D. P0 — Evaluation fixtures and honest release gates

**Current gap.** There are substantial deterministic operator tests, but no complete held-out interpretation/retrieval evaluation and release-gate package. Two backends agreeing can still share the same semantic mistake; planning successfully is not an answer-correctness test.

**Implementation steps:**

1. Create a small checked-in fixture suite with: catalog and rows, original question, allowed scope/request clock, relevant and competing definitions, expected requirements, expected result rows or explicit rejection, and diagnostic expectations. Put fixtures beside existing compiler tests; generated reports belong in `docs/generated`.
2. Cover at least: governed metric + policy; billing/shipping roles; duplicate dimension keys; missing group versus present-null measure; composite-key and global-scalar composition; set duplicate multiplicities; aggregate versus window filter stage; calendar/DST boundary; wrong grain/coverage; ambiguous definitions; denied scope; and bounded context exhaustion.
3. Reuse existing operator fixtures. The composite-key and global-scalar graph cases deserve explicit verification; the current main composition fixture does not establish all variants.
4. Run deterministic proposal/binder tests separately from interpretation/retrieval evaluations. A scripted provider tests orchestration, not LLM accuracy. Label any live-model results by model/configuration and keep them optional when credentials/network are unavailable.
5. Compare retrieved context with a sufficient/full-context baseline using expected selected definitions and outcomes. Include distractors and a competing definition introduced by a catalog mutation.
6. Add focused mutation/boundary tests for policy, mapping, relationship, scope and semantic-contract changes. Verify bounded retries/cancellation and explicit unsupported outcomes, not only success paths.
7. Produce a requirement audit table: architecture section, implementation/files, validating tests, supported limitation, remaining work. Mark a requirement complete only with evidence. Record correctness failures as release blockers.

**Start here:** `crates/semantic-compiler/tests/{typed,retrieval,compilation}.rs`, catalog tests, engine tests, `apps/semantic-server/tests/typed.rs`, `apps/semantic-cli/tests/commands.rs`.

**Done when:** every claimed MVP capability has a positive example, relevant negative example, independently expected result, and public-entry-point coverage where applicable. All mandatory offline checks pass. Unrun live tests and unsupported capabilities are listed explicitly. No percentage-complete or “full design implemented” claim is made from a raw test count.

**Performance scope:** for MVP, run repeatable small cold/warm/invalidation and bounded-resource smoke measurements. Do not invent latency SLOs or treat the existing single-run million-field measurements as p95 evidence. Full percentile/memory/concurrency/shadow evaluations remain P1 unless measurements reveal an MVP blocker.

## E. P0 audit — Records, pass checks, privacy and work bounds

Existing records, capture and metrics are substantial. Audit gaps before adding infrastructure:

- Ensure every supported path records accurate terminal diagnostics, required dispositions, runtime obligations and work accounting. Graph-specific node/edge work and relational-stage fingerprint coverage need review.
- Verify structure, slot scope, output schema, policy/requirement preservation and analysis validity at trust boundaries. Add focused checks to the existing fixed pipeline. A generic pass scheduler is unnecessary for MVP.
- Inspect arbitrary user/model-supplied IDs as well as literal values before claiming records contain no raw request text. Use bounded safe references/digests for normal telemetry; keep exact sensitive evidence in explicit captures.
- Exercise graph expansion limits, cancellation, admission, cache retention and capture truncation. A truncated debug capture must not change a query outcome or be described as a complete replay.
- Report runtime uniqueness obligations as pending until execution checks them. Preserve same-query duplicate checks; authored cardinality is not proof.

**Start here:** compiler `typed/{mod,metrics,capture,graph,lower}.rs`, `typed/lower/lookup.rs`, server admission/metrics tests. Architecture §§9.4, 15.

**Done when:** no discovered correctness/privacy/boundedness failure remains on an MVP path. More elaborate versioned analysis ownership and comprehensive decision/pass records remain P1 where the narrow implementation does not yet satisfy the full design. Use ordinary Rust tracing; **do not add OpenTelemetry**.

## F. P1 — Calculations after fact composition

**Confirmed gap.** `GraphOperation` currently has only `Rows`, `Set` and `Compose`. A row leaf can calculate a ratio, but a graph cannot generally compute a new ratio from the outputs of two separately aggregated facts. For example, composing revenue and spend by customer does not yet expose a downstream “revenue / spend” calculation node.

Implement a narrow typed projection/calculation node over graph slots, with explicit output identities/types and ratio zero/null/precision behavior. Reuse existing exact ratio semantics; reject unsupported operand types rather than implicitly casting. Add a downstream filter only if the selected use case requires it. Preserve semantic unit/grain information from A, evidence from B, and versioning/replay/SQL/direct parity. Do not emit model-authored SQL expressions.

**Start here:** plan `graph.rs`; compiler `typed/graph.rs`, `typed/graph/compose.rs`, `typed/lower.rs`; engine `compiler_functions.rs`; model `prompt.txt`.

**Done when:** a two-fact ratio fixture aggregates each fact first, aligns complete keys, computes the ratio afterward, distinguishes absent from null groups, handles zero denominators explicitly, and sorts/filters by the derived output with identical expected results on both backend paths. No sum-of-ratios or average-of-averages rewrite is introduced.

## G. P1/P2 — Remaining full-design work after the MVP

| Workstream | Remaining work | Budget guidance |
| --- | --- | --- |
| Broader semantics | Rich semantic types/entity/time grain, coverage/alternative reasoning, governed conversions, temporal relationships, allocation and merge/finalize representations where supported | P1; add executable, tested profiles, not empty interfaces |
| Cache/lifecycle | Graph cache; dependency-aware reuse including negative and alternative lookups; context/analysis caches; cold-build coalescing audit; retention/concurrency guarantees | Conservative full-snapshot invalidation is already safe. Graph cache and narrower reuse are performance work, not prerequisites for semantic correctness |
| Deferred sources | Verify/wire deferred providers through real project/configuration entry points and selected dependency resolution | P1 unless the launch catalog requires this path; then promote the needed slice |
| Importers | Import additional executable metric/concept profiles only with meaningful grain, provenance and acceptance contracts | Existing unsupported Ossie expressions should continue to fail explicitly |
| Compiler internals | Complete versioned pass/analysis ownership, decision provenance, observation/IR compatibility policies | P1; avoid introducing frameworks without consumers |
| Evaluation/operations | Held-out model evaluations, shadow compatibility, repeatable p50/p95 cold/warm/invalidation/memory/concurrency/telemetry-overhead matrix and measured release thresholds | P1; do not present synthetic single-run timing as production evidence |
| Optional extensions | Embeddings/rerankers, additional SQL dialects, Substrait/interchange | P2, justified by measurements or a real consumer |

OpenTelemetry remains out of scope, including after MVP, unless the user changes that instruction.

## Working constraints and verification commands

- Read `AGENTS.md`. Edit generated docs only under `docs/generated`; ask before changing other documentation.
- Preserve concurrent work, especially Postgres and engine parameter/read/write changes. Earlier connector work landed in `d53b348`; the current baseline includes later compiler work in `e0a7b88`. Do not revert/stage/commit other work.
- No new agent/thread is necessary to implement this handoff; the user is launching the next agent.
- Restricted network/database permissions must not halt unrelated implementation. Record blocked integration checks precisely. Do not claim them passed.
- The user previously ran `cargo clean` to reclaim roughly 150 GB. Expect a rebuild if artifacts are absent; avoid broad repeated rebuilds or another clean without need. Do not edit affected sources while a test build is in progress.
- Use targeted tests while editing. Format touched Rust files without recursively rewriting unrelated modules, for example `rustfmt --edition 2024 --config skip_children=true <files>` after verifying the crate edition.

Targeted examples (run the tests relevant to the slice, not all commands after every edit):

```sh
cargo test -p semantic-compiler --test typed --offline
cargo test -p semantic-compiler --test retrieval --offline
cargo test -p semantic-catalog --test publication --test snapshots --test search --offline
cargo test -p semantic-server --test typed --offline
```

Final offline regression gate, matching the previous broad checkpoint and explicitly including the server graph/HTTP test binary:

```sh
cargo test -p semantic-catalog -p semantic-compiler -p semantic-engine \
  -p semantic-ossie -p semantic-server -p semantic-cli \
  --lib --test publication --test snapshots --test search --test source \
  --test typed --test retrieval --test compilation --test deferred \
  --test query --test compiler_functions --test import --test commands --offline

cargo clippy -p semantic-catalog -p semantic-compiler -p semantic-engine \
  -p semantic-ossie -p semantic-server -p semantic-cli \
  --all-targets --offline -- -D warnings

git diff --check
```

When finished, update the generated progress/audit documents with changes, exact tests run, unrun checks and remaining P1/P2 work. Declare MVP complete only against the agreed supported profile and P0 evidence. Declare the full architecture complete only after its separate requirement audit has no outstanding mandatory work.
