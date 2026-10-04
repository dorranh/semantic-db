# Acceptance suite and evaluation harness plan

Status: implemented and verified on `feature/semantic-eval-acceptance`,
2026-10-04. The generic harness, reviewed commerce dataset, and informational
CI job are implemented. Verification evidence and current acceptance failures
are recorded in `semantic-eval-implementation.md`; release acceptance is not met.

## Objective and agreed contract

Build a versioned, reproducible acceptance dataset and a reusable runner for SQL
and natural-language queries. Start with 100 answerable cases, each containing
natural language, reference SQL, and independently verified expected results.
Both interfaces must produce those results; generated SQL need not resemble the
reference SQL. Add a small companion set of expected clarification, rejection,
and execution-error cases.

The dataset specifies desired product behavior. Missing importer, interpreter,
compiler, connector, or interface functionality is allowed to make tests fail
during development. Every failure must be fixed before release. There are no
expected-failure annotations, feature exemptions, or baseline-based waivers.
Filtered development runs are useful but cannot establish release acceptance.
Acceptance runs in CI are informational and must not fail the overall build or
block merging. The runner still reports every failure and returns a nonzero exit
code locally and in CI; the CI job handles that failure as nonblocking. Release
acceptance remains a separate complete, zero-failure requirement.

The initial dataset is an acceptance suite used for polishing. Later independently
curated evaluation datasets use the same harness and are scored separately.
Passing this suite does not establish natural-language generalization accuracy.

The harness lives in the generic `crates/semantic-eval` library/runner crate.
Datasets are standalone versioned artifacts supplied by manifest path, with no
dataset-specific code or embedded fixture bank required in the crate.

## Planned subagents and review workflow

During implementation, use a dedicated business analyst subagent to own dataset
curation, a separate reviewer subagent to check its work independently, and an
implementer subagent to write code. The top-level orchestrator owns verification
and runs all Cargo commands. These are explicit implementation roles; this
implementation uses GPT-6.1 Sol for all three subagents.

| Role | Responsibility | Deliverables |
| --- | --- | --- |
| Business analyst subagent | Define the business scenario, business terms, grains, relationships, metrics, calendars, type/edge-case coverage, and distinguishing source records; populate the Ossie model and query bank | Dataset artifacts, canonical data, 100 NL/SQL pairs, 15 companion cases, expected results, and explanations of business meaning |
| Reviewer subagent | Independently review business coherence, Ossie fidelity, query equivalence, result oracles, edge cases, and coverage; recompute selected answers using a separate method | Concrete findings with case/model references and a review disposition for the reviewed artifact revision |
| Implementer subagent | Write the generic harness, lifecycle adapters, comparator, tests, CLI/infrastructure changes, and necessary product fixes; propose focused verification commands | Reviewable code changes and handoffs describing changes, expected behavior, and checks to run; no Cargo execution |
| Top-level orchestrator | Coordinate the artifact/API contract and agent work, review/integrate changes, run all Cargo commands, return diagnostics to the implementer, and manage build resources | Verified integration, command logs/results, dataset review reconciliation, and release disposition |

Give the business analyst the agreed product requirements and coverage matrix,
rather than an instruction to reproduce current implementation limitations. It
owns both the data and authored semantics so business definitions remain coherent.
The analyst owns the dataset's Compose definition and bootstrap requirements;
the implementer writes any supporting bootstrap code and generic lifecycle
machinery. The analyst must not compensate for product gaps with hidden catalog
injection or harness branches.

Keep the reviewer available at three points: the initial schema/Ossie design and
first 10 cases, the completed 100-case bank plus companion cases, and subsequent
substantive dataset changes. The reviewer starts by examining artifacts without
editing them, independently derives expected answers for high-risk measures and
joins, and looks for plausible wrong queries that accidentally return the same
results. It checks temporal boundaries, monetary rounding, fan-out, relationship
roles, null/missing behavior, and semantic type contracts explicitly.

The analyst resolves findings, and the reviewer rechecks the revised artifacts.
The orchestrator resolves disagreements against the stated business requirements
and records the reasoning; a green product run is not proof that the oracle is
correct. Dataset curation is complete only when substantive review findings are
resolved and the reviewer has checked the final revision. This review is internal
to the implementation workflow and does not require repeated user approvals.

