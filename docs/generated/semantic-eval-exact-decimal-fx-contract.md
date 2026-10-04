# Proposed exact Decimal FX contract from existing commerce facts

This refines the unloaded finance proposal; it is not an accepted extension grammar. Both bundles remain frozen. No source columns, request records, questions, golds or expected-answer hints are added. Parent owns Cargo and live verification.

## Authoritative source and grain

exchange_rates is an actual CSV relation with nonnull Utf8 currency, nonnull Date32 rate_date and nonnull Decimal128(18,6) chf_per_unit. Its declared logical identity is (currency,rate_date); declaration does not enforce runtime uniqueness. There are three canonical rows dated 2024-02-29: CHF 1.000000, EUR 0.950050 and USD 0.900000. They state CHF major units per one foreign major unit on that exact civil date. Target CHF is an authored constant unit fact, not an absent physical target-currency column. The source supplies no interval endpoints and does not justify carrying rates to other dates.

The quarantined exchange_rates_duplicate relation has the same physical columns and two EUR rows for the same date with different rates. The actual lookup must reject multiple matching rows even if their values happen to agree: selecting MIN/MAX/first or deduplicating by value would conceal nonunique matches. Both tables use reader-enforced nonnull columns; generic null handling must still be explicit for other valid source profiles. A null lookup key must not match another null key. A null rate should produce a governed missing-value diagnostic or explicitly authored null outcome, never parity or zero. Current fixtures do not test nullable rates.

## General versioned contract boundaries

A reviewed versioned exact-rate profile should declare the real rate relation, source currency field, matching date field and rate value field; target currency/unit may be an authored constant or an actual field. Date matching is exact Date32 equality, not an interval or nearest-date interpolation. If rates use actual rational numerator/denominator fields, that is a separate supported representation branch; do not fabricate those fields for a Decimal source. Pin relation/field/definition revisions and keep row policies and same-query read boundaries.

The request amount is a tagged typed input: either an exact prepared host literal with explicit monetary unit, or an actual source field whose governed unit and scale establish the amount. Source currency/date may likewise come from explicit typed host bindings or authorized source fields. Literal input is not a new fact relation. Field input must preserve original fact population and cannot fan out under duplicate rates. Distinguish major units from minor units: 100 foreign major units is not 100 CHF cents; conversion must normalize each declared coefficient scale exactly. Unknown/ambiguous unit or currency needs clarification instead of guessing from currently available rows.

A row-wise rate presentation mode uses the actual policy-visible rate rows as its population: multiply each genuine row's rate by the same prepared amount. It is distinct from requesting exactly one rate for one currency/date. Both modes share exact monetary arithmetic; row-wise presentation does not invent synthetic request rows. A requested exact-date conversion requires exactly one matching rate, while a request to list every rate preserves rows and has its own declared identity checks.

## Exact result and rounding semantics

Result currency is CHF and requested presentation is Decimal128(18,2) major units. Decimal coefficients and scales are authoritative. For an amount coefficient A at scale a and rate coefficient R at scale r, the unrounded result is A*R / 10^(a+r), with checked exact wide intermediate arithmetic. Quantize once to the requested result scale. For quotient q and signed remainder, compare twice the absolute remainder with the positive denominator; half-away-from-zero increments the magnitude at ties and otherwise rounds to the nearest representable coefficient. Preserve sign symmetrically, including negative credits. Reject multiplication/final coefficient overflow; no float conversion, tolerance, intermediate truncation or double rounding is allowed for money.

Existing ties are distinguishing: 100 × 0.950050 = 95.005000 CHF and -100 × 0.950050 = -95.005000 CHF, yielding 95.01 and -95.01. Half-even would yield ±95.00 and is not interchangeable. Exact rate coefficient is 950050 at scale 6, not an approximated binary floating point. CHF and USD give exactly ±100.00 and ±90.00. Zero input remains exact zero, although current fixtures do not independently distinguish negative-zero representation; serialized monetary zero should be canonical.

