# Bounded model evidence recovery

This is a read-only design review, not a claim that recovery is implemented or tested. The strict compiler evidence contract remains authoritative.

## Insertion point

In `semantic-interpreter/src/typed/mod.rs`, decoded `Intent` and `GraphIntent` proposals are converted to row/graph proposals with evidence attached to compiler options. Recovery belongs inside that conversion after exact `original_request` validation and host-ledger comparison, before evidence is attached. Corresponding host-supplied evidence disables recovery entirely. Legacy proposals and direct compiler APIs retain existing behavior.

The model response retained in conversation and accounting stays byte-identical. Recovery changes the interpreted evidence envelope only; it must not change operations, aliases, source text, IDs, original request, unresolved alternatives, evidence version or request identity.

## Eligibility

For an existing row requirement entry or graph node/leaf entry with source text:

1. Preserve spans already valid under the compiler's nonempty, ordered, nonoverlapping, UTF-8-boundary, non-whitespace, exact joined-text checks. Valid discontinuous evidence is not collapsed.
2. For invalid spans, require non-whitespace source text that appears exactly once as a contiguous substring of the original host request.
3. Replace only those existing spans with one half-open UTF-8 byte range covering that exact unique substring.
4. Run the unchanged strict compiler validator on the resulting envelope.

Missing entries, unknown graph targets, duplicate evidence, empty or paraphrased source text, repeated phrases, unmatched text, and graph outputs/order/limit evidence without a source-text field are not repaired. They follow existing rejection/model-repair behavior. Recovery does not establish semantic completeness or correct interpretation.

## Uniqueness and resource bounds

Uniqueness includes overlapping occurrences. Rust `str::match_indices` alone is insufficient: `aa` appears twice in `aaa`, although nonoverlapping iteration reports one occurrence. Search from successive UTF-8 character boundaries, stopping at the second occurrence. Do not normalize case, whitespace or Unicode. Exact byte equality is the contract.

Bound entry visits and searches with the existing node and byte budgets. Charge normalization work, including unchanged entries, to a bounded work counter; check cancellation/deadline during traversal and searches. Abort resource exhaustion with the existing incomplete/resource outcome. Do not allocate joined strings beyond authored/model input limits or expose arbitrary IDs in normal diagnostic records.

## Provenance

Record a normalization event per changed entry: model-attempt number, numeric requirement/evidence ordinal, previous and replacement numeric byte ranges, and a fixed normalization reason. Retain a total normalization count/work accounting. No request substrings, requirement IDs, graph node IDs, literal values or model responses belong in ordinary records. The unchanged original model output remains available only through existing explicitly sensitive capture mechanisms.

A subsequent model repair still receives the unchanged prior response plus safe diagnostics and its existing exact original request. No implicit provider retry or extra model request is introduced by deterministic recovery.

## Required regression coverage

- Unique ASCII text with character-count or off-by-one offsets recovers and executes.
- Multibyte Unicode source text recovers to exact byte offsets.
- Already-valid one-span and discontinuous spans remain byte-identical.
- Repeated, overlapping (`aaa`/`aa`), missing, paraphrased and whitespace-only text does not recover.
- Missing row entries and graph output/order/limit entries are not fabricated.
- Unknown and duplicate graph targets remain rejected.
- Host ledgers remain unchanged and mismatches remain rejected.
- Query/operations/source text are unchanged; model response/token/call accounting is unchanged.
- Row and graph normalization events identify numeric positions without leaking private text or IDs through serialization, Display, Debug or tracing.
- Budget/cancellation failures remain incomplete and cause no additional provider call.
- Direct compiler invalid evidence remains rejected even for uniquely occurring text.

## Following capability priorities

The commerce ledger separates offset errors from semantic feature gaps. After evidence recovery, prioritize reverse existence/absence and related concept scoping, composite relationship paths, compatible metric lookup dimensions and independent fact alignment. Then add faithful executable history/calendar/active-subscription profiles, exact rounded ratio/conversion/allocation operations, and the requested windows/set/Boolean composition. Missing executable declarations require authored profiles and compiler support; neither prose-only rules nor relaxed expected outcomes establish acceptance success.