Keep generated design notes and review findings under `docs/generated`, in
`commerce-v1-dataset-design.md` and `commerce-v1-dataset-review.md`. Record reviewed
dataset/case digests so later artifact changes cannot inherit a stale review.

### Cargo ownership and build-directory maintenance

Only the top-level orchestrator runs `cargo` commands, including build, check,
test, fmt, clippy, run, and clean. Subagents must not invoke Cargo directly or
through scripts, IDE tasks, or background jobs. The implementer hands off a
coherent code slice and suggested focused checks; the orchestrator runs them and
sends actual diagnostics back for fixes. Avoid concurrent Cargo activity in the
shared workspace. The analyst/reviewer can independently inspect artifacts and
compute reference results without launching Rust builds.

Check available disk space and target-directory size before substantial builds.
Reuse the existing target directory and avoid unnecessary profile/feature
combinations. If accumulated outputs cause space pressure, the orchestrator may
run targeted `cargo clean -p <package>` or, when insufficient, `cargo clean`.
Clean only with no active Cargo process, retain reports/logs outside `target`, and
record the cleanup and ensuing rebuild. Do not clean on a fixed cadence: full
cleanup discards dependency builds and can make the next verification expensive.

## Repository foundations and integration points

Reuse the following mechanisms rather than introducing another query pipeline:

- `crates/semantic-interpreter/examples/held_out_eval.rs`: scripted/live provider
  execution, expected results, context modes, and usage reporting. Extract useful
  patterns; its hard-coded catalogs and fixture switches are not the new dataset
  interface.
- `tests/support/postgres.rs`: useful PostgreSQL readiness and fixture precedents.
  The new harness uses the dataset's Compose stack rather than a hard-coded
  PostgreSQL container; pin images in the dataset and use a dedicated fixture schema.
- `crates/semantic-sources`: `Project`/`Registry` loading, CSV `physical_types` and
  `null_regex`, PostgreSQL configuration, and provider-backed execution.
- `crates/semantic-ossie/src/document.rs` and `executable_profile.rs`: core Ossie
  relationships and bounded metrics, concepts, units, identity, enum mapping,
  conversion, and view-lineage extensions. Broader executable contracts are
  implementation work where selected cases require them.
- `crates/semantic-sources/src/conformance.rs`: batch-independent comparison
  precedent. The acceptance comparator also needs unordered bags, portable
  expected values, explicit nulls, and configurable float comparison.
- `apps/semantic-cli/src/lib.rs`: real SQL and typed Ask execution. Ask currently
  defaults to SQL compatibility; explicitly select typed mode in this suite.
  Typed Ask already executes accepted row/graph artifacts with bound parameters.

Some existing generated reference documents describe older executable profiles.
Use current source and fixtures when deciding what a failing case requires.

## Dataset: commerce-v1

Use one coherent small commerce business with customers, orders, products,
returns, subscriptions, and historical customer regions. This gives natural
questions with identifiable grains, several relationship roles, and multiple
independently aggregatable facts. Additional domains belong in later datasets.

Proposed source placement:

| Source | Relations | Purpose |
| --- | --- | --- |
| CSV | customers, products, exchange_rates, business_calendar | Dimensions, exact rates, authored calendar mapping |
| PostgreSQL | orders, order_items, refunds, subscriptions, customer_region_history | Transaction facts, composite item keys, recurring facts, as-of relationships |

Define billing and shipping customer relationships separately. Link items to
orders/products and refunds to the appropriate order/item grain. Include
multiple items and refunds per order so unqualified joins visibly multiply
amounts. Define valid rollups, independent fact aggregation, missing matches,
and any allocation required by selected cases.

Keep tables small enough for a human to audit, generally tens of rows. Version
canonical data and explicit physical schemas. Derive PostgreSQL inserts from
that data, rather than maintaining an unrelated handwritten seed. The primary
project must genuinely split tables across CSV and PostgreSQL; no local-only
substitution is allowed to pass the federated acceptance run.

