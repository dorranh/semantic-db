# Catalog and execution contract drafts after full SQL verification

These are review proposals, not loaded artifact extensions. Both live artifact bundles stay frozen. Full SQL evidence is commerce 103/103 and BIRD 66/66; remaining Ask failures must be fixed without weakening golds. The old commerce and BIRD Ask reports predate several catalog/compiler repairs, so their refusals identify investigation targets rather than proving that the latest implementation still fails. The current full run independently confirms the unsold-product reverse-role gap below.

## Priority 1: complete authentic relationship directions and related concept scopes

Commerce currently authors items→products (`item_product`) but lacks products→items. A proposed `product_items` relationship uses exact product_id equality, possible multiple item rows, and missing items; it must not assert unique right keys. The unsold-products request starts with the catalog product population and excludes products for which an item has positive quantity and its related order satisfies the pinned completed concept. Product 40 is the existing distinguishing unsold record.

Adding the reverse role alone is insufficient. A related/existence scope must be able to invoke an authored concept on a further related occurrence, retaining definition references and typed arguments. The generic operation should reuse a bound concept filter inside the related scope instead of asking the model to reconstruct its predicate. The membership date argument must remain an exact prepared Date32 binding. Nested existence should preserve the start population and avoid fanout.

The other genuine equality directions to audit are refunds→orders (`refund_order`) and orders→refunds (`order_refunds`), alongside the existing composite refund→item and item→refund roles. A refund's order_id is a real declared business key, not an invented shortcut; qualification by completed order can then preserve independent refund aggregation. Current buyer/order, recipient/order, subscriber/customer and item/order directions already have both orientations. Products→items is still missing. Date-calendar reverse equality is meaningful only for the Date32 rule; UTC business-date mapping needs the authored timezone transformation, not raw timestamp/date equality. BIRD needs the same direction audit for each original FK, with honest many/missing semantics and explicit occurrence roles.

## Priority 2: exact signed ratio finalization

Commerce weighted unit price means `SUM(line_minor) / SUM(quantity)`, including negative credit quantities. It is not `SUM(value * nonnegative_weight) / SUM(nonnegative_weight)`: the existing weighted-average state rejects negative weights and would double-weight line amounts. Keep the signed numerator and denominator sums, their independent populations, and zero-denominator null behavior.

The existing ratio execution returns Decimal128(38,18) by truncating toward zero. A proposed generic finalization contract must specify output precision/scale, rounding, and zero behavior explicitly, for example:

```json
{"result_type":{"Decimal128":[18,6]},"rounding":"half_away_from_zero","zero":"null"}
```

That object is a proposal, not a currently accepted extension. Compute the requested decimal coefficient using checked exact numerator/denominator arithmetic and its remainder. Preserve sign, reject overflow, and round once at the requested scale without binary floats. Do not silently claim that the existing scale 18 truncation satisfies every requested rounding mode/scale. The source quantity remains Int32; an exact aggregate promotion to Int64 must be explicit and checked, rather than changing the width fixture or inventing a new fact.

A percentage is an explicitly scaled dimensionless ratio with factor 100, not an unsupported arbitrary expression. A general typed ratio scale/finalizer can cover BIRD percentages without a special dataset branch. Zero policies belong to authored/requested contracts; explicit arithmetic-error requests require an error policy distinct from nullable reporting ratios.

## Priority 3: exact decimal rates, exact-date lookup, and signed monetary rounding

The actual commerce rate relation has currency, rate_date(Date32) and chf_per_unit(Decimal128(18,6)). It states an exact rate to CHF for that particular business date. Its data does not establish a validity interval extending to other dates. The existing CurrencyRateRule requires separate Int64 rational fields, source/target currency fields and nonnull interval endpoints; the existing scalar conversion rounding enum contains truncate/half_even, not half-away-from-zero. Neither profile honestly represents all current asks.

A proposed general rate-lookup profile should support:

- an exact-date match on rate_date, with same-query uniqueness and coverage checks;
- source currency from the actual currency field and constant target currency CHF;
- exact decimal-rate representation using its coefficient and physical scale, or authored rational fields when they genuinely exist;
- typed monetary input from a source field or a host literal, avoiding a fabricated conversion-request fact table;
- output currency/minor-unit scale and half-away-from-zero rounding;
- distinct missing/ambiguous/duplicate outcomes and their semantic diagnostics.

The positive and negative halfway counterexamples are real: 100 EUR at 0.950050 gives ±95.005 CHF, which must become ±95.01 at two decimals. Half-even would yield ±95.00, so substituting that mode is wrong. The rate coefficient 950050/1000000 can be derived exactly from the actual decimal type/value; no binary-float tolerance applies to money. For a lookup request, missing currency/date needs clarification, a deliberately duplicate exact-date lookup must reject nonunique matches, and a required absent exact-date rate must report unavailable data rather than inventing or extrapolating a rate. Unqualified “dollars” must not be assumed to mean USD solely because the current rate table happens to contain USD.

For row-wise “each rate” presentation, the amount literal is multiplied by each genuine rate record and rounded under the same generic exact monetary expression contract. It does not require injecting one synthetic input row per case.

## Priority 4: allocation from genuinely recomputed domain sources

The actual requested allocation is net order amount after all linked refunds, distributed proportionally to original signed line cents. The current order 101 example has exact integer shares; it does not exercise a residual-cent tie. The existing AllocationContract requires a complete source-declared bridge count/weight total, Int64 keys, a real net source amount, and nonnegative weights. Those fields/types are absent from the base tables, and signed line weights need a clearly reviewed sign policy.

