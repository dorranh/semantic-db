# Generic acceptance evaluator

The `semantic-eval` library and executable load standalone dataset bundles by a
manifest path. Business semantics, canonical data, Compose services, bootstrap,
SQL, questions, and typed gold belong in that bundle. The runner imports the
actual project and never substitutes a reduced catalog or SQL compatibility Ask.

```sh
cargo run -p semantic-eval -- validate --dataset tests/datasets/commerce-v1/manifest.json
cargo run -p semantic-eval -- run --dataset tests/datasets/commerce-v1/manifest.json --interface sql
cargo run -p semantic-eval -- run --dataset tests/datasets/commerce-v1/manifest.json --interface ask --context auto --env-file .env
cargo run -p semantic-eval -- run --dataset tests/datasets/commerce-v1/manifest.json --case projection.customer_ids
cargo run -p semantic-eval -- release --dataset tests/datasets/commerce-v1/manifest.json --repetitions 3 --env-file .env
```

Reports and Compose logs default to unique run subdirectories of `.semantic-eval`, outside build outputs.
`--artifacts` chooses the parent directory; each run reports its exact JSON path.
Outputs must stay outside the immutable bundle. When the default would land
inside the bundle, it moves to `.semantic-eval` in the bundle parent; explicitly
requesting a nested output directory fails before setup. A missing model,
setup failure, timeout, or provider error remains a nonpass and makes the run
incomplete. The exit status is nonzero on failures or incomplete execution.
Filtered development runs can pass, but report `full_coverage: false` and cannot
establish release acceptance. Ordinary deterministic tests need no Docker or
provider credentials. `tests/fixtures/tiny` is a second, CSV-only artifact bundle.

## Artifact format version 1

The manifest requires `format_version`, `id`, `version`, `project`, `cases`,
`required_paired_cases`, and `context`. `public_cases` names paired cases for mandatory public interface smoke checks.
`required_companion_cases` defaults to
zero and makes companion coverage mandatory when populated. Context declares
RFC3339 `reference_time`, IANA `timezone`, `calendar` (currently only Gregorian),
and optional `allowed_relations` for typed compilation and execution scope.
Optional `schemas` and `canonical_data` point to accompanying metadata. Optional
`fixtures` list SQL/expected paths for setup verification.

Paths must be relative, remain inside the bundle, and exist. The bundle digest
covers sorted relative filenames and all file bytes. Case files contain an
array of unique stable IDs, questions, optional SQL, expected paths, comparison
settings, tags, requirements, primary groups, oracle reasoning, distinguishing
rows, and optional full context overrides. Every paired case must expect a
result. Cases without SQL run through Ask only.

Expected outcomes are tagged by `outcome`: `result`, `needs_clarification`,
`unsupported`, `rejected`, or `execution_error`. Non-result expectations may
specify `diagnostic_contains`. Results contain ordered `columns` and `rows`.
Each column has a descriptive `name`, `type`, optional `nullable` (default true),
optional `physical_nullable` to assert provider schema metadata,
and optional float `tolerance` with nonnegative `absolute` and `relative` bounds.
`nullable` is an authored data-domain invariant, not proof that a provider
enforces it. Setup compares all canonical rows, verifies key assumptions,
checks exact physical type/name contracts, and verifies each actual configured
connector against the schema source; fixture evidence retains provider metadata.
Null is JSON null. Integers and decimals are lossless strings; Boolean and finite
float values use JSON Boolean/numbers. UTF-8 uses strings. Supported types are
`int16`, `int32`, `int64`, `uint8`, `uint16`, `uint32`, `uint64`, `float32`, `float64`, `boolean`, `utf8`,
`decimal128(precision,scale)`, `date32`, `time64(us)`/`time64(ns)`,
`timestamp(s|ms|us|ns)`, and `timestamp(s|ms|us|ns,UTC)`.
Dates use ISO dates, times use ISO wall-clock times, naive timestamps use ISO
without an offset, and instants use RFC3339 with an offset. Exact normalization
preserves the distinction between wall time and instant. Schema persists when
rows are empty.

Comparison is a duplicate-preserving bag by default; `ordered: true` compares a
sequence. `assert_names` and `assert_physical_types` default false. Logical
compatibility permits exact signed/unsigned integer and float width and exact decimal/temporal promotion;
incompatible types never match. Float tolerance never applies to decimals.
Tolerance bag comparison uses full matching rather than greedy removal.

## Lifecycle and configuration

An optional `environment` supplies relative `compose_files`, long-running
`services`, optional `bootstrap_service`, positive `startup_timeout_seconds`
and `bootstrap_timeout_seconds`, and `bindings`. Each binding maps a project
secret name to `{service,port,template}`. The template substitutes `{host}` and
`{port}` from the published Compose endpoint. Use random published host ports.
Services must declare healthchecks and become healthy. The one-shot bootstrap
service exits zero only after an idempotent fixture load. The runner captures
startup, health, endpoint, bootstrap, service, and cleanup logs, and removes
services/networks/volumes after success or failure. `--attach` uses an explicitly
prepared environment and process/env-file bindings; it cannot establish release
reproducibility. `--keep-environment` retains resources for debugging and reports
the exact cleanup arguments.

