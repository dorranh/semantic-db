# Interpreter separation and usable compiler release plan

Status: scoped implementation integrated and selected checks green. Workspace all-target/all-feature, formatting and whitespace checks pass; 100 interpreter tests, eight numeric/demo tests and six server tests pass. Live ClickHouse remains blocked by Docker container creation timeout. Broad runtime/matrix verification limits are recorded in `compiler-release-checkpoint.md`. No implementation workers remain active.

## Objective and separation of responsibilities

Complete the deterministic path from structured intent through binding and query IR to executable SQL/direct plans for an explicit language contract. Every well-typed, semantically valid intent within that contract and configured resource bounds must compile. Invalid intents must receive a specific diagnostic. Valid plans may still fail explicit runtime obligations or encounter source failures; those are different from missing compiler coverage.

Natural language to intent remains probabilistic. The existing interpreter is sufficient to exercise the compiler and can improve separately. Live interpretation accuracy, mandatory phrase evidence, a new wording ledger, and proof of original-request completeness are not release gates for this work. Preserve existing evidence checks and honest explanations; never claim that successful deterministic compilation proves the model understood the question.

The release is driven by the declared intent language, not a growing list of demo questions. Demo questions are smoke tests of the completed compiler surface.

## Component boundary

Retain `semantic-plan` as the shared serializable intent contract. Add `semantic-interpreter` for language-model providers, prompts, context selection/retrieval orchestration, interpretation outcomes, model transcripts and bounded model repair. Narrow `semantic-compiler` to deterministic intent validation, catalog binding, compiler-owned checked query IR, SQL/direct lowering, compiler diagnostics and compiler artifacts.

The intent remains an untrusted proposed semantic query: it may propose exact catalog relation/field references, but the compiler authoritatively verifies them. Compiler inputs are intent, a pinned authoritative catalog, explicit semantic context and compilation limits. Compiler results are a checked artifact or deterministic diagnostics, with runtime obligations distinguished from compilation failure. The compiler must never call or depend on a model provider.

Both behavioral crates consume shared contract types; the compiler does not depend on the interpreter. A convenience workflow may live on the interpreter/application side and call compilation, then use diagnostics for bounded repair. No third orchestration crate is required. Keep the checked query IR compiler-owned rather than exposing construction of trusted plans through the shared intent crate.

Split model budgets, provider metadata and interpretation history out of compiler options/records. Keep compiler cancellation, work limits, semantic context, access scope and query replay independent of model transport. Preserve existing SQL compatibility functionality in an explicitly separate workflow without making the deterministic compiler depend on model-written SQL.

Verify the boundary structurally: compiler builds/tests require no interpreter or model-provider dependency; structured calls make no model calls; CLI/server callers retain their existing workflow through the new entry point. No new architectural abstraction is needed merely to move code.

## Required demo surface

Three concrete scenarios define product acceptance. The operation/type contract must support their general combinations, not special-case these questions.

| Scenario | Required useful queries | Independent expected outcome |
| --- | --- | --- |
| Local geospatial wells | Count/filter wells; northernmost/deepest; top N; combined basin/status/depth restrictions; grouped numeric summaries | 5 wells; Birch-4 at 56.25 north; Willow-3 at 4100 m deepest; active North Basin wells W-001/W-004; average depth 2870 m |
| ClickHouse drilling samples | Project/filter Float64 measurements; count and sum/average by well; order grouped results and limit | Well 1: 4 samples, distance 60 m, duration 13 min, nonnull mean load 40 kN. Well 2: 2 samples, distance 20 m, duration 4 min, mean load 30 kN |
| Authored relational joins | Inner samples-to-wells enrichment and aggregate by basin; left wells-to-summary retaining unmeasured wells; existence/absence | North: 60 m across 4 samples. South: 20 m across 2. Well 3 remains with null summary measures under left lookup and is returned by absence query |

Use existing `examples/geospatial` and `examples/clickhouse` fixtures. The ClickHouse model currently describes fields but does not declare the executable relationships needed by the typed join path; add those authored contracts and verify import→publication→binding. Declared keys alone do not prove uniqueness.

Join scope is explicit many-to-one equijoins for inner/left enrichment and grouped dimensions, plus semi/anti existence. Reuse existing lookup/related operations where they express that scope, including same-query uniqueness checks. Reject duplicate right keys rather than silently multiplying aggregate values. Verify unmatched/null keys and endpoint policies. Arbitrary many-to-many joins, non-equality joins and a new general join grammar are not included. This is a concrete useful join contract, not a claim of general SQL join coverage.

ClickHouse is a required existing backend, not a new connector project. Verify typed compilation and actual execution over a controlled local ClickHouse fixture. Test federation on/off for the accepted cases, or explicitly keep operations local when remote semantics are not proven equivalent. Record where execution occurs; reading ClickHouse data does not prove a join or aggregate was pushed down. Remote scans remain bounded and failures must not return partial success.