Use an explicit CSV null token such as `\\N`, distinct from an empty string.
Declare physical types for all file columns. PostgreSQL DDL records matching
precision, scale, nullability, and temporal meaning. Validate row counts, schema
contracts, key assumptions, and seed digests before running queries.

### Type and data coverage

- Signed 16/32/64-bit integers, finite Float32/Float64, fixed-scale decimals,
  booleans, and UTF-8 strings, including apostrophes, Unicode, and empty strings.
- Date, time-of-day, timezone-free timestamp, and UTC timestamp with fractional
  seconds. Preserve the distinction between local wall time and an instant.
- Nulls in non-key fields, selected null relationship keys, zero and negative
  amounts, duplicate values, and empty result sets.
- Day/month/year transitions, leap day, and Europe/Zurich DST boundaries.
- Tied ranks, unmatched dimensions, customers with no orders, missing months,
  and multiple matching fact rows.
- Store exact monetary amounts in integer minor units and decimal fields with
  authored units. Rates and rounding behavior are explicit. Float columns serve
  measurements rather than silently approximating currency.

Unsigned integers, binary/nested values, and other representations can be added
with their own selected product semantics later. The initial type matrix names
the exact chosen representations rather than claiming every Arrow type.

### Semantic model

Provide a schema-valid Ossie definition containing descriptions, synonyms, AI
context, semantic field mappings, keys, and relationships. Put product-specific
contracts in explicit `SEMANTIC_DB` vendor extensions when the core schema has
no representation. Version and validate each selected extension contract.

Model the following meanings for the selected queries:

- Entity identities and source/output grains; declared keys are not fabricated
  proof of provider-enforced uniqueness.
- Billing/shipping roles, composite keys, existence/absence, and historical
  region validity using half-open intervals.
- Governed counts, sums, averages, weighted averages, distinct counts, and
  ratios, with units, empty behavior, zero-denominator behavior, and rollup rules.
- Business concepts such as active subscriptions and completed orders, defined
  once; field names alone do not establish their meaning.
- Derived financial measures, conversions, business/fiscal calendars, and
  views needed by the query bank.

Load semantics through the actual Ossie/project import path. The runner must
not inject missing catalog facts in Rust or replace a rich definition with a
reduced model to make cases pass. If model loading fails, report the affected
cases as nonpassing with the setup diagnostic, then fix that dependency first.

## Query bank

Give each case one primary coverage group and additional tags. This allocation
totals 100 paired answerable cases; write the exact questions and SQL during
dataset curation.

| Primary group | Cases | Representative coverage |
| --- | ---: | --- |
| Projection, filtering, sorting, limits | 12 | Types, nulls, Boolean scope, negation, stable top-N |
| Concepts, synonyms, views | 8 | Authored business definitions, renamed fields, derived/nested views |
| Relationships and role selection | 15 | Billing/shipping, inner/left joins, composite keys, missing matches |
| Existence and absence | 8 | Customers without orders, products with qualifying sales, scoped predicates |
| Aggregations and governed metrics | 15 | Count/sum/min/max/mean/distinct, empty aggregates, grain correctness |
| Multiple facts and fan-out prevention | 10 | Order versus item grain, refund composition, weighted measures |
| Calendars and temporal interpretation | 10 | Fixed relative periods, local boundaries, fiscal groups, missing periods |
| Historical relationships | 6 | Current versus as-of region, boundary and missing history |
| Windows, ranking, and cumulative values | 8 | Ties, partitions, ranking, final order/limit, explicit running frames |
| Set operations, conversions, and calculations | 8 | Bag/distinct sets, exact conversion, ratios, allocation/derived measures |

At least 30 cases must require interaction between the two sources. Include
combinations of features, not only one isolated query per operator. Each case
records which fixture rows distinguish its intended answer from a plausible
wrong interpretation. No accidental ties may make a top-N answer ambiguous.

Natural language must specify enough information to determine the answer using
the authored model. For multicolumn output, state the requested projection order
where necessary. Exact generated aliases are not part of semantic correctness.
The reference SQL runs through Semantic DB's SQL interface against semantic
relations, not directly against PostgreSQL's physical tables. Explicitly expand
governed meanings and relevant policies in SQL where raw SQL does not apply them.

