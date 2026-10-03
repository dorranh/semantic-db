# Semantic compiler MVP release audit

Prepared 2026-09-29 for the narrow compiler profile described in
`compiler-supported-profile.md`. This is an evidence index, not a claim that the
full architecture is implemented.

## Gate status

The additive release-gate suite is
`crates/semantic-compiler/tests/mvp_acceptance.rs`; its independently stored
expectations are in `crates/semantic-compiler/tests/fixtures/mvp_acceptance.json`.
All 10 acceptance tests pass. The combined offline regression gate passes 157
tests across 20 selected catalog/compiler/engine/Ossie/server/CLI binaries,
including 51 typed compiler tests, 10 acceptance tests, 8 catalog publication
tests, 6 retrieval tests, 3 in-process server tests and 5 CLI command tests.
Strict all-target Clippy also passes across those packages plus the PostgreSQL
connector. Formatting and diff checks pass.

The suite uses deterministic typed proposals for semantic correctness. Its one
scripted provider case establishes orchestration and accounting only. It does
not measure whether a live model understands held-out language. No live database,
network, or model-provider result is claimed.

## Requirement evidence

| Architecture area | Implementation evidence | Release-gate evidence | Supported limitation | Remaining work |
| --- | --- | --- | --- | --- |
| §§6–8: governed metrics and mandatory policy | `semantic-catalog` metric/policy contracts; compiler binder and lowerer | `governed_metric_and_row_policy_match_independent_expected_rows` checks direct and emitted-SQL rows | Exact authored metric, filter, unit and source-grain profile | Broader unit conversion, allocation and predicate implication remain unsupported |
| §§6, 12.4: repeated relation roles | Relationship and lookup binding | `lookup_roles_are_distinct_and_duplicate_dimension_keys_fail_closed` checks billing/shipping role separation and runtime uniqueness failure | Authored direct lookup relationships with explicit missing-match behavior | General multi-hop and temporal relationships remain outside the MVP |
| §§7–9: graph set semantics | Typed graph set node | `bag_set_operations_preserve_duplicate_and_null_multiplicity` independently checks `INTERSECT ALL` and `EXCEPT ALL` duplicate/null multiplicities through both execution paths | Explicit compatible output slots and set mode | No implicit coercion or inferred column correspondence |
| §§7–9: independently aggregated fact composition | Typed graph composition and authored relationship validation | `composition_covers_missing_null_composite_key_and_global_scalar_domains` checks present-null versus missing-zero, full composite keys, and empty-key scalar composition through both paths | Exact key types, complete authored key tuple, explicit group/null/missing behavior | Calculations after composition remain a separate feature |
| §§7–9: aggregate/window stage | Typed output-filter stages and window contracts | `aggregate_and_window_filters_execute_at_the_declared_stage` uses a result that changes if HAVING and QUALIFY are reordered | Supported aggregate/window functions and declared frames only | General window frames and unsupported nesting reject |
| §§6–8, 12: calendar and applicability | Pinned request context, half-open UTC bounds, metric temporal applicability | `calendar_day_uses_half_open_bounds_across_a_dst_transition` and `metric_time_grain_and_coverage_reject_outside_the_authored_contract` | Gregorian calendar, named timezone, Date32 or UTC timestamp fields, exact grain and coverage | Business calendars and temporal relationship alignment remain unsupported |
| §§4–6, 11: access and work bounds | Scope-aware retrieval/binding and bounded candidate search | `scope_and_context_bounds_fail_explicitly_and_scripted_provider_is_only_orchestration` checks `access_scope`, `search_limit`, and zero model calls on pre-provider exhaustion | Caller-provided relation allow-list and configured local limits | Broader admission/load testing remains P1 unless integration exposes a blocker |
| §§6–8: ambiguity | Binder preserves competing exact authored labels and applies explicit applicability | `applicability_resolves_competing_metrics_and_catalog_mutation_restores_ambiguity` checks unit-based disambiguation, then mutates the competitor into applicability and requires `ambiguous_metric`; `structurally_ambiguous_output_aliases_reject_instead_of_being_ranked` separately checks `invalid_output` | Exact authored-label competition and the narrow exact applicability profile | General authority/precedence systems and approximate semantic equivalence remain unsupported |
| §§9, 12–13: backend validity | Versioned engine and PostgreSQL execution profiles; centralized local-only compiler functions; conservative connector eligibility | Fake-remote tests cover safe aggregate pushdown plus local ratio/checked-SUM/window/uniqueness behavior; PostgreSQL unit tests cover exact integer/null/order/limit and local text/arithmetic fallback | Local DataFusion is the semantic reference; PostgreSQL only pushes its declared equivalent subset | Live PostgreSQL was not run; no external connector equivalence claim is made |
| §10: public entry points | Existing HTTP structured row, graph and graph-intent routes; existing CLI typed flags | 3 `apps/semantic-server/tests/typed.rs` and 5 `apps/semantic-cli/tests/commands.rs` tests pass in the combined gate | Compilation APIs; the HTTP compilation route does not execute rows | A live external server/CLI environment is not claimed |
| §§13–15: records, replay and tracing | Bounded records, ordinary Rust tracing, captures, replay and graph replay; opaque normal-record requirement IDs; graph work/fingerprint accounting; execution-profile identity | Focused privacy, work-accounting, scope-revalidation and replay-profile tests in `crates/semantic-compiler/tests/typed.rs` | Exact sensitive evidence is retained only in explicit artifacts/captures; restricted artifacts require current execution authorization | OpenTelemetry is explicitly out of scope; no exporter requirement is part of this gate |

## Recorded integration commands

The build marshal should run, in order:

```text
cargo test -p semantic-compiler --test mvp_acceptance --offline
cargo test -p semantic-catalog -p semantic-compiler -p semantic-engine \
  -p semantic-ossie -p semantic-server -p semantic-cli \
  --lib --test publication --test snapshots --test search --test source \
  --test typed --test retrieval --test compilation --test deferred \
  --test query --test compiler_functions --test import --test commands \
  --test mvp_acceptance --offline
cargo clippy -p semantic-catalog -p semantic-compiler -p semantic-engine \
  -p semantic-postgres -p semantic-ossie -p semantic-server -p semantic-cli \
  --all-targets --offline -- -D warnings
cargo fmt --all -- --check
git diff --check
```

These gates passed on 2026-09-29. The backend lane additionally passed 9
PostgreSQL library tests. Live PostgreSQL, ClickHouse and model-provider checks
are separate optional evidence and must be labelled with their environment and
configuration.

## Release criteria and non-claims

- Any incorrect expected row, mismatch between direct and emitted-SQL execution,
  unexpected successful ambiguity/scope/applicability case, or failure to enforce
  duplicate lookup keys is a correctness release blocker.
- The checked-in competing-definition fixture passes against the integrated
  `ambiguous_metric` contract. Future changes that make it select by rank are a
  release blocker.
- Held-out interpretation/retrieval quality is not measured by scripted-provider
  tests. Optional live-model evaluations must record model, prompt/protocol,
  configuration, fixture revision and results separately.
- No percentage-complete, universal SQL, general semantic reasoning, live
  connector, or full-architecture claim follows from this suite.
- No OpenTelemetry dependency or instrumentation is required or introduced.