## Existing case obligations and typed gold

calculation.exchange asks every leap-day rate, currency first, then positive and negative converted values, rounding half-away to two decimals, ordered by currency. Typed shape is Utf8, Decimal128(18,2), Decimal128(18,2). Existing gold is:

```json
[["CHF","100.00","-100.00"],["EUR","95.01","-95.01"],["USD","90.00","-90.00"]]
```

Those rows are oracle evidence in this review document only; they must not enter model instructions, request context or profile parameters. Bind requested literal 100/-100 and the explicitly requested date normally. No case-ID branches or authored expected amount constants belong in the API.

reject.duplicate_rate requires rejection of the deliberately conflicting exact-date lookup. unsupported.future_rate explicitly requests unavailable-data handling for a missing 2030-01-01 rate: report unavailable rather than infer parity, extrapolate, or return a fabricated conversion. Its current expected outcome name is unsupported, but the semantic cause must be data absence, not refusal due to missing engine support. clarify.currency omits unambiguous currency/date (“100 dollars in francs”): ask for the missing exact currency/date; do not assume USD or a rate date solely from the fixture. The clarification and empty-lookup outcomes must remain distinct from arithmetic overflow and provider failure.

## Incompatibility with current source-only interval profile

Current CurrencyRateRule requires Int64 source_amount_field, actual source/target currency fields, validity interval fields and Int64 rate_numerator_field/rate_denominator_field. It cannot honestly represent the existing Decimal exact-date table or a standalone literal input without invented physical observations. Existing ConversionRounding permits truncate and half_even, not the requested half-away mode. Adding Decimal representation, exact-date match, constant currency, typed literal/field amount and explicit final scale is a general versioned extension; do not silently change the legacy interval/rational contract or advertise a wrapper that the importer cannot publish.

## Minimum verification batch

Generic exact arithmetic tests must cover signed halfway, below/above halfway, differing input/rate scales, major/minor units, exact zero and checked overflow. Lookup tests must distinguish one match, no match, duplicate identical values, duplicate conflicting values, null keys and policy-hidden matches under one pinned read. Row-wise tests preserve currency order and do not deduplicate rates without an authored identity guard. Prepared literal values must remain host parameters with strict original request evidence and exact types; unrelated lookup failures must not satisfy intended duplicate/missing diagnostics. Compiler publication must validate unit/type/profile compatibility before execution. Existing commerce cases retain their unchanged SQL/gold contract, with parent offline validation and fresh SQL/Ask proof after implementation.

## Review of the concrete API and strengthened negative oracles

The concrete proposal in exact-decimal-rate-api-design.md faithfully bounds a first row-wise literal-major batch; it does not claim one-match or source-field execution yet. RateSourceCurrency derives units from each actual rate row, and the additive Decimal quantizer preserves the legacy rounding profile. Reviewer independently checked current canonical rates, duplicate fixture and all four cases.

The two existing negative companions currently lack execution stage and typed reason requirements; compile-stage capability refusals can therefore satisfy them without reading data. A reviewed metadata migration should strengthen only those obligations to execution-stage data_unavailable/missing_exact_rate and data_rejected/non_unique_rate, requiring successful compiled-query evidence and pinned guard/profile provenance. Prefer those explicit codes consistently across runtime adapters and the harness; the concrete draft's rate_missing/rate_nonunique labels need reconciliation before public API approval. Generic execution_error or diagnostic strings are insufficient. Preserve previous negative oracle files and reports as provenance. No artifact migration is authorized yet.

A missing-rate pass must prove an actual exact-date lookup found zero policy-visible matches; a duplicate-rate pass must prove more than one matching row, including equal-valued duplicates. Unsupported capability, malformed proposal, provider failure, timeout, overflow and unrelated data conditions must fail those expectations. Null value and nonpositive rate require separately typed policies/codes and must not masquerade as missing exact-date data. Checked wide coefficient products, scaling, doubled remainders and final precision boundaries need overflow-safe implementation and tests.
