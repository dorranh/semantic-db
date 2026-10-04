# Exact decimal rate API proposal

Status: proposed for review, not implemented. Existing integer/rational interval
conversion remains unchanged. Artifact SQL and result golds are not part of model
input. Implement row-wise arithmetic first; exact one-match lookup and companion
outcome migration follow as a separate batch.

## Authored contract

Add `ExactDecimalRateRule` to the catalog, under a new
`RelationSemantics.exact_decimal_rates` map, indexed and published like existing
conversion definitions. Version 1 uses actual decimal rate rows:

```json
{
  "version": 1,
  "id": "rates/domain-to-target",
  "rate_relation": "actual_rates",
  "source_currency_field": "currency",
  "date_field": "rate_date",
  "rate_field": "major_target_per_major_source",
  "target_currency": "CHF",
  "positive_only": true,
  "null_rate": "unavailable",
  "source_refs": []
}
```

Currency must be Utf8, date Date32, rate Decimal128 with validated precision and
scale. Rate means target major units per source major unit. Target currency is an
authored constant, never a fabricated target field. Version 1 declares positive
rates only and rejects null rate values as a typed missing-value condition; these
policies are explicit authored fields (`positive_only: true`,
`null_rate: "unavailable"`) and no other policy is accepted yet. Publishing validates physical
fields, supported exact decimal scales, valid currency identity and provenance.
A future rational representation or interval variant must be separately tagged.
Ossie can publish this exact struct with a new `exact_decimal_rate` wrapper. No
actual rate value, case constant or result is authored into this contract.

## Row-wise request operation

Add a closed `RateAmount` input:

```json
{
  "kind": "literal",
  "value": {"type": "decimal128", "value": {"coefficient": "100", "precision": 3, "scale": 0}},
  "unit": {"kind": "rate_source_currency"},
  "basis": "major"
}
```

The decimal literal uses the existing exact Decimal128 wire shape and illustrates
mathematical 100; there is no second serializer. The input can alternatively be `kind: field` with a scoped
FieldRef; that field must have a matching authored monetary unit and exact scale.
The first implementation accepts only literal amounts. Field input is reserved
until a genuine source-fact population and lookup lowering are implemented.
`rate_source_currency` refers to this pinned profile's currency field on each real
rate row; it does not falsely declare all input amounts to be USD or CHF. `major`
and `minor` are explicit amount bases. Minor requires a reviewed authored currency
minor scale; first implementation supports major only and rejects other modes.

A row operation `convert_rate` selects a named published profile and applies the
literal to the current rate row:

```json
{
  "kind": "convert_rate",
  "rate": "domain_to_target",
  "amount": {"kind": "literal", "value": {"type": "decimal128", "value": {"coefficient": "100", "precision": 3, "scale": 0}}, "unit": {"kind": "rate_source_currency"}, "basis": "major"},
  "result_type": {"Decimal128": [18, 2]},
  "rounding": "half_away_from_zero",
  "alias": "converted"
}
```

This operation requires the row query's input relation to equal the profile rate
relation. It preserves that actual policy-visible row population. Date selection
is an ordinary exact Date32 filter, with typed literals/host parameters. It does
not create a synthetic input relation, infer a missing date, deduplicate, choose a
first rate or interpolate. Projection/order follow normal row-query rules. The
binder validates the profile, source/result exact types, requested unit and all
parameters; compiled artifacts pin the profile/fields/function/read scope.
Output SlotMeaning retains Currency(CHF), with major-unit basis and the exact
result scale carried by the Arrow Decimal128 type. It cannot be summed as a
different monetary basis merely because both units have the same currency code.

## Exact arithmetic boundary

A new closed local scalar contract multiplies two Decimal128 coefficients and
quantizes once to the explicitly bound output precision/scale. Its checked wide
intermediate uses i256. For input scales a,r and result scale s, multiply A*R and
then multiply/divide by the exact power of ten corresponding to s-a-r. Precision,
scale and rounding are compiler-bound constants, not unrestricted SQL or model
functions. The UDF validates them again for callers bypassing the binder.

Rounding supports truncate, half_even and half_away_from_zero in a new decimal
rounding enum; do not extend/reinterpret legacy conversion rounding behavior.
Compare absolute remainder with half a positive denominator without overflow,
round symmetrically once, normalize zero, enforce final declared precision, and
reject intermediate or final overflow. No binary floating point is permitted.
Null amount propagates only under an explicit input contract. A missing/null rate
is a semantic data condition, not parity/zero. Nonpositive rates are rejected
unless a later version explicitly defines a different financial domain.

Public engine profile adds only the closed checked function/signatures and remains
LocalOnly. Compiler pipeline/cache revisions and runtime profile revisions change
with accepted execution semantics. Tests cover signed ties, below/above ties,
unequal scales, zero, null input, invalid/null rate, overflow, scalar/array
broadcasting and independent exact arithmetic. Compiler tests verify direct/SQL
parity, units, scope, row policies, exact prepared literal parameters, result
schema and imported authoring roundtrip.

## Exact one-match lookup: separate follow-up