Add 15 companion cases for genuinely ambiguous concepts/roles, denied scope,
unavailable data, invalid rollups, duplicate lookup keys, conflicting temporal
matches, and arithmetic errors. These have expected semantic outcomes or errors;
they need not have a paired answer-producing SQL query. Expected clarification
is a passing outcome only where the case explicitly requires clarification.

## Dataset and result contracts

Separate the generic workspace crate from the dataset artifacts:

```text
crates/semantic-eval/
  Cargo.toml
  src/lib.rs             # generic dataset contract, validation, execution, reports
  src/bin/semantic-eval.rs
  tests/                 # harness unit/integration fixtures, not the query bank

tests/datasets/commerce-v1/
  manifest.json
  compose.yaml           # dataset-owned services, images, volumes, health checks
  bootstrap/             # dataset-owned setup scripts/DDL/seed material
  model.ossie.yaml
  semantic-db.yaml
  schemas.json
  data/                  # canonical records and CSV files
  cases.json             # query bank and semantic requirement notes
  expected/              # independently checked typed results

docs/generated/
  commerce-v1-dataset-design.md
  commerce-v1-dataset-review.md
```

A dataset directory is a self-contained artifact bundle. Resolve its project,
schema, Compose, bootstrap, data, case, seed, and expected-result paths relative
to its manifest,
independent of the current working directory. The same bundle can be checked in,
copied elsewhere, or unpacked from a dataset archive and run by manifest path.
Adding a dataset must not require rebuilding the crate, adding `include_str!`,
registering a domain name, or changing a Rust match statement.

Keep the initial library surface small: dataset loading/validation, run options,
execution returning a structured report, and report serialization. Use existing
source-registry and model-provider contracts where injection is needed. Keep
manifest validation and result comparison pure and separately testable; isolate
container/database/model/process effects in execution adapters. Support a small
dataset-defined Compose/bootstrap contract rather than encoding database setup
logic in the crate. No new plugin system is needed.

Acceptance completeness rules and later evaluation scoring use the same case and
result contracts, with policy supplied by manifest/run configuration. Commerce
table names, business definitions, expected answers, and source placement belong
only in the artifact bundle. The executable is a thin client of the library so
other tools can run datasets without invoking a subprocess.

The manifest identifies its format version, dataset ID/version, project path,
environment lifecycle configuration, default request clock/timezone, case file,
and required case count. Cases have stable IDs, tags, natural language, reference SQL, expected
result path, comparison mode, optional context overrides, and a short semantic
requirement ledger. The ledger is not a required generated IR or SQL shape.

Use a tagged expected-outcome contract: `result`, `needs_clarification`,
`unsupported`, `rejected`, or `execution_error`. Initially every paired case
expects `result`. Add optional deterministic intent proposals for targeted
compiler diagnosis without requiring every future evaluation dataset to author IR.

Expected results include a column/type contract and typed rows. Preserve integer
and decimal values losslessly as strings where JSON numbers would lose precision;
encode dates/timestamps with explicit units/timezone semantics. Null is JSON
null, never a display string. Preserve output schema for zero-row results.

Comparator rules:

- Compare both SQL and Ask independently to the same expected result.
- Unordered results are multisets: duplicate multiplicity must match. Ordered
  results are sequences. Ignore Arrow batch boundaries.
- Column order and semantic type/value contracts must match. Column labels are
  descriptive by default; assert exact names or physical widths only when the
  case specifically tests that interface contract. Retain actual schemas in
  reports even when names are not asserted.
- Integers, decimals, booleans, strings, dates, and temporal values compare
  exactly after their declared lossless normalization. Do not string-format all
  values or cast incompatible types to get a match.
- Float tolerance is explicit per column using absolute/relative bounds;
  default to exact comparison. Do not apply float tolerance to money.
- Schema/value mismatch, dropped duplicates, unexpected extra columns, partial
  results, or an unintended clarification/refusal are failures.

