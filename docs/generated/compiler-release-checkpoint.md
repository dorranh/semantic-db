# Compiler release verification report

Recorded 2026-09-30. The interpreter/compiler split and the scoped numeric/join implementation are integrated. Final workspace all-target/all-feature, formatting and whitespace checks pass. Changes remain uncommitted in the feature branch working tree.

## Delivered

- `semantic-interpreter` owns model providers, prompts, context selection/cache/hydration, capture, bounded repair, model budgets and interpretation records.
- `semantic-compiler` accepts structured intent without a model-provider dependency, binds against authoritative catalog data, and produces checked query IR plus SQL/direct plans. Shared untrusted intent types remain in `semantic-plan`.
- CLI, server, facade, tests, examples and performance targets use the new boundary. The interpreter prompt advertises the implemented Float64/Avg capabilities.
- Float64 literals/comparisons/ordering and numeric aggregates work through both SQL and direct plans. Nonfinite literals reject. Signed-integer Avg uses explicit Int64 casts and the existing exact mean implementation; exact integer/decimal sums remain intact. Execution profile revision is v11.
- Authored drilling relationships support grouped inner lookup, left lookup preserving unmatched wells, and existence/absence. Duplicate lookup keys fail the checked query.
- Interpreter lifecycle metrics are separate from compiler metrics. Server model requests attach interpreter metrics, and `/v1/compiler/metrics` exposes an `interpretations` snapshot alongside `compilations` and `cache`. Explicitly supplied compiler metrics remain active when interpretation invokes compilation.
- Snapshot drift across interpretation and compilation is rejected. A wrapper-record terminal-status bug found by the held-out tests was repaired.

## Final verification evidence

Results below were reported by the executing Sol workers, except the approved live ClickHouse attempt, which the coordinator ran. Checks were targeted to the affected behavior; no claim of an exhaustive language proof is made.

| Check | Result |
| --- | --- |
| `cargo check --workspace --all-targets --all-features --offline` | Passed on the integrated tree |
| `cargo fmt --all -- --check` | Passed after final edits |
| `git diff --check` | Passed after final edits |
| Interpreter tests/examples build | Passed |
| Performance all-target build | Passed |
| Interpreter snapshot drift test | 1/1 passed |
| Interpreter metrics lifecycle | 3/3 passed |
| Interpreter context cache | 2/2 passed |
| Interpreter held-out cases | 2/2 passed |
| Interpreter retrieval | 8/8 passed |
| Interpreter typed cases | 52/52 passed |
| Interpreter context dependency reuse | 3/3 passed |
| Interpreter context manifest | 11/11 passed |
| Interpreter compilation | 8/8 passed |
| Interpreter MVP acceptance | 10/10 passed on rerun |
| Compiler `typed_float` | 3/3 passed |
| Ossie `geospatial_typed` | 2/2 passed |
| Ossie `drilling_typed` | 3/3 passed, including actual Arrow UInt64 summary sample counts |
| Server library tests | 3/3 passed |
| Server typed-route tests | 3/3 passed on rerun |
| Broad deterministic compiler runtime sweep | Interrupted to free the shared build lock; completed groups passed, but do not count this as a full pass |
| Live typed ClickHouse test | Compiled, but Docker container creation timed out before any query executed |

The listed interpreter targets total 100 passing tests. Numeric/geospatial/drilling targets add eight, and server targets add six. Earlier failing runs are not counted as passes; relevant fixes were rerun as noted.

## Demonstrated query behavior

The local wells tests exercise count, Float64 extrema/average and combined filters. The drilling tests exercise authored relationship import, aggregation by a looked-up basin, left lookup with missing summary rows, absence and duplicate-right-key rejection. Independent expected rows are compared with SQL and direct execution. The offline drilling fixture now uses UInt64 summary counts rather than an Int64 approximation.

## Live ClickHouse blocker

The user approved elevated Docker access. The coordinator ran the ignored typed ClickHouse integration test, which compiled and then failed after approximately 120 seconds with `Client(CreateContainer(RequestTimeoutError))`. No ClickHouse query ran. Starting the installed Docker application and bounded daemon checks did not establish readiness. This is a container-startup blocker, not a successful backend test and not evidence of a query failure.

Once Docker is healthy, rerun:

```sh
cargo test --offline -p semantic-sources --features clickhouse --test clickhouse_typed authored_typed_drilling_queries_run_on_clickhouse_with_federation_on_and_off -- --ignored --exact --nocapture
```

## Verification limits

- No live-model interpretation accuracy evaluation was performed. Natural-language completeness remains separate from deterministic compilation.
- Source NaN/infinity behavior lacks the planned boundary coverage. The finite-value fixtures and nonfinite literal rejection do not prove every exceptional source-value case.
- OpenAI transport and nested-view interpreter runtime targets were build-checked but not run in the final selected sweep. CLI/REPL targets build; a complete CLI/REPL runtime sweep was not performed.
- The broader compiler runtime sweep was partial. A complete operation/type/stage matrix for all existing advanced families has not been verified.
- No claim is made that all SQL, all possible intent combinations, or all backend pushdowns are supported. The scoped implemented cases above are the verified deliverable; live ClickHouse remains outstanding.
