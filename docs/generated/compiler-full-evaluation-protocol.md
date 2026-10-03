# Full compiler evaluation protocol

This protocol is recorded before any live-model release scoring. The current
scripted fixtures check compiler orchestration and independent expected rows;
they do not estimate interpretation accuracy. No live-model credentials were
available at the batch-31 checkpoint.

## Hold-out construction

Use at least 100 independently labeled requests from at least five domains,
with distinct phrasing from implementation fixtures. Each case records the exact
request, catalog and source rows, scope, request clock, sufficient required facts,
acceptable requirement ledger and interpretations, expected rows or diagnostic,
and a critical-failure flag. Include governed policies, ambiguous definitions,
fan-out, billing/shipping roles, missing versus null groups, composite keys,
calendar/DST boundaries, conversion, allocation, wrong grain/coverage, denied
scope, context exhaustion and prompt injection. Keep test cases fixed during a
release scoring run; new failures join the next hold-out version.

## Runs and comparisons

For each request, evaluate full compact context when it fits, retrieved compact,
and an independently curated sufficient context with the same model version,
temperature, prompt, output schema, call limits and reference clock. Record when
full context cannot fit; a truncated projection is not a full baseline. Run each
mode three times in randomized order and retain provider/model configuration,
input/output token usage or explicit missing-usage status, elapsed time, model
calls, expansions, selected facts, requirement dispositions, final outcome and
exact rows. The deterministic scripted-provider runner remains a separate gate.

Score correct resolution, incorrect acceptance, false refusal/clarification,
requirement coverage, query-level recall of **all** required facts, expansion
recovery, and cost per correct resolution. Report numerators, denominators and
variation by domain and mode, not just a pooled percentage. A known policy,
scope, ambiguity or fan-out violation that produces accepted wrong rows is a
critical incorrect acceptance.

## Provisional release gate

These thresholds are chosen before live scoring and can be revised only for a
subsequent hold-out version with the decision recorded. Across the minimum 300
runs per mode: zero critical incorrect accepts; at least 90% correct resolution;
at least 95% complete-required-fact recall for retrieved context; and no more
than 5% false refusal/clarification on cases with an executable answer. Retrieved
mode must not lose more than two percentage points of correct resolution versus
full compact on the paired cases that fit. Report uncertainty and all misses;
passing a percentage never waives a critical failure. These are quality gates,
not latency or cost service-level objectives.

Before a production rollout claim, run a separately authorized shadow sample
with the same critical-failure check and bounded extra model work. Keep full
compact as the rollback selection policy where it fits. Until credentials,
approved representative data and this protocol are available, report only the
offline scripted and deterministic execution evidence.
