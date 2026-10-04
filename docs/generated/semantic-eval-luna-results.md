# GPT-6 Luna acceptance observations

Date: 2026-10-04. Branch: `feature/semantic-eval-acceptance`.

The live interpreter was run with `--model gpt-6-luna`, using the configured
OpenAI API credential. Implementation and review agents remained GPT-6.1 Sol;
the parent orchestrator ran all Cargo commands. Credentials were not changed.

The unchanged commerce bundle has harness SHA-256
`b7f30c2e568dd1640a33f55bb0c0734fb709badf0898846de6fa5e01d01e367a`.
All attempts used automatic context selection and a 120-second case timeout.
The dataset supplies its fixed request clock, timezone, Compose stack, and
bootstrap. Startup, fixture checks, artifact integrity, and cleanup succeeded.

## Results

| Attempt | Passing observations | Completeness |
| --- | --- | --- |
| Tiny live Luna preflight | 1/1 Ask | Complete |
| Full commerce SQL | 102/103 | Complete SQL observations; all 100 result pairs passed |
| Full commerce Ask | 18/115 | Incomplete: 39 cases returned HTTP 429 |
| Full-run public checks | 4/9 | Five public calls ended in provider failure |
| Paced followup of exactly the 39 HTTP 429 Ask cases | 16/39 | Complete selected observations; zero provider errors |
| Separate public-interface followup | 7/9 | No provider failures; two acceptance failures remain |

Across the original valid Ask observations and the transport-only followup,
34 of the 115 case IDs passed: 27/100 result pairs and 7/15 companion outcomes.
All 115 IDs now have a non-provider outcome. This is a diagnostic summary of
two attempts, not a passing full run or release qualification. The original
failed attempts remain intact; semantic failures were not selectively rerun.

The combined observed Ask outcomes were 30 results, 23 clarification requests,
56 unsupported responses, three unresolved responses, and three rejections.
Outcome counts differ from passes because each case must match its authored
result or companion expectation. Three executed results failed comparison.

The full main run made 122 model calls: 83 completed and 39 failed. The paced
followup made 44 calls, all completed; additional calls include the interpreter's
existing repair/context behavior. No transport retries were added.

## Pacing and verification

An optional `--model-request-interval-ms` harness setting was added after the
initial rate-limited run. Its default is zero; values above 60000 are rejected.
One shared gate spaces every in-process model call across cases, repairs, and
context expansions. Gate waiting counts toward existing deadlines and reported
latency. Public subprocess calls are unaffected.

The followup used a 12000-millisecond minimum start interval. Its case latency
median was 12.05 seconds and maximum 32.18 seconds, including gate waits. Its
report declares `full_coverage=false`; it cannot certify release acceptance.
The retry selection was frozen in `retry-plan.json` before execution and
contains only original Ask cases with an HTTP 429 provider error.

Review also found that compiler deadline/cancellation/resource-limit diagnostics
could be reported without marking the attempt incomplete. The implementer
corrected that classification and added a cancellation regression.

All 24 evaluator tests passed, including shared pacing, retained HTTP 429
failures, cancellation before a second provider call, interval bounds, and old
report compatibility. Evaluator Clippy with warnings denied, the evaluator/CLI
build, formatting, and whitespace checks passed. Earlier verification also
passed 52 CLI and 19 file-source tests. Hosted CI was not run locally; its
informational failure policy remains unchanged.

## Concrete failing requirements

Every failure remains acceptance work; none was waived or converted to an
expected failure. Representative retained evidence:

- `error.overflow`: reference SQL returns signed bigint `-2` for maximum bigint
  multiplied by two, while the gold requires an execution error. Paced Ask
  calls multiplication unsupported.
- `calendar.leap`: Ask requests a Date32 epoch-day count from the user instead
  of interpreting February 29, 2024. The gold expects order 104 and buyer Chen.
- `absence.buyers` and `absence.no_subscriptions`: responses refuse the reverse
  relationship traversal needed to identify related absence.
- `metrics.average` and `fanout.weighted_price`: responses refuse the specified
  exact financial rounding. Their golds remain unchanged.
- `concept.purchases`: the executed result adds a third column, includes an
  unknown client, and does not follow customer-ID ordering.
- `metrics.customer_count_zero`: the paced result includes zero-order
  customers but fails the required row ordering.
- `relationships.products` and `metrics.minmax`: successful provider responses
  fail request-span validation before execution. The diagnostic says requirement
  text must exactly equal its source spans joined by a space.

The public followup passed all three CLI SQL checks, two CLI typed Ask checks,
and two HTTP typed-compilation-to-PostgreSQL execution checks. Billing HTTP
compilation was rejected by request-span validation. Previous-month CLI Ask
requested clarification about preserving orders with missing payer records;
the corresponding HTTP execution passed. Both failures remain recorded.

These observations identify investigation targets without assigning root cause
to a particular layer. The report retains compilation, interpretation, request
context, token accounting, comparisons, and sanitized provider diagnostics.

## Retained local artifacts

Paths are relative to the repository root. Run artifacts are ignored by Git.

- Tiny Luna preflight: `.semantic-eval/luna-preflight/run-140131-18db4d67ab4b5ed5/report.json`.
- Full run: `.semantic-eval/commerce-luna/run-140456-18db4d78c2641149/report.json`.
- Frozen followup plan: `.semantic-eval/commerce-luna/retry-plan.json`.
- Paced followup: `.semantic-eval/commerce-luna-paced/run-162605-18db5450b1357cb4/report.json`.
- Public followup: `.semantic-eval/commerce-luna-public-followup/run-175403-18db54ebf54ea47e/report.json`.
- Combined diagnostic summary with source report hashes: `.semantic-eval/commerce-luna/observations.json`.

Each run directory retains environment lifecycle evidence and logs. Public
attempts also retain command logs, compilation artifacts, and observed JSON.
The summary does not overwrite or alter either source report.

## Reproduction

With Docker access and the existing local `.env`, run the full SQL and paced Ask
interfaces separately:

```sh
target/debug/semantic-eval run --dataset tests/datasets/commerce-v1/manifest.json --interface sql --artifacts .semantic-eval/luna-sql
target/debug/semantic-eval run --dataset tests/datasets/commerce-v1/manifest.json --interface ask --model gpt-6-luna --env-file .env --timeout-seconds 120 --model-request-interval-ms 12000 --artifacts .semantic-eval/luna-ask
```

The current shell needs `sg docker -c '<command>'` because its inherited groups
predate Docker membership. Add `--public-interfaces --cli-binary
/home/dorranh/dev/semantic-db/target/debug/sdb` for public checks; those model
calls are not paced. Local commands retain nonzero exit status on acceptance
failure. Release qualification requires the full repeated run described in
`semantic-eval.md`.
