# Exact rate lookup and typed execution outcome design

Status: proposal only. Production sources and both dataset artifacts remain frozen while workspace verification runs. The existing rowwise `convert_rate` operation and legacy rational conversion are unchanged by this document. The reviewed rowwise slice supports literal major amounts, `RateSourceCurrency`, exact decimal result controls and an authored exact-rate profile. It does not implement this lookup or these outcome adapters.

## Closed request operation

Add a separate row operation rather than changing the population meaning of `convert_rate`:

```json
{
  "kind": "convert_rate_exact",
  "rate": "authored_profile_name_or_id",
  "selector": {
    "currency": {"type":"utf8","value":"USD"},
    "date": {"type":"gregorian_date","value":"2024-04-01"}
  },
  "amount": {
    "kind":"literal",
    "value":{"type":"decimal128","value":{"coefficient":"100","precision":3,"scale":0}},
    "unit":{"kind":"rate_source_currency"},
    "basis":"major"
  },
  "result_type":{"precision":18,"scale":2},
  "rounding":"half_away_from_zero",
  "alias":"converted"
}
```

`ExactRateSelector { currency: Literal, date: Literal }` has `deny_unknown_fields`. Binding accepts only Utf8 currency and Date32/GregorianDate date. GregorianDate is validated and canonicalized to Date32 before selector identity and SQL parameter serialization. Currency is an exact authored source identifier; this does not assert an ISO domain or case-insensitive comparison. Explicit null keys, field selectors, intervals, nearest-date selection, host amount placeholders, minor amounts and source-fact amounts are absent from this first lookup API.

The row input must equal the published profile's rate relation. The first profile is terminal: exactly one `convert_rate_exact` requirement, with policies added by binding; no caller-supplied grouping, filter, related, order, limit or projection can silently narrow the match population or conceal ambiguity. Its output is one requested decimal amount, in the profile's target major currency, with conservative nullable metadata matching the reviewed quantizer contract. The source is still the genuine rate relation. The one-row output arises from a global aggregate over that relation; no synthetic fact, request table or invented rate row is introduced. A graph can combine independent terminal leaves if existing composition contracts permit it.

The binder applies its normal node/depth/deadline limits, resolves one exact profile, validates relation and physical fields (including existing exact string comparison-profile restrictions), binds the two exact comparisons, and adds all row policies before the summary. Currency/date must remain physically nonnull Utf8/Date32. The rate may be nullable under the existing explicit policy. Literal amount/result/rounding validation reuses the reviewed rowwise code. The relation, selected field schemas, policy references, profile reference, guard function and quantizer function are pinned. Scope checks cover the actual rate read. The output has `Currency(target)` meaning and major-unit basis, with exact result scale in Arrow metadata.

## One matching population and one runtime summary

Lower to the following private sequence:

1. Read the pinned rate relation once in the query plan.
2. Apply its row policies and the exact currency/date selector.
3. Globally summarize that same matching stream, retaining count and at most one rate coefficient.
4. Check cardinality and rate validity at execution, then quantize the literal amount once.
5. Project only the requested output.

Use a private checked aggregate state `(match_count, first_rate)` with checked count merge. Keeping `first_rate` is permitted only as state; it is never a selection policy. Final evaluation must validate count before exposing a rate. Each matching row counts, including null-valued rates and duplicate identical rates. State size is constant. Merge combines counts and one retained value; order and partitioning cannot affect the outcome.

A single closed local aggregate is preferable to independently scanning for COUNT and a selected rate. It provides the logical global COUNT plus rate summary from one admitted population. It does not use MIN/MAX/AVG, ignore null rates in the count, deduplicate, or issue a preliminary count query. If a count-plus-rate plan uses multiple aggregate expressions, they must share the same aggregate input and execution, with a verified plan invariant; two separately executed plans are prohibited.

Final evaluation is:

| Matching rows | Result |
|---|---|
| 0 | `MissingExactRate` / `missing_exact_rate` |
| greater than 1 | `NonUniqueRate` / `non_unique_rate`, even when all rates are identical |
| 1, null rate | `RateValueMissing` / `rate_value_missing` |
| 1, nonpositive or invalid declared decimal coefficient | `InvalidRate` / `invalid_rate` |
| 1, valid positive rate | Existing checked decimal quantizer |

