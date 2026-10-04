# Acceptance polishing evidence

Active objective: fix every discovered acceptance failure, add another external
dataset usable through SQL and Ask, and fix the failures exposed by both suites.
Work remains on `feature/semantic-eval-acceptance`, with incremental commits.

The original commerce baseline and all 81 failing Ask IDs are recorded in
[semantic-eval-failure-ledger.md](semantic-eval-failure-ledger.md). None of those
cases is waived. Original reports remain available separately from subsequent
attempts and artifact revisions.

## Gregorian dates and request evidence

Commit `0e54d05` adds explicit `gregorian_date` intent literals. Binding validates
YYYY-MM-DD and produces Date32 independently of timezone. SQL artifacts expose
canonical Date32 parameters, preserving existing external codecs. Invalid dates
have input/proposal diagnostics rather than an internal-error classification.
Strict source-span validation remains in place; repair diagnostics identify the
numeric requirement position and UTF-8 byte ranges without exposing request text
or model identifiers. Presentation instructions default unspecified sort
direction to ascending and prohibit projecting fields requested only for filters
or ordering. Compiler pipeline revision is 33.

The parent orchestrator ran:

- `cargo test -p semantic-compiler -p semantic-interpreter --locked --offline`:
  247 tests passed across 64 targets, with no failed or ignored tests. Log:
  `/tmp/semantic-eval-date-span-tests.log`.
- `cargo clippy -p semantic-plan -p semantic-compiler -p semantic-interpreter
  --all-targets --locked --offline -- -D warnings`: passed. Log:
  `/tmp/semantic-eval-date-span-clippy.log`.
- `cargo build -p semantic-eval -p semantic-cli --locked --offline`: passed.
  Log: `/tmp/semantic-eval-date-span-build.log`.
- Formatting and whitespace checks passed.

Build/test/Clippy commands used two jobs and disabled incremental compilation.
Implementer and reviewer agents used GPT-6.1 Sol and did not run Cargo.

A focused live run used GPT-6 Luna, automatic context, a 120-second case deadline,
and 12000 milliseconds between in-process model calls. All eight selected SQL
cases passed; five of eight Ask cases passed. Provider errors were absent and
environment cleanup succeeded.

| Case | Ask observation |
| --- | --- |
| `calendar.leap` | Passed |
| `roles.both` | Passed |
| `projection.negation` | Passed |
| `calculation.tax` | Passed |
| `relationships.products` | Passed |
| `concept.active` | Clarification about retaining subscriptions with unmatched customers |
| `concept.zero_active` | Refusal because the active-membership rule was not exposed as an executable concept |
| `metrics.minmax` | Request-span rejection after bounded repair |

Report: `.semantic-eval/commerce-polish-date-span/run-188512-18db56970db88cb5/report.json`.
Artifact digest:
`3bf6bebb5649e527921b0beb13c93fff0b815a4d7496ea90f87e50538bc5bb88`.
This is filtered development evidence, not full-suite or release acceptance.

## Commerce question precision

Independent review found seven questions that under-specified their intended SQL
oracles. The question bank and authoring script now agree on:

- Exact FR region code.
- Exclusion of unknown billed clients and unmatched subscriber records in the
  specific queries whose SQL uses inner joins.
- Zero refunds for every line of the requested order.
- Highest completed order cents, with the smaller ID breaking ties, for the
  top-per-buyer window case.
- Explicit billed/subscriber identities and the year 2024 for active-membership
  set operations.

Only question strings changed in the case bank. SQL, expected results, comparison
rules, tags, and case coverage remained unchanged. The live artifact digest
above includes this wording revision; original baseline reports retain the
original artifact digest.

## Checked integer arithmetic

The engine rewrites signed and unsigned integer addition, subtraction and
multiplication, and signed unary negation, after native type coercion and before
constant folding. Arrow checked kernels report overflow instead of wrapping.
The rule applies to normal and materialization sessions, preserves native
nullability and unary field metadata, and marks its functions local for
federation. Floating-point and decimal arithmetic keep their existing behavior.
The execution profile is `semantic-datafusion-typed-v12`.

The parent ran the complete engine/compiler test suites: 205 tests passed across
68 targets with no failures or ignored tests. After the final unary metadata
correction, all nine focused arithmetic/materialization tests passed. All-target
Clippy for both crates passed with warnings denied. Logs:
`/tmp/semantic-eval-checked-integer-broad.log`,
`/tmp/semantic-eval-checked-integer-focused-final.log`, and
`/tmp/semantic-eval-checked-integer-clippy.log`.

These commands used two jobs, disabled incremental compilation, and set
`CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`. Debug assertions remain
enabled. A preceding build exhausted local disk before tests ran; targeted Cargo
cleanup freed 66.7 GiB. That failed build provides no test result.

## Authored Ossie contracts and terminal reports

The importer preserves authored metric lookup permissions, state, rollup
dimensions, row filters and explicit result types. It accepts bounded field-only
MIN/MAX/AVG expressions, business-calendar contracts and parameterized concepts.
Named Float32/Float64 measurement units are supported; floating currency tags
remain rejected. Date32 concept parameters accept validated Gregorian dates and
legacy Date32 literals without accepting ordinary strings. Compiler pipeline 34
separates the new acceptance boundary from prior cache and replay artifacts.

All 258 Ossie/catalog/compiler tests passed across 82 targets. After the pipeline
revision change, ten prepared/cache/replay tests passed. Logs:
`/tmp/semantic-eval-ossie-advanced-all.log` and
`/tmp/semantic-eval-ossie-pipeline34-tests.log`.