Establish the result oracle by hand review of small fixtures and an independent
calculation or reference database query. A temporary full-data PostgreSQL copy
may help verify joins/aggregates, but is not the tested federated path. Include
reasoning for business metrics and distinguishing edge cases. Never overwrite
expected results automatically from the product output to make a run green.

## Runner and execution paths

Implement one runner with dataset discovery, stable case selection, adapters,
fixture lifecycle, comparison, and structured reports. New datasets should need
data/model/configuration and manifests, not domain-specific Rust switches. The
primary input is a manifest path, not a compile-time dataset name or fixed
repository location; discovery is an optional convenience over artifact paths.

Execution paths:

1. **SQL:** load the real mixed-source project, execute the reference SQL using
   the public engine SQL path, and compare schema and complete rows to the oracle.
2. **Ask:** use `Interpreter::compile_typed` with explicitly selected context
   mode and request context. Execute its returned row or graph artifact with its
   bound parameters; compare to the oracle. Do not expose reference SQL, expected
   results, requirement notes, or optional diagnostic IR to the model.
3. **Public interfaces:** run representative actual CLI SQL/Ask cases and HTTP
   typed-compilation plus server SQL execution cases using the same fixtures.
   Ensure command configuration, serialization, parameter forwarding, graph
   execution, failure reporting, and host context do not diverge from the core
   paths. The full bank runs in-process to avoid 100 repeated application starts.

Use typed-auto as the primary Ask mode and allow typed-full and typed-retrieved
as explicit comparable runs. SQL compatibility remains separately labeled and
cannot turn a failing typed case into a passing one. Fix the host reference
instant, timezone, scope, and calendar context; never substitute wall-clock time.

If public CLI adapters need machine-readable result/schema output or request
clock/timezone options, add them to the actual commands and verify their behavior
with this fixture. Do not scrape pretty-printed tables or create a test-only
command that bypasses the normal command flow.

### Dataset-owned Compose and bootstrap lifecycle

Each dataset supplies its own Compose stack. It defines required databases or
other services, pinned images, volumes, fixture mounts, health checks, and a
simple bootstrap step that creates schemas, loads canonical data, and sets any
required service configuration. The harness owns the lifecycle; it does not need
to understand the dataset's DDL, seed contents, service business purpose, or a
specific database's setup procedure.

Use a designated one-shot Compose service for bootstrap. Dataset-owned scripts
and data are mounted or included by the Compose definition; invoke the service
through structured Docker Compose arguments, not an embedded host-shell string.
The manifest names Compose file(s), long-running service names, the optional
bootstrap service, readiness/bootstrap timeouts, and environment bindings needed
by the Semantic DB project. The bootstrap step has a simple exit-code contract:
zero means setup completed; nonzero means setup failed. Repeated setup must
produce the same fixture, not append duplicate seed rows.

The lifecycle is:

1. Validate the artifact bundle, resolve all paths from its manifest, and create
   a unique Compose project name and run-artifact directory.
2. Start the dataset's long-running services and wait for their declared health
   checks with a bounded startup deadline.
3. Discover published endpoints from Compose and resolve manifest-defined
   environment bindings. Avoid fixed host ports so independent runs can coexist.
4. Run the dataset's bootstrap service once to completion before loading the
   semantic project. It can access stack services by their Compose service names.
5. Load the real semantic project with runtime connection bindings, verify
   fixture/schema contracts, and execute the selected cases against the prepared
   environment. Reuse it read-only within that run.
6. Collect reports and Compose/bootstrap logs, then tear down the run's services,
   networks, and ephemeral volumes on success, failure, timeout, or cancellation.

Environment bindings connect dataset-declared variables to discovered service
host/port values and any explicit connection templates or bootstrap-generated
runtime metadata. Runtime outputs belong in the run-artifact directory rather
than modifying checked-in project/data files. Do not log provider credentials.
Keep the first contract small and document it so a later dataset can bring a
different stack without changing Rust source.

Normal and release runs manage a fresh stack automatically. An explicit
development option may attach to an already prepared environment; report that
mode and do not use it as evidence of reproducible release setup. Offer an
explicit keep-environment option for debugging, report the Compose project name
and cleanup command, and clean stale run resources deliberately after a crashed
process. Limit concurrency, model calls, output rows/bytes, and query deadlines.