Cardinality has precedence over rate-value validity, so two null/invalid rows still represent a nonunique match. Arithmetic overflow remains its distinct genuine execution error. Source, transport, cancellation and resource-limit failures remain distinct and cannot be converted into any of these four conditions.

The compiler must not inspect source data, execute this aggregate during binding, or classify empty/duplicate matches from catalog metadata. The guard must execute even for a predicate that the optimizer reduces to an empty relation. A global empty-input aggregate still emits its execution failure. If an optimizer folds the guard into a planning failure, that is a defect; declaring the request unsupported is not a substitute. Tests must require successful compilation before each data condition.

“One read” means this query's single policy-visible matching population. It does not claim a transaction snapshot across unrelated connectors or queries. Existing engine query admission, cancellation and source consistency contracts remain in force.

## Typed failure and actual guard origin

Keep the existing static `SemanticDataCondition` code enum in semantic-runtime. Add a typed error envelope:

```rust
pub struct SemanticDataFailure {
    pub condition: SemanticDataCondition,
    pub origin: Option<SemanticDataOrigin>,
}

pub struct SemanticRevisionRef {
    pub id: String,
    pub revision: String,
}

pub struct SemanticDataOrigin {
    pub profile: SemanticRevisionRef,
    pub guard: SemanticRevisionRef,
    pub selector_digest: String,
}
```

`SemanticRevisionRef` is owned by semantic-runtime and has the same `{id, revision}` wire shape as catalog ObjectRef. The compiler explicitly copies a checked catalog reference into it; runtime does not depend on catalog. All three public envelope structs derive Clone/Eq/serde, with unknown fields rejected on decoding. These types are error/report contracts, not untrusted proposal authority. `SemanticDataCondition::category()` supplies the closed Unavailable/Rejected category rather than accepting another independently mutable category string.

The guard reference uses compiler-owned structural occurrence identity (graph node ordinal and row requirement ordinal, or an equivalent canonical path), plus a revision digest of the sealed guard descriptor. It contains no model-supplied IDs, aliases, request clauses, selector values, SQL or row content. The descriptor pins profile, relation/field/policy references and the canonical selector digest. Its digest can also include the operation's checked result controls; private bound state retains full values for execution.

The aggregate instance that actually fails must capture this origin. The quantizer used by that checked instance must retain the same origin for rate-value errors. Capturing a profile that merely appears in the compilation's definition list is insufficient: two guards can share a profile and have different selectors; several profiles can occur in one graph. Wrapping an unidentified static error with the first profile from the query is prohibited.

Preferred implementation boundary: construct a private aggregate instance with a bound origin, and install it in a request-local function registry used consistently by native and generated SQL planning/execution. The registry associates compiler-owned function occurrences with sealed descriptors, and cannot pollute the engine's global namespace or outlive its request. The engine adapter must preserve the existing session configuration, materializations, federation planner, cache and authorization. Native and generated SQL paths construct the same aggregate instance from the same private binding. This requires a small reviewed engine planning adapter before implementation.

A raw SQL call to a generic public guard may still produce a typed static condition with `origin: None`; it cannot claim an authored profile receipt. Do not accept arbitrary SQL string arguments as trusted profile/guard origin. A possible fixed-name implementation with opaque occurrence tokens is acceptable only if the request-local registry validates those tokens against sealed bindings before creating an accumulator. A token supplied by untrusted SQL alone is not provenance.

## Preserve typed errors before display conversion

Expose one semantic-runtime extractor for an error/source chain, returning a cloned typed failure. It must recognize the concrete envelope through `DataFusionError::External` and nested DataFusion contexts/sources. It must not infer a condition from Display strings, SQLSTATE alone, aliases, diagnostic substring matching or the existence of compiled profile references. Generic errors that imitate the safe message remain generic errors.

Several existing boundaries erase type information and need explicit adapters:

- `typed/lower.rs::backend_error` currently accepts Display and returns a generic compiler diagnostic.
- `CompiledQuery::execute` and graph execution convert engine errors into compiler diagnostics; stream errors occur later while collecting batches.
- Evaluator `record_execution` currently handles an anyhow result and classifies generic execution failures.
- PG protocol currently maps stream errors to a safe generic message.

Extract before each conversion. For compatibility, an additive optional execution-failure field on `CompileDiagnostic` can retain the typed envelope where existing public execute methods must continue returning that type. Display/Debug remain safe, and compiler-phase failures cannot masquerade as execution-stage semantic conditions. A cleaner separate typed execution error API is an alternative, but must preserve callers and be reviewed before widening this batch.