`--env-file` is parsed without modifying global environment; process environment
wins. Model configuration uses `SEMANTIC_EVAL_MODEL` (or `OPENAI_MODEL`) and
`OPENAI_API_KEY`; endpoint uses `SEMANTIC_EVAL_BASE_URL` (or `OPENAI_BASE_URL`).
Credentials are never written into reports. Questions and authored model are
the only dataset semantics exposed to Ask; oracle SQL/results/notes are excluded.

CI acceptance uses job-level `continue-on-error: true`, executes SQL and Ask
independently, and always summarizes and uploads available reports/logs. Runner
failures retain their exit status. No baseline or expected-failure waiver exists.

## Public interface checks

`--public-interfaces --cli-binary /absolute/path/to/sdb` runs the manifest-selected
representative cases through real CLI SQL, CLI typed Ask, and HTTP typed compilation
followed by PostgreSQL protocol execution with returned bound parameters.
Release always requires these checks for all manifest `public_cases`. The executable must already be built;
the harness never invokes Cargo. The selected case IDs and each check outcome appear
in `public_checks`. Without explicit `public_cases`, the first supported scalar result is used.
Missing binaries/providers or unsupported wire parameter
types remain failures. HTTP checks verify the retained fixed request instant.
Use `cargo build -p semantic-cli -p semantic-eval` before a local release run.
The public command's `--output-json` emits the same lossless typed rows and
schema contract consumed by the comparator, including schema for empty output.

An embedding application may use `run_with_provider` with the existing
`ModelProvider` trait for deterministic orchestration diagnostics. Such runs
explicitly cannot certify live release acceptance.

## Explicit CSV nonnull source contracts

CSV source configuration may declare `non_nullable_columns` using physical
column names. The shared loader streams all declared columns to completion
before publishing the provider, and Arrow batch validation rejects actual nulls.
This opt-in preflight establishes an observed source invariant before Ossie
imports entity identities; it does not fabricate metadata from keys or a sample.
Other columns remain nullable. Unknown/duplicate names and non-CSV use fail
validation. Default sources retain their lazy loading behavior. Dataset fixtures
are immutable/read-only during a run; the digest guard rejects artifact changes.

Provider failures retain sanitized `provider_errors` for each case, including
HTTP status codes such as 429. Response bodies and credentials are excluded.
This distinguishes rate limits, quota/authentication failures, and transport
failures from semantic misses without retrying until a passing answer appears.

## Optional model call pacing

`--model-request-interval-ms` sets a minimum interval between starts of harness
model requests, from 0 (the default, disabled) to 60000 milliseconds. For example:

```sh
semantic-eval run --dataset tests/datasets/commerce-v1/manifest.json --interface ask --model gpt-6-luna --env-file .env --timeout-seconds 120 --model-request-interval-ms 12000
```

One shared gate covers every in-process model call, including compiler repairs
and context expansions across cases. Waiting counts against the existing case
and compilation deadlines, and reported latency includes that wait. The report
records `model_request_interval_millis`. Pacing adds no transport retries and
does not rerun semantic cases or replace failure evidence.

Public CLI/HTTP subprocess calls are not throttled by this harness option.
Use an Ask-only run without `--public-interfaces` when collecting paced
observations of previously provider-blocked cases.

Execution admission and output collection use separate limits. `--max-decoded-bytes`
overrides the engine admission budget (product fallback 1 GiB); this includes
conservative source decoding and scratch estimates, rather than final result size.
`--max-output-bytes` (default 32 MiB) and `--max-output-rows` (default 100,000) bound
collected results. SQL and typed Ask receive the same execution budget. Reports retain
all effective engine budgets and both output limits. Public subprocess checks forward the same effective deadline and all three admission
limits to CLI SQL, typed Ask and the HTTP/PostgreSQL server. The generic product
flags are `--query-timeout-seconds`, `--query-max-requests`,
`--query-max-decoded-bytes` and `--query-max-remote-bytes`. Absent product limit flags
preserve the engine's configured budgets and cache/materialization policy.

Reports are atomically replaced, beginning with a running report before setup. Running
snapshots have `finalized=false` and `complete=false`. The declared `planned_cases`
contain each requested case/interface/repetition tuple; planned public checks and
expected/completed counts are recorded separately. `full_coverage` describes selection,
not completion. Success requires terminal cleanup and digest checks plus the exact
planned identities and counts. Older reports without finalization evidence cannot pass.

A manifest may declare artifact capacity independently of output collection:

```json
"execution": { "max_requests": 1024, "max_decoded_bytes": 1073741824, "max_remote_bytes": 268435456 }
```

Every field is optional. Explicit `RunOptions` or CLI `--max-requests`,
`--max-decoded-bytes`, and `--max-remote-bytes` overrides take precedence over the
manifest; absent values use product defaults (256 requests, 1 GiB decoded admission,
256 MiB remote bytes). Admission counts genuine source operations such as each
PostgreSQL cursor fetch. Capacity must be positive, bounded to 1,000,000 requests and
1 TiB per byte limit, and pass the engine's query-budget validation. Invalid artifact
capacity fails offline validation. This configuration applies to every selected case
and both in-process interfaces. Reports retain the resolved budgets.