The harness separates decoded execution admission from collected output limits,
records effective budgets and planned attempt identities, writes reports
atomically, and keeps running reports unfinalized and incomplete. All 26 evaluator
tests passed, including inspection of a snapshot while a second model call is
blocked. All-target Clippy for Ossie, catalog, compiler and evaluator passed with
warnings denied. Evaluator/CLI build, formatting and whitespace checks passed.
Logs: `/tmp/semantic-eval-lifecycle-budget-tests.log`,
`/tmp/semantic-eval-advanced-harness-clippy-final.log`, and
`/tmp/semantic-eval-advanced-harness-build.log`.

The reviewed commerce catalog exposes exact mean state, completed MIN/MAX,
lookup role permissions, parameterized active membership and distinct date/UTC
Swiss calendar rules. Case questions, SQL, golds and canonical source data are
unchanged from the wording commit. Its live SQL run passed **103/103**, including
all three error expectations, with successful setup/cleanup and artifact checks:
`.semantic-eval/commerce-sql-v12-p34/run-244568-18db5ace2f629413/report.json`.
Digest: `9c8779aaa036a3640119725ac28b4f429adfaa26736037bddd2fca285da08c53`.
This verifies the integer-overflow fix with the real mixed-source project.

## Work still in progress

Remaining commerce Ask failures require implementation and fresh full-suite
verification. A bounded model evidence recovery patch is awaiting verification;
direct compiler span checks remain authoritative.

The additional dataset is the complete Formula 1 family from the canonical
[BIRD Mini-Dev release](https://huggingface.co/datasets/birdsql/bird_mini_dev):
66 tasks, 13 tables, and all 493257 source rows, routed through CSV and PostgreSQL.
This preserves a coherent database family without selecting cases by whether
the product passes them. Original source versions, annotations, and SQL are
retained. Independent review identified upstream annotation contradictions;
the explicit corrections are recorded and all 66 typed golds have been checked
independently against SQLite and PostgreSQL. This derived suite is not an official
BIRD leaderboard score. Its authored bundle is reviewed and frozen.

The first product SQL run attempted all 66 cases: 46 passed, 16 exceeded the
decoded-byte budget, and four returned mismatched results. All 13 row-count
fixtures, setup, and cleanup succeeded. The report is incomplete because resource
exhaustion is not a completed semantic outcome:
`.semantic-eval/bird-sql-v12/run-209884-18db58aace290bb1/report.json`.
Artifact digest:
`9c2b85d1a3abbb899a31e26a758a60ccc3b66afbd83ef7be2cc9fb5fa4695e62`.

Independent investigation found that the harness conflated its 32 MiB collected
output limit with the engine's decoded scan limit; the product default is 1 GiB.
The four result differences are reference dialect adaptations: PostgreSQL date
format tokens are literal text in DataFusion's Chrono formatter, and REAL casts
produce Float32 intermediates while the SQLite reference computes Float64.
Reviewed adaptations use explicit year/month extraction and DOUBLE PRECISION.
Questions, expected rows, and tolerances remain unchanged. Original reports and
bundle digests remain preserved.

The initial full Luna Ask run attempted all 66 tasks: eight passed, 17 had provider
failures, and 41 had other failures. Setup, cleanup and artifact checks succeeded.
Report: `.semantic-eval/bird-luna-v12/run-210719-18db58ba059a4381/report.json`;
SHA256 `4aa0af473cd81ed390649dc071419c14b75f1766842905a93579c116dcf41d71`.
It used the original BIRD artifact digest above, pipeline 33 and profile 12, with
12000-millisecond pacing. It is incomplete and cannot establish full semantic
coverage. Individual calls reported approximately 66700 input tokens.

The next SQL run against version 1.0.1 passed 57/66:
`.semantic-eval/bird-sql-v12-p34/run-244569-18db5ace4ed2c19a/report.json`;
digest `887735e2d5cb2416eadc82e41c7fb75f7c4d8376dc54b8968ead4135d36ce074`.
Four reference mismatches previously masked by resource exhaustion remained.
A full-family audit found six remaining REAL casts and one date-format call;
version 1.0.2 adapts these explicitly, preserving all gold files byte-for-byte.
The other five failures exhausted the product's 256-request budget: streaming
400524 lap records in 1024-row PostgreSQL portal fetches requires roughly 392
actual requests. Dataset-owned, recorded execution limits and a fresh SQL run
remain required. None of these failures is waived.

## Full SQL evidence after portability, capacity and role vocabulary repairs

Fresh real mixed-source runs are complete and finalized with full coverage:
commerce passed103/103 SQL-bearing cases, including all three intended error
companions, and BIRD passed66/66 required tasks. Both reports have no setup,
cleanup or artifact errors. Reports:
`.semantic-eval/commerce-sql-roles/run-251955-18db5cb4c88a0f55/report.json`
(artifact `79bc641044762043ad84c5eee984cc299c7f55425753c505f028be46be993cf0`)
and `.semantic-eval/bird-sql-capacity/run-251915-18db5cb4a0764b64/report.json`
(artifact `1a88e121488ce099e5ea97cc9efcdba90edf7d2b56c58dfadc3b1569ca550c01`).
The request capacity is authored for the entire BIRD artifact, with real provider
fetch accounting and unchanged batch sizes. Readonly regeneration in temporary
copies reproduced every file in both frozen bundles byte-for-byte; all BIRD
source hashes matched and no bytecode files were generated. Previous SQL/Ask
failures remain historical evidence. Fresh Ask verification and remaining
semantic-profile implementation are still required; SQL success is not an Ask
waiver.
