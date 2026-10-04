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

## Work still in progress

Checked integer arithmetic must prevent constant and column overflow in the
engine, including materialization sessions and federation placement. Executable
Ossie metric contracts and the catalog declarations needed by the remaining
semantic cases also require implementation and validation.

The additional dataset is the complete Formula 1 family from the canonical
[BIRD Mini-Dev release](https://huggingface.co/datasets/birdsql/bird_mini_dev):
66 tasks, 13 tables, and all 493257 source rows, routed through CSV and PostgreSQL.
This preserves a coherent database family without selecting cases by whether
the product passes them. Original source versions, annotations, and SQL are
retained. Independent review is identifying upstream annotation contradictions;
any corrected references must be recorded explicitly and checked independently
against SQLite and PostgreSQL. This derived suite is not an official BIRD
leaderboard score. Its authoring and acceptance verification are not complete.