The extractor must cover both execute-start and later stream failures. Preserve cancellation/resource errors as incomplete failures, including when a semantic failure occurs after earlier batches. Never emit partial rows as a successful response. Existing compile diagnostics stay compile diagnostics; no provider refusal or malformed proposal is a semantic data condition.

## Default report and precise expected outcomes

Successful typed compilation records a public guard manifest, independent of debug capture. Each entry includes its compiler-owned guard reference, profile reference, selector digest and pinned relation/policy references. This is the checked plan's receipt, not SQL or a model transcript.

Default `CaseReport` retains:

- compilation status, explicitly distinguishing `compiled` from unavailable/failed/not requested;
- execution stage/status;
- the optional typed semantic data failure and its actual guard origin;
- checked guard manifests for successful compilation.

New optional fields use serde defaults so old reports remain readable. An absent receipt is not inferred from an old record or a profile reference list. Error adapters must preserve these fields on stream failure, timeout, capture-off runs and public interface attempts.

Add closed expected outcomes `data_unavailable` and `data_rejected` with required `stage: "execution"`, exact canonical condition code, exact profile `{id, revision}`, and canonical selector digest. The selector digest is independently computed from authored expected currency/date literals, with GregorianDate canonicalized to Date32. This prevents an arbitrary different absent date from satisfying a missing-rate negative. It is oracle metadata only and is never provider input.

A case passes only when typed compilation actually succeeded, execution reached the corresponding guard, the exact condition/category/profile/selector match, and the actual failing guard reference is present in that successful artifact's checked manifest. The actual error must carry the guard's origin. A missing origin, merely present profile reference, wrong selector, wrong guard revision, unrelated SQL error, provider error, parse error, overflow, budget exhaustion or compile-time Unsupported/Rejected fails. Resource-limited or cancelled attempts remain incomplete and nonpassing.

For reference SQL attempts, do not fabricate typed compilation success. The first strengthened companions can be Ask-only. A later generic SQL negative contract must explicitly define whether it requires a typed receipt; raw SQL static conditions cannot automatically satisfy the stronger authored-guard oracle.

## Public interfaces

Typed HTTP compilation still returns a successful compiled artifact before data is read. It must not report missing/nonunique rates based on compilation alone.

For machine CLI execution, after successful compilation and before printing rows, return a nonzero error envelope containing the safe category, canonical code, execution stage and optional checked origin. Human output uses static sanitized messages. Plain SQL execution can report an unqualified typed condition but must not claim typed compilation or authored guard provenance.

HTTP query execution returns a safe structured failure envelope with the same fields and a non-success status; compilation endpoints remain successful for these valid requests. Do not confuse query execution data errors with model or provider failures.

PG execution retains static safe messages and adds canonical code/origin in sanitized structured DETAIL. Suggested SQLSTATE mapping is `P0002` for missing exact rate/rate value, `21000` for nonunique rate, and `22000` for invalid rate. These mappings require protocol review; they do not alter existing overflow, cancellation or budget mappings. Strong public checks require compile-success proof plus code and receipt matching, not SQLSTATE alone. HTTPcompile→PGexecute uses the PG failure receipt to identify the actual runtime guard. No key values, SQL, credentials, raw provider errors or internal row/source content are included.

All three effective execution budgets and timeout continue to flow through CLI/server/PG. A restrictive budget cannot be converted into missing-rate data unavailability. Public clients must buffer or otherwise avoid declaring a successful complete result before the guard and all source reads finish.

## Verification and later artifact migration

Independent engine fixtures cover N=0,1,2; duplicate identical values; null rate; zero/negative rate; declared coefficient violations; positive signed amount ties; overflow; zero and null key handling; policy-hidden and policy-visible matches; multiple partitions and merges; and a genuine empty source. A query with no matches must compile before runtime MissingExactRate.

Compiler tests cover exact typed selector parameters, scope, field/profile revisions, stale inner policy/profile replay, closed unknown forms, native/generated SQL bag and schema parity, target currency meaning, no extra filters/limit/distinct, and admission/cancellation. Multiple failing guards test origin attribution, including two occurrences of the same profile with different selectors and two profiles in one graph. Reversing node order must not attach the other guard's origin.