Proposed harness commands (to implement):

```text
cargo run -p semantic-eval -- validate --dataset tests/datasets/commerce-v1/manifest.json
cargo run -p semantic-eval -- run --dataset tests/datasets/commerce-v1/manifest.json --interface sql
cargo run -p semantic-eval -- run --dataset tests/datasets/commerce-v1/manifest.json --interface ask --context auto
cargo run -p semantic-eval -- run --dataset tests/datasets/commerce-v1/manifest.json --case roles.billing_vs_shipping
cargo run -p semantic-eval -- release --dataset tests/datasets/commerce-v1/manifest.json --repetitions 3
```

Reports record suite completeness, revision and fixture/case digests, interface,
context mode, configured and returned model identity, settings, per-case outcome,
actual schema/rows and readable diffs, compilation/execution diagnostics,
latency, model calls/repairs/context expansions, and token usage or explicitly
unknown usage. Exclude credentials. Write JSON plus a concise terminal summary
to a configurable gitignored artifact directory. Separate suites in summaries;
do not pool future evaluation scores with commerce-v1 acceptance.

Continue after individual failures to produce the full report. A model quota,
transport, fixture-loading, or missing-Docker failure leaves affected cases
nonpassing and the run incomplete; it never counts as semantic success.
Return nonzero when any selected case fails or when a required run is incomplete.
The dedicated CI acceptance job tolerates this exit status without changing the
runner's result or concealing failures in its reports.
Release mode also rejects filtered/incomplete coverage and confirms every
mandatory interface/context run occurred.

## Implementation sequence and delivery checks

### 1. Dataset contract and fixture design

Implement the manifest/case/typed-result contracts and offline validation.
The orchestrator coordinates the generic `semantic-eval` boundary and artifact
schema with the implementer, then delegates business definitions, canonical
records, Compose/bootstrap definition, Ossie population, and the first 10 cases
to the business analyst subagent. Include basic filtering,
nulls, roles, aggregation, and a cross-source join. The independent reviewer
checks that first slice before the analyst expands the bank. Harness work can
proceed independently once the artifact contract is agreed.

Check: invalid IDs/versions, unresolved paths, missing expectations, malformed
typed values, and inconsistent expected column shapes fail before model calls.
The first dataset slice has resolved reviewer findings and recorded oracle
reasoning, even if product execution still fails.

### 2. Reproducible mixed-source loading

Add canonical records, CSV output, the dataset's Compose/bootstrap artifacts,
project configuration, and the analyst-authored Ossie model. The implementer
writes generic lifecycle management and artifact-driven loading through the
public project loader; the orchestrator runs Cargo verification and checks exact
seed/type contracts. The analyst grows the full model as the bank is curated;
repair import gaps instead of bypassing Ossie.

Check: repeated fixture creation has identical schema/data; null and empty text
remain different; a real cross-source query reaches both providers. Corrupt
schema/seed input fails setup with a useful diagnostic. Bootstrap/readiness
failure still produces logs and cleans up the stack. Parallel dataset runs do
not collide in project names or host ports.

### 3. SQL runner and comparator

The implementer writes the execution adapter, typed comparison, case filtering,
full failure collection, and JSON reports. The orchestrator runs the first 10
SQL cases through the real project and all Cargo checks.

Check: deliberate wrong rows, multiplicity, order, nulls, decimals, empty schemas,
and float bounds produce accurate diffs. Batch splitting does not change results.
Fix any product SQL failures against the reviewed requirements.

### 4. Typed Ask and public-interface integration

The implementer connects a configured live provider, pins host context, executes
accepted artifacts, and adds actual command/server smoke cases. The orchestrator
runs the Cargo commands and evaluates the reports. Use scripted providers only for
deterministic harness tests, including malformed output, clarification, bound
parameters, graph results, provider failure, and report accounting. Scripted
success is not a substitute for running acceptance questions against a model.

Check: Ask and SQL each compare to the oracle; oracle fields never enter model
context; parameters and request clock survive the public paths; no compatibility
fallback masks typed failure. Record the initial live failures without exemptions.

### 5. Complete the bank and polish

