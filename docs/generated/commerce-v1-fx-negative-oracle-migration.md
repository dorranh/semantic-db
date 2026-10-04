# Planned versioned strengthening of two FX companion oracles

Status: proposed metadata erratum only; no artifact edit authorized or performed. Current commerce full Ask is running against a frozen bundle. This migration is pending exact-date lookup execution and public typed outcome support; the row-wise Decimal FX batch does not implement or satisfy it.

## Exact scope and stronger contracts

| Existing case | Historical expected outcome | Proposed required outcome | Required stage/code |
| --- | --- | --- | --- |
| reject.duplicate_rate | rejected | data_rejected | execution / non_unique_rate |
| unsupported.future_rate | unsupported | data_unavailable | execution / missing_exact_rate |

Both proposed expectations additionally require successful typed compilation and pinned exact-rate profile/guard provenance. Public field names and serializer version await generic API approval; do not invent artifact JSON fields before that contract exists. Mandatory execution stage and code are semantic requirements, not optional diagnostic substring matches. No other companion, paired result, question, SQL, numeric gold, tolerance, comparison setting or source fact changes.

The historical coarse outcomes can pass on compile-time refusal without performing the requested lookup. Strengthening closes that false-positive route; it is not an exemption or a relaxation. Preserve the old expected JSON bytes, case metadata, generator revision and artifact digest as historical provenance. Old reports keep their original meaning and cannot be retroactively scored against the stronger contract.

## Independent population oracle

Use the actual policy-visible rate relation under a pinned execution read, exact Utf8 currency equality and exact Date32 date equality. Null key components never match. Define N as the number of matching source rows, not distinct rate values. N=0 gives missing_exact_rate; N=1 permits exact conversion subject to separate rate-value/unit/arithmetic validation; N>1 gives non_unique_rate even if duplicate values agree. Never choose first/min/max or deduplicate by value.

The quarantined duplicate fixture has two EUR rows dated 2024-02-29, with rates 0.950000 and 0.960000: N=2 independently proves the rejection obligation. The production fixture has no EUR row on 2030-01-01: N=0 independently proves unavailability. The production EUR leap-day lookup has N=1 and provides a positive control. These counts are oracle evidence, not expected-answer material for model instructions or contract parameters. Reader-enforced physical types and source policies remain authoritative.

If a policy hides a row, count only rows visible under that same pinned read and disclose no hidden key/value content. Do not use a separate preflight count, stale cached count or a less restrictive access context. The generic tests need one-match, missing, duplicate-identical, duplicate-conflicting, null-key and policy-hidden populations, including the same-query guard behavior. Current canonical duplicates cover conflicting values only; identical duplicate coverage must be a real generic runtime fixture rather than a fabricated commerce observation.

## Compiler and runtime separation

The compiler validates the declared profile, request scope, currency/date/amount units, exact types and host parameters without consulting live data. Successful compilation does not assert that a rate exists or is unique. Execution performs the exact lookup and emits a typed sanitized SemanticDataCondition with the requested stage and code. Pin the authored profile revision, relevant fields, guard function revision and read boundary in compilation/execution provenance. A compilation Unsupported/Rejected response must fail both strengthened expectations.

Runtime/public adapters must preserve typed condition identity through engine wrappers, CLI/HTTP and PostgreSQL execution rather than infer it from messages. Generic execution_error, missing rate value, invalid/nonpositive rate, arithmetic overflow, provider failure, timeout, malformed proposal and unrelated diagnostics must not satisfy missing_exact_rate or non_unique_rate. A guard failure unrelated to the selected lookup must also fail. Error messages must not expose rate/key rows, credentials or SQL; provenance can identify the authorized profile and guard without leaking data.

## Authorized authoring sequence after implementation

1. Parent approves the exact generic expected-outcome schema, runtime adapters and public code names; implementer completes exact one-match execution and tests. Reviewer checks that row-wise-only success is not presented as lookup completion.
2. Analyst snapshots all file hashes, case questions/SQL/comparison settings, current expected bytes and generator version. Add immutable historical expectation copies and a visible oracle metadata erratum recording old/new outcomes, stage/code requirements and reason.
3. Change only the two expectation contracts and their generator emission, plus required manifest dataset-version/provenance fields. Preserve current IDs and questions. The former IDs retain historical names without overriding their stronger typed outcome semantics.
4. Regenerate into an isolated copy first and compare exact changes against the approved allowlist. Verify all paired numeric golds, SQL, tolerances, questions, schemas and canonical fixture data remain byte-identical. Publish pre/post manifest/model/whole-artifact hashes and historical baseline association.
5. Independently recompute N=0/1/>1 population controls under the intended policies; reviewer verifies the actual metadata delta and rejects accidental broadening or substring-only matching. Parent runs offline validation, full unchanged SQL and targeted typed Ask/public lookup checks.
6. Freeze the new artifact only with explicit versioned provenance and report its actual acceptance failures. Pending unsupported exact-lookup capability must remain failing until execution reaches the intended guard; no expected-failure flags or case exemptions.

Analyst owns later artifact authoring only after reviewer and parent approval. This document authorizes no migration, model hints, source enrichment or live-bundle mutation.