Adapter tests use typed errors nested in DataFusion and anyhow/source contexts, typed stream failures, generic errors with identical Display text, cancellation and limits. Capture-off default reports must include compile success and exact guard receipt. CLI JSON, HTTP compilation plus query execution, and PG SQLSTATE/DETAIL tests require the actual safe structured metadata and no partial success.

Only after actual execution guards, typed adapters, public parity and strict harness matching are implemented and reviewed may the analyst update the two negative companion metadata entries: future exact-date rate to `data_unavailable/missing_exact_rate`, duplicate exact-date rate to `data_rejected/non_unique_rate`. Record previous and new outcome contracts and retained old reports as an oracle-strengthening erratum. Preserve NL, result golds, numeric tolerances and source facts. Do not move a compile-time refusal into a passing runtime category. Parent/reviewer approval and independent analyst verification are required; this document alone authorizes no artifact change.

## Receipt lifetime across HTTP compilation and PG execution

Review identified a blocking lifecycle seam: a request-local guard registry cannot disappear after HTTP compilation and then magically supply trusted origin to a later PG request. Generic generated SQL alone is not a sealed artifact, and rebuilding origin from its function names/arguments is prohibited. The following is the concrete proposed lifecycle; its server/API implementation requires separate approval before public lookup parity is claimed.

An in-process `CompiledQuery`/`CompiledGraph` owns immutable private guard bindings. Its execute method installs those bindings into a scoped copy of the existing engine session state for that execution, preserving all engine configuration and current query options. It does not register request functions globally. Replay rebuilds the binding through the ordinary compiler and pinned definitions.

For HTTPcompile→PGexecute, the server needs a bounded trusted `TypedReceiptStore`, not a client-deserialized binding. A successful HTTP compile with an explicit request for an execution receipt stores the sealed compiled artifact, its guard bindings, catalog/definition/scope identities, exact statement and parameter contract, and an authenticated owner identity. It returns an opaque cryptographically random receipt token alongside the ordinary SQL artifact. The token identifies server-owned state; possession alone cannot override owner/scope checks. The store has configurable positive limits (proposed defaults 256 receipts, 32 MiB admitted artifact memory, 5-minute lifetime), admits entries atomically, and rejects requests when full. It must not silently evict an active execution. Expired/unknown receipts fail clearly; no fallback to raw SQL or recreated guard metadata is allowed.

A PG extended-query request references the receipt using a narrowly parsed leading marker, for example `/* semantic_typed_receipt:<opaque-token> */`, followed by the unchanged artifact statement. The marker is dispatch metadata, not executable SQL or a source of provenance. The server finds the stored sealed artifact and verifies the current authenticated owner, current access scope, pinned catalog/definition validity, exact remaining statement bytes and bound parameter values/types against the stored parameter contract before execution. Any mismatch fails before source reads. It then executes the stored compiled artifact with its own private bindings and current effective server budgets; it does not execute arbitrary SQL with a globally registered trusted function. Ordinary SQL without a valid receipt continues through the existing raw SQL path and can produce only unqualified static conditions.

Receipt tokens and raw statements/parameters are not included in default error receipts. Public safe failure metadata includes only the reviewed condition and actual guard/profile/selector identities. Tokens should be treated as execution capabilities and kept out of routine logs/debug Display. Server shutdown destroys the store. Cancellation drops only execution resources; retained receipts remain available until expiry unless an explicit one-shot contract is selected. Concurrent use requires separate scoped execution state, not shared mutable accumulators.

An alternative dedicated authenticated typed execution HTTP route can execute the same server-owned receipt without adding the PG marker, but it cannot establish HTTPcompile→PGexecute parity by itself. A first in-process/CLI implementation may be delivered before this server lifecycle, provided it explicitly reports that public PG lookup receipts are unavailable and does not claim completed public/release parity. The full acceptance goal still requires the approved public path later; this is a staging boundary, not a waiver.

The receipt-store and marker API need lifecycle review against the server's actual authorization model before implementation. Bind an existing authenticated principal when available. A deployment that intentionally uses one server-wide trusted principal can bind that explicit server-owned principal while retaining the artifact's narrower scope; do not invent a client-supplied owner string or broaden the artifact's scope. This proposal does not require adding an unrelated authentication system solely to implement readonly receipts. Tests must cover cross-owner and narrowed-scope reuse, stale catalog/profile/policy revisions, expiry, unknown tokens, altered SQL/parameters, full-store admission, cancellation, concurrent executions and no global function pollution. Planning-time typed failures never acquire an execution stage simply because a receipt exists.