Expand to the 100 paired cases and 15 companion cases using the coverage matrix.
The business analyst owns this expansion; the independent reviewer checks query
meaning, Ossie definitions, reference SQL, expected types/rows, and distinguishing
data together, and rechecks the analyst's fixes. Run the complete suite and fix
product failures one by one, adding focused regressions where they help isolate
defects. Repairing a wrong fixture or oracle is appropriate only with documented
semantic evidence and renewed reviewer scrutiny.

Check: exactly 100 paired cases; at least 30 genuinely cross-source; all selected
type and feature groups exercised. No cases are removed or relabeled merely
because implementation is difficult. Dataset design and oracle review are
complete for the exact final artifact revision.

### 6. CI, release execution, and future dataset extension

Run manifest/comparator/scripted-harness checks without provider credentials.
Add a dedicated, nonblocking acceptance job for Docker-backed SQL and configured
live Ask runs using each dataset's own Compose/bootstrap lifecycle. The implementer
writes the workflow and commands; the orchestrator verifies them. Use job-level
`continue-on-error: true` so acceptance failures do
not fail the overall workflow. Keep acceptance execution outside the blocking
workspace test command. Run SQL and Ask independently so a failure in one does
not prevent the other from producing results. Always publish available JSON
reports and logs as artifacts and write a job summary with passed/failed counts,
incomplete runs, and model availability, using `if: always()` for reporting steps.
Do not erase the runner's exit status with `|| true` or turn failed cases green.

Provide a complete local release command. Absence of model credentials means Ask
acceptance has not run and must be visible in the CI summary; SQL should still
run. The normal build remains usable throughout polishing. There is no
expected-failure baseline.

Release requires all 100 SQL cases and all 100 Ask cases to pass against the
primary mixed-source configuration, plus all mandatory companion/interface
checks. Repeat live Ask three times per case with fixed settings and randomized
case order; every repetition must pass. Required provider failures are nonpasses,
not silently retried until a passing response is selected. Full/retrieved context
experiments and pushdown-disabled SQL conformance runs are reported explicitly;
select any additional release-required modes before the release run.

Check: a deliberately failing acceptance case produces a failure report and CI
summary while the overall build remains successful; available artifacts are
uploaded after failures. The release command must still fail for that same case.
The release report proves complete coverage and zero failures. Add a tiny
second artifact dataset to prove loading and reporting work without adding a
domain-specific Rust branch. Run both bundles from a location outside the source
checkout using the same built executable to verify manifest-relative paths.
Later evaluation datasets reuse these contracts while keeping their own labels,
versions, and scoring rules.

## Completion criteria

- Reproducible versioned data, physical schemas, a rich Ossie definition, and a
  real CSV/PostgreSQL split.
- 100 reviewed NL/SQL pairs with independent typed result oracles and documented
  semantic coverage, plus 15 reviewed companion cases.
- Extensible runner, honest complete reports, pinned context/model configuration,
  meaningful comparator tests, and real public-interface checks.
- A generic `crates/semantic-eval` library and executable that load standalone
  dataset bundles by manifest path without dataset-specific Rust code.
- Dataset-owned Compose stacks and bootstrap steps, with generic startup,
  readiness, runtime binding, logging, and teardown managed by the harness.
- A dedicated business analyst has authored the dataset/Ossie/query bank, and a
  separate reviewer has independently checked the final artifacts and resolved
  substantive findings.
- An implementer subagent owns code changes; the top-level orchestrator runs all
  Cargo commands and manages build-directory cleanup when disk pressure warrants it.
- Separate deterministic harness verification, SQL acceptance, and live Ask
  acceptance. A green scripted run cannot satisfy the live Ask requirement.
- Nonblocking CI acceptance with visible summaries and retained failure reports;
  the local runner and release command preserve meaningful failure exit codes.
- All required acceptance runs pass before release; current missing behavior
  remains a visible failure until implemented.

The next implementation step is phase 1: define the generic artifact contract,
assign the business analyst and independent reviewer subagents, and produce the
first 10 reviewed cases with a dedicated implementer handling code and the
orchestrator handling Cargo verification before expanding the bank.
