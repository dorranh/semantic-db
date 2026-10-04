# Acceptance suite implementation evidence

Date: 2026-10-04. Branch: `feature/semantic-eval-acceptance`.

The implementation adds a standalone artifact-driven `semantic-eval` crate,
the reviewed `commerce-v1` dataset, and an informational GitHub Actions job.
SQL and typed Ask are independently compared with typed result golds; generated
SQL is retained as evidence rather than compared with the reference SQL text.

The business analyst, implementer, and independent reviewer all used GPT-6.1 Sol.
Only the parent orchestrator ran Cargo commands. Dataset design and the reviewed
revision are recorded in `commerce-v1-dataset-design.md` and
`commerce-v1-dataset-review.md`.

The dataset contains 100 result-bearing NL/SQL pairs and 15 companion outcomes.
Three execution-error companions also have SQL, giving 103 SQL attempts and
115 Ask attempts per full repetition. Four dimensions are CSV and five fact
relations are PostgreSQL; 53 paired cases actually cross the source boundary.
The bundle owns its Compose stack, bootstrap, canonical data, schemas, Ossie
model, case bank, and independent golds. A separate tiny CSV bundle exercises
reuse without commerce-specific branches.

## Verification

Verification completed with two build jobs and incremental compilation disabled
to fit the available machine. The orchestrator ran the following checks with
`CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0` for build, test, and clippy commands:

| Check | Observed result |
| --- | --- |
| `cargo test -p semantic-eval -p semantic-cli --locked --offline`, followed by the final evaluator-only rerun | 52 CLI tests and 21 evaluator tests passed; one existing live CLI test remained ignored |
| `cargo test -p semantic-sources --features postgres --test files --locked --offline` | All 19 file-source tests passed |
| `cargo clippy -p semantic-eval -p semantic-cli -p semantic-sources --all-targets --features semantic-sources/postgres --locked --offline -- -D warnings` | Passed |
| `cargo build -p semantic-eval -p semantic-cli --locked --offline` | Passed |
| `cargo fmt --all -- --check`, `git diff --check` | Passed |
| `semantic-eval validate --dataset tests/datasets/commerce-v1/manifest.json` | Imported project validates; 100 paired cases and 15 companions |
| Copied tiny bundle, run from its own `/tmp` working directory | SQL 1/1, complete; default artifacts routed outside bundle |
| Tiny live run with public interfaces | SQL/Ask 2/2 and all three public checks passed |
| Full commerce SQL baseline | 102/103, complete; all 100 answerable pairs passed |
| Three-case commerce live/public smoke | SQL/Ask 5/6; public checks 5/9; nonzero exit retained |
| Full commerce Ask with provider diagnostics | 3/115, incomplete; 109 provider failures retain HTTP 429 |

The total is 92 passing focused tests. The complete workspace and hosted GitHub
Actions workflow were not executed locally. Ordinary tests use deterministic
providers and do not call a live model. Live evidence was obtained separately.

Before building, targeted Cargo cleanup removed approximately 82.5 GiB of stale
workspace outputs. Dependency outputs were retained; evaluation evidence lives
outside `target`. Docker access requires `sg docker -c '...'` in the current
shell because its inherited groups predate the Docker group membership change.

An initial real Compose run proved startup, bootstrap, log capture, failure
reporting, and cleanup. It exposed misplaced identity extensions in the authored
Ossie document. The analyst moved all nine extensions to the model level and
updated the generator; the reviewer checked the corrected artifact.

The PostgreSQL fixture URL was also updated with explicit `sslmode=disable` for
its dedicated local test service. CSV entity identity loading then exposed the
need for explicit nonnull source contracts. Required-column preflight tests found
that DataFusion 55 applies custom CSV null patterns to inference but omits them
from its runtime reader. The source fix preserves declared null tokens and empty
strings through Arrow decoding and validates required columns before import.

A copied tiny bundle passed from `/tmp` with its own working directory and
default output placement outside the bundle. A live tiny run passed both SQL and
Ask, plus CLI SQL, CLI typed Ask, and HTTP typed compilation followed by bound
PostgreSQL protocol execution. The live interpreter used configured
`gpt-4.1-mini`; implementation subagent models were GPT-6.1 Sol.