An authorized future domain source may be a real bootstrap-derived table populated from every preserved base fact. At order grain, derive net_minor, expected_line_count and expected_line_weight_total by independently aggregating refunds/items before joining to orders. A bridge table may preserve line identity with an exact Int64 representation of line_no. Declare physical NOT NULL constraints and verify canonical rows/counts: ordinary PostgreSQL views expose nullable metadata and cannot merely pretend their keys are physically nonnull. No case-specific IDs, predicates, answer aggregates or harness-injected observations may create these fields.

Keep version 1 nonnegative allocation semantics intact. A signed-proportional version needs exact signed denominator arithmetic, zero-denominator rejection, explicit mixed-sign behavior, conservation, and a typed target ordering rule for residual ties. Lexicographic string comparison of line numbers would put 10 before 2 and is not the same as a numeric lower-line-number policy. Do not silently replace signed weights by absolute weights: mixed credit/sale lines would change the requested proportions. Any new rounding/residual behavior must be reviewed against a distinguishing independent arithmetic fixture before being advertised as covered.

## Priority 5: governed historical paths and composite lookup execution

The existing catalog RelationshipPath/AsOfJoin structs already express half-open history with same-query uniqueness. A future Ossie wrapper can carry those exact structs; this sketch uses their current fields and is not yet an accepted importer kind:

```json
{
  "version":1,"start_relation":"orders","start_occurrence":"order","usage":"lookup_unique",
  "steps":[{
    "from_occurrence":"order","to_occurrence":"history","right_relation":"customer_region_history",
    "relationship":"billing_region_history","role":"historical_billing_region",
    "as_of":{"fact_time":"order_date","valid_from":"valid_from","valid_to":"valid_to",
      "timezone":null,"missing":"null","same_query_uniqueness":true}
  }]
}
```

The equality key is billed customer ID; the path adds valid_from<=order_date<valid_to with null upper bound open-ended. Declare the edge as a historical candidate relation, not an ordinary unique FK implying as-of behavior. Missing history preserves the order with null historical region; current customer region never substitutes. Deliberate overlap must fail the same-query interval guard. Composite refund/item keys must retain both order_id and line_no through a multi-hop lookup; relaxing the current one-key-per-hop execution restriction must preserve vector key equality and uniqueness obligations.

## Priority 6: calendar population and benchmark representations

The current Swiss business-calendar contracts truthfully cover Date32 order_date and UTC placed_at separately, with mapping-data revision and Zurich timezone. They do not yet declare month_key mapping or a distinct authored month population/spine. A future calendar dimension contract should identify the actual month_key field and unique month population for zero-filled reporting. Use the distinct authored months, including the existing no-sales month, rather than inventing all months between minimum and maximum dates. Missing mapping/duplicate-date guards must run under the fact read boundary.

BIRD Ask refusals also identify general representation needs: typed duration parsing with seconds/minutes/hours components, guarded numeric Utf8 conversion for fastestLapSpeed, Gregorian date-part/year-difference expressions, projection after grouping/ranking, and counting filtered groups. All 400524 original lapTimes strings were independently Decimal-parsed and match milliseconds exactly, supporting an authored equivalent-representation contract without replacing raw source strings. Qualifying/fastest-lap durations, whole-race elapsed times and trailing-driver offsets are different representations and must not be conflated. Do not substitute birthday-adjusted age where the provided benchmark evidence specifies year difference, or conventional SUM(wins) where evidence explicitly specifies COUNT(wins).

Author legitimate table-grain row/event counts and measured aggregates where business meaning is clear, or expose the compiler's real generic aggregate capability by physical types. Do not manufacture governed metrics solely to imitate a reference result. Benchmark-provided per-task evidence should eventually be a bounded, provenance-pinned request-context input separate from the unchanged question and expected answer, rather than broadcasting every task's evidence into every request. No reference SQL/gold may enter that context. These proposals require API review and fresh full Ask evidence; none is an acceptance exemption.

## Unloaded reverse-role and nested-scope proposal

The next commerce author patch should add `product_items`, from `products.product_id` to `order_items.product_id`, using the same equality relationship format as existing reverse roles. Its right side permits multiple rows and zero matches; no uniqueness or cardinality enforcement is asserted. This proposal remains outside the frozen artifact.

The current typed `Related` operation accepts one relationship, role, instance, existence mode, and optional `RowPredicate`. The predicate grammar lacks another relationship scope or a concept invocation. A general extension needs bounded recursive existence predicates and bound concept references, resolved against each related occurrence. For unsold products, the semantic structure is NOT EXISTS item WHERE quantity > 0 AND EXISTS order WHERE completed(order). Keep the catalog products as the starting population, correlate both declared equality edges, and expand the pinned completed concept inside the order occurrence. Positive quantity is a request condition, not a new invented governed concept.

The compiler must validate occurrence scope, authored roles, concept arguments and physical types before planning. Bind concept parameters once through the normal host parameter path. Correlated EXISTS must avoid duplicating starting rows even when multiple items or orders qualify. Tests should distinguish an unsold product, a product with pending-only positive sales, a product with only completed credit/zero-quantity lines, and a product with multiple qualifying completed lines. Current fixtures cover some of these populations; additional coverage must be honest rather than claimed from the reverse edge alone.