An additive request selector distinguishes `each_row` from
`exact { currency, date }`, with explicit typed host values or scoped source
fields. Exact selection requires one policy-visible match under the same pinned
query read as the amount. Use same-read count/coverage guards; null keys never
match. Duplicate identical rates still fail. Required zero matches are unavailable;
no MIN/MAX/first selection or nearest-date behavior is allowed. Source-field
conversion preserves fact population and rejects fanout. These capabilities are
not claimed by the first row-wise batch.

## Public runtime outcome and oracle migration

Introduce a public typed `SemanticDataCondition` error in semantic-runtime:

- `Unavailable { code: MissingExactRate | RateValueMissing }`
- `Rejected { code: NonUniqueRate | InvalidRate }`

Use typed external errors/downcasting through engine/DataFusion wrappers, not
substring matching. Messages and codes are static and omit key values, credentials,
SQL and row content. Arithmetic overflow remains its real execution error;
transport/provider failures remain distinct. Compilers remain data independent.
An HTTP compilation request can succeed even if later execution finds missing or
nonunique rates. CLI machine execution and query HTTP responses expose the typed
condition/code with nonzero/error status. PG execution preserves sanitized messages
and uses suitable SQLSTATE (missing data P0002, nonunique cardinality 21000); it
must not reveal internal row/source content.

Harness expected outcomes add exact-code `data_unavailable` and `data_rejected`,
with mandatory stage `execution` and successful typed compilation evidence.
Each carries the stable typed condition code and pinned profile provenance.
Compile success/status and profile provenance must remain in default no-debug
reports, independently of optional raw SQL/transcript capture. Canonical codes:
`missing_exact_rate`, `non_unique_rate`, `rate_value_missing`, `invalid_rate`.
Update only the affected negative companion contract after reviewer/analyst verify
its independent semantic obligation: unavailable future exact-date rate becomes
`data_unavailable/missing_exact_rate`; duplicate exact-date rate becomes
`data_rejected/non_unique_rate`. A compile-time Unsupported/Rejected will fail those
expectations. A generic execution error, provider failure, timeout, unsupported
capability or different data code will also fail. Retain prior reports and record
that this strengthens execution-phase and reason requirements; no result gold or
comparison tolerance changes. Row-wise-only work leaves those companions failing
until their actual execution guards/outcome adapters are implemented and verified.

No artifact migration is authorized by this design document alone. Parent and
reviewer approve the public outcome/API shape before implementation, then the
analyst owns standalone artifact changes and independent oracle checks.


## Rowwise integration handoff

The proposed `convert_rate` row operation now uses a closed `RateAmount::Literal` major-unit amount tagged `RateSourceCurrency`, a `DecimalResultType { precision, scale }`, and shared `DecimalRounding` owned by semantic-plan (catalog reexports it). The Ossie wrapper is `{ kind: "exact_decimal_rate", name, dataset, rule }`. The rule uses version 1, exact physical nonnull Utf8 currency and Date32 date fields, a Decimal128 rate field, an authored target currency, positive-only rates, and explicit null-rate unavailability. Source currency identifiers are authored values; this does not assert universal ISO-domain validation or unique rates.

Compiler integration pins the authored profile, physical relation/fields, policies and local function. Native and generated SQL plans use the same private exact quantizer; generated SQL binds the request amount as a typed decimal parameter. Output meaning is target Currency in major units at the requested physical scale. Context rendering and its audit retain the complete profile and source provenance. Pipeline 39 and lowering pass 9 invalidate incompatible retained plans; execution profile 13 was introduced by the separately verified arithmetic slice.

This slice does not add field/minor amounts, exact lookup, prepared amount placeholders, missing/nonunique match guards, or typed runtime outcome adapters. Literal amounts are retained as ordinary SQL artifact parameters. The new compiler/importer tests require independent signed tie results, policy filtering, scope/type/result/alias rejection, closed malformed wire contracts, graph slot unit retention and imported source provenance. The parent’s focused Cargo verification passed 11 tests across four targets (catalog 2, compiler 6, engine 1, Ossie importer 2). Full workspace all-feature verification passed: 658 tests, zero failures, 29 ignored, across 188 target results (149 nonempty). Workspace all-target/all-feature Clippy with warnings denied also passed. The context manifest suite passed 13 tests, including repaired complete/retrieved fixtures and exact-rate group omission/tampering regressions. The all-feature workspace binary build for semantic-eval and sdb passed. The parent retained immutable pipeline-39/profile-14/context-renderer-3 binaries for subsequent runs. No live acceptance result is claimed for this slice yet.


The initial integration run exposed a real output metadata mismatch: a known nonnull decimal literal planned a nonnullable output while the same typed amount as an unbound SQL parameter planned nullable. A focused real Engine metadata probe confirmed equal decimal types and different nullability. The UDF now conservatively declares nullable output for both forms, and the private binder/verifier require that same contract; exact schema checks remain unchanged. Execution profile 14 and function definition revision 2 reflect this observable metadata change. Nullable metadata does not introduce default values or suppress rate errors. Literal/unbound-parameter and empty-population regressions cover the portable schema.