## Baseline results and retained evidence

The final dataset SHA-256 using harness framing is
`b7f30c2e568dd1640a33f55bb0c0734fb709badf0898846de6fa5e01d01e367a`.
The analyst's bundle digest uses a different framing; exact file hashes and the
independent oracle methods appear in the dataset review.

Local evidence paths, relative to the repository root:

- SQL: `.semantic-eval/commerce-sql-baseline/run-131458-18db42121552288d/report.json`.
- Ask with HTTP status capture: `.semantic-eval/commerce-ask-diagnostics/run-134453-18db42729f10e77d/report.json`.
- Commerce public interfaces: `.semantic-eval/commerce-live-public/run-127782-18db41e6602c371a/report.json`.
- Tiny live public interfaces: `.semantic-eval/tiny-live-public/run-124157-18db412aa923c3fe/report.json`.

Each real Compose run successfully cleaned up its resources. Startup, healthy
service checks, bootstrap, fixture evidence, service logs, and cleanup output
are retained alongside reports. Public runs retain sanitized command/server
logs, typed compilation artifacts, and observed result JSON. Reports are ignored
by Git; the CI workflow uploads its own run artifacts.

The remaining SQL failure is `error.overflow`: multiplying the maximum signed
64-bit integer by two returns a wrapped result, while the gold requires an
execution error. This is a failing acceptance requirement, not a waived case.

The commerce smoke run passed all three CLI SQL checks and both natural-language
public checks for null/empty-string filtering. Billing joins and previous-month
questions exposed typed proposal/compilation failures in the public paths.
The main runner separately answered the billing join in that run, illustrating
why repetitions and public interface coverage matter.

The first full Ask run was 4/115 with 108 generic provider failures. The final
run preserves safe `provider_errors` and confirms HTTP 429 on 109 attempts.
Its other outcomes were three results, one unresolved request, one rejection,
and one unsupported response; three cases matched their golds. These scores
are incomplete evidence, not an estimate of interpreter accuracy. Valid live
responses also exposed a leap-day result mismatch and invalid typed proposals.
The full run needs sufficient provider rate capacity before it can establish
acceptance. No automatic transport retries or failed-case waivers were added.

## GPT-6 Luna followup

The user subsequently requested live evaluation with `gpt-6-luna`. The full
run passed 102/103 SQL attempts and 18/115 Ask attempts, with 39 HTTP 429 Ask
failures. Optional harness request pacing was added and reviewed; an explicit
followup of those 39 provider-blocked cases passed 16/39 with no provider errors.
Combined diagnostic observations cover every Ask case ID and pass 34/115
(27/100 result pairs and 7/15 companions). These two attempts remain separate
reports and do not establish release acceptance. Public checks subsequently
passed 7/9 without provider failures.

The final evaluator rerun passed 24 tests, bringing the focused passing test
count to 95 with the previously verified CLI and file-source tests. Evaluator
Clippy, the evaluator/CLI build, formatting, and whitespace checks also passed.
Exact evidence, failure examples, and reproduction commands are recorded in
[semantic-eval-luna-results.md](semantic-eval-luna-results.md).

## Acceptance and release interpretation

The acceptance bank describes desired behavior. A compiler or interpreter
failure remains a failing case; there are no expected-failure waivers. The CI
job records failures and artifacts but does not block the overall build.
Local acceptance commands return nonzero on failure or incomplete evidence.

Passing focused tests or a filtered dataset run does not establish release
acceptance. Release requires complete SQL and live typed Ask coverage, at least
three repetitions, mandatory manifest-selected public interface checks, and
successful environment cleanup. Application-injected model providers are
diagnostic evidence and cannot qualify as live release evidence.

Version 1 explicitly fails scoped reference SQL and public-interface scope
forwarding until those adapters exist. The main Ask path enforces its authored
scope. This limitation is reported instead of silently ignoring a dataset's
request context. Public PostgreSQL smoke codecs cover the selected scalar and
temporal types; the main harness comparator additionally covers exact decimals.