Exercise finalized `summary` measures without averaging per-well averages, and recombine partial `totals` correctly before treating them as per-well totals. Include exact provider-resolved types, including unsigned fields where these fixtures require them, in the audit rather than silently coercing them. The public Hacker News example can provide a bounded smoke query with explicitly requested ID range and limit; it is not the correctness oracle because its data changes.

## First deliverable: an executable language contract

Inventory every current row operation, predicate, literal, aggregate/window function and graph operation in `crates/semantic-plan/src/typed.rs` and `graph.rs`. Match each against binding, relational lowering, SQL emission, direct planning and existing tests. Audit implementation coverage, not the entire architecture document.

Produce one matrix recording:

- Operand physical types and semantic constraints, including units, grain and authored definitions where relevant.
- Legal input/output stages, result type and nullability.
- Legal combinations with filtering, grouping, windows, ordering, limits and graph consumers.
- Current implementation status at each deterministic stage and a named validating fixture.
- Whether a restriction is a necessary semantic rejection, an explicitly designed language boundary, or an implementation gap.

Do not define correctness as whatever today's binder accepts. That would bless accidental restrictions such as missing float sorting. Do not quietly exclude inconvenient existing advertised capabilities to declare completion. Genuine boundaries must be stated in the contract, with gaps and their estimated cost visible before implementation.

The grammar contains intentionally bounded operations, such as a specific cast target and calendar profile. Completing that grammar does not imply implementing all SQL, all casts, or every calendar. Within each agreed operation's domain, all valid type/stage combinations must work. Resource limits remain explicit.

## Baseline product requirement

The contract must cover ordinary single-relation projection; Boolean filters and null tests; grouping; count/min/max/sum and average; source/output ordering; and limits/pagination. It must include nullable text/Boolean categories, signed integers and Float64 measurements. Preserve existing decimal and temporal contracts. Average is an explicit language addition where the current representation cannot express the required result; it is not a reason to introduce arbitrary expressions.

Float support must be coherent across literals, parameters, comparison, ordering and numeric aggregation, with explicit handling of NaN, infinities, signed zero, nulls, empty groups and overflow. Float arithmetic is approximate; existing exact integer/decimal arithmetic must remain exact. Specify output types and comparison tolerances. Do not claim input values are finite unless that restriction is enforced.

Existing advanced row/graph operations are part of the inventory and regression obligation. Their declared domains must have end-to-end coverage. Any uncovered advertised case becomes a costed gap; substantial repairs may make the proposed budget insufficient. The required ClickHouse/join scenarios above are included; further advanced operations or backend families are not automatically added.

## Implementation approach

Reuse the existing bound query, relational IR and DataFusion execution paths. Centralize repeated scalar/type/result rules where needed so individual binders and emitters cannot disagree. Avoid a wholesale compiler or capability-framework rewrite.

Ensure every accepted bound operation has both SQL and direct lowering with consistent parameters, output types, nullability, requirement preservation and policies. Update artifact/execution-profile revisions where semantics change. Check all consumers of new literal or aggregate variants; do not accidentally broaden governed metrics, relationship keys or window profiles.

Once an intent passes binding, unsupported-operation failures in later compiler stages are compiler defects, not normal user ambiguity. Genuine backend capability restrictions belong in explicit target validation. Source access failures and checked runtime obligations remain separately classified.

## Verification and definition of done

1. A checked-in operation/type/stage matrix accounts for the entire agreed language. Each included cell has a fixture or coverage family; each excluded combination has a reason and rejection test.
2. Deterministic structured-intent fixtures compile through the public compiler API to query IR, SQL and direct plans. Compare both execution paths with independent expected rows and schemas, not only with each other.
3. Test composition systematically: filter then aggregate; group then order by output; order then limit; null predicates under AND/OR/NOT; supported window and graph consumers. Use table-driven/generated bounded combinations and representative boundary values rather than claiming exhaustive testing of infinite inputs.
4. Test invalid scope, incompatible types/units, illegal stages, missing metadata, lost policies, arithmetic boundaries and runtime obligations. A compiler that accepts incorrect intent is not complete.
5. Exercise request serialization, parameters, artifacts/replay and relevant cache behavior for changed types and operations.
6. Add public CLI/REPL tests using a deterministic provider that returns the intended structured proposal. This validates application wiring without making interpreter quality the acceptance criterion. Explicitly use the typed route; existing SQL-compatibility mocks are not evidence for it.
7. Run the wells demo as a small smoke test. Expected examples include count 5, northernmost Birch-4 at latitude 56.25, deepest Willow-3 at 4100 metres, active North Basin wells W-001/W-004, and average depth 2870 metres. A live model's incorrect interpretation is recorded separately from a compiler failure; it does not trigger unbounded prompt work.
8. At final integration, run the controlled ClickHouse and authored-join scenarios above through the real connector, plus equivalent local fixtures. Check independent rows, schemas, nulls, uniqueness failures and policies. If Docker/ClickHouse is unavailable, preserve offline evidence and report the live backend gate blocked; do not call that scenario complete.
9. Consolidate affected regression suites and required workspace checks at final integration. During implementation, run only targeted checks that resolve a concrete uncertainty. Record final commands, results, source revision and blocked checks concisely. Tests and a coverage matrix support a bounded completion claim, not a mathematical proof over every possible input.

Ship when the agreed language contract is closed across the deterministic pipeline. Report remaining gaps explicitly if it is not. Do not redefine release completion as a number of features implemented or demo phrases passing.

## Guidance required in every subagent handoff

This is a greenfield project on a feature branch. Optimize for a coherent final design and fast integration, not incremental compatibility. The user explicitly permits substantial refactoring and breaking internal API/IR changes. Update current callers directly; do not build migration layers, deprecated wrappers or old-wire-format compatibility unless a concrete requirement demands it. Preserve unrelated user work.

Agents may make coordinated large changes and temporarily leave the branch unbuildable while interfaces move. Agree on shared types and file ownership early, but do not require a full green test suite between extraction and implementation. Keep handoffs explicit about changed interfaces, unfinished consumers and known failures.

During implementation, use only cheap feedback that answers a real uncertainty: targeted compilation after major interface changes and a small focused test when behavior is difficult to reason about. Do not repeatedly run broad suites, freeze manifests after each edit, audit unrelated architectural requirements, or produce extensive verification reports. Author meaningful expected-result tests alongside the work where useful, but consolidate broad execution at the end.

One integration owner runs the complete required build and acceptance checks on the assembled tree, then routes concrete failures to the owning agent. Repeat only affected checks during repair, followed by the necessary final integrated verification. ClickHouse, join, policy, arithmetic and expected-result gates still apply before declaring completion. Deferred verification is a workflow choice, not permission to report untested behavior as working.

Include this guidance, the relevant ownership boundary and the final acceptance criteria in every implementation and review agent prompt. Keep the independent final review short and focused on correctness and missing contract coverage; no general cleanup or style campaign.

## Sol execution and budget

Planning began at 81% account usage. Implementation preflight reported 83%, so the original approximate target had been reached. The user explicitly authorized a fresh implementation budget of roughly 10% of the remaining allowance: 1.7 percentage points from an 83% baseline, with an approximate ceiling of 84.7%. At the 84% checkpoint, work paused for a budget decision. The user subsequently authorized continuing under the requested additional three percentage points, giving an approximate ceiling of 87%. The resume check reported 85%; do not silently reset the extension from that later value.

This is an approximate spending target, not an enforceable task allocation. The meter is account-wide and coarse; concurrent chats and tool/provider activity may have separate costs. API prices do not directly predict included subscription usage. Official guidance: <https://learn.chatgpt.com/docs/pricing>.

Use GPT-6 Sol (`gpt-6-sol`) with fresh, focused agent context. No recursive delegation. The expanded scope now includes crate separation, joins and live ClickHouse verification. It cannot honestly be promised within the original budget until the audit estimates those tasks and distinguishes missing general rules from substantial unfinished semantic families. The newly authorized implementation allowance remains the target, not automatic authorization to spend more.

Sequence:

1. **One Sol contract and separation audit:** produce the operation/type matrix, map crate dependencies and shared options/records, inspect ClickHouse fixture/model/relationship wiring, and estimate a finite closure batch. No implementation during this audit. Avoid an architecture-wide review.
2. **Budget decision:** if the complete agreed scope appears feasible, freeze it. If not, present the concrete gaps and cost tradeoff before edits; no quiet reduction to demo patches.
3. **Bounded crate extraction first:** a Sol worker establishes the new crate ownership and shared interfaces, moving interpreter behavior and updating callers. Use a targeted compile check where useful; a fully green suite is not a prerequisite for continuing. Hand off moved-file ownership before parallel work begins.
4. **At most two Sol workers after separation:** one owns compiler/type/lowering changes; the other owns catalog fixture wiring, independent expected-result fixtures and ClickHouse/application integration tests. Freeze interfaces and file ownership. Reuse workers for bounded repair.
5. **Sol integration/review:** validate the crate boundary and matrix against the final tree, run release checks, repair defects within the reserved budget, and produce one completion report under `docs/generated`.

Reserve approximately 10% of the remaining implementation budget for the audit, 55% for implementation and 35% for integration/verification. Check usage at handoffs. At the first observed percentage-point increase from the authorized implementation baseline, reassess the remaining work before dispatching more. Preserve work and report incomplete status if remaining allowance cannot plausibly fund completion. Meter polling cannot guarantee an exact hard cap.

## Out of scope

Improving probabilistic interpretation quality, enforcing new natural-language evidence requirements, proving wording completeness, joins beyond the declared inner/left/semi/anti contract, new graph families, spatial distance/CRS calculations, new calendar profiles, database backends beyond the existing required ClickHouse path, generalized SQL equivalence and unrelated optimization/observability work. Prompt/schema descriptions may be updated mechanically to match the completed compiler contract, without adding a new interpretation project.

Generated documentation stays under `docs/generated`. No edits to other documentation without permission.
