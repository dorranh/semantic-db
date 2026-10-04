# Commerce acceptance failure ledger

This is investigation evidence, not a release pass. All failing expectations remain acceptance work. No cases are exempted and no gold results are weakened. This reviewer performed static report/source review; Cargo and runtime tests are owned by the orchestrator.

## Source identity and selection

- `.semantic-eval/commerce-luna/observations.json` — SHA-256 `3c99d1dd9529453c6f1aeca6db1fdacc3569006f51982b5462a58d264961462d`
- `.semantic-eval/commerce-luna/run-140456-18db4d78c2641149/report.json` — SHA-256 `eee5867b23239d908110f207e7be7f0ae60428ede19188b42dbf2bd3af30bd72`
- `.semantic-eval/commerce-luna-paced/run-162605-18db5450b1357cb4/report.json` — SHA-256 `d461a9865279c078d86aa914c9eb63d9dcc1aee7e766e264679eadc5defa83df`
- `.semantic-eval/commerce-luna-public-followup/run-175403-18db54ebf54ea47e/report.json` — SHA-256 `eaefdcbeb9c5b1432e6937908ae296fa3f42a8b2e57b1d29da87534252065d51`

Artifact digest recorded by observations: `b7f30c2e568dd1640a33f55bb0c0734fb709badf0898846de6fa5e01d01e367a`. Model: `gpt-6-luna`.

The merged Ask ledger uses the original 115-case run, replacing only its 39 HTTP 429 provider failures with the paced followup for the same case IDs. All 39 replacements satisfy that selection rule. The resulting 115 distinct Ask cases contain 34 passes and 81 failures, with no remaining provider errors. This merge is diagnostic evidence; it is not a clean full release run.

## Priorities and shared capabilities

1. **P0: preserve intent/provenance and exact result shape.** Repair request-span validation and model repair behavior; retain binding guarantees. Check projection, population retention and explicit output ordering for the three executed mismatches. Public HTTP `roles.billing` reproduces span rejection.
2. **P1: relationship and governed composition.** Support reverse existence/absence, population-preserving traversal, composite-key multi-hop paths, concept predicates in related scopes, field comparisons at authored integer widths, compatible metric lookup dimensions, and independent fact aggregation.
3. **P1: exact numeric and temporal behavior.** Preserve monetary rounding and null policies; implement checked scalar overflow, conversion and allocation; add faithful executable calendar/history/active-subscription declarations where currently prose-only. Human dates should not require manual epoch conversion.
4. **P2: windows, sets, Boolean concepts and ordering.** Add the actual requested operators and bindings; resolve genuine question/catalog ambiguities without replacing expected answers.
5. **P1: outcome fidelity.** Distinguish clarification, scope rejection, invalid governed lookup rejection, unsupported capability, and resource exhaustion. An unsupported outcome does not satisfy an expected rejection or clarification.

## Complete Ask failure coverage

Diagnostics below are exact strings from the selected report; result mismatches show the recorded comparator differences. Grouping follows authored primary groups. Reported explanations identify hypotheses and do not establish which component is responsible.

### aggregations (9)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `metrics.average` | `unsupported` | The query requires an exact arithmetic average rounded to two decimal places, but the available aggregate average returns Float64 and the ratio operation has fixed Decimal128(38,18) truncation. No supported operation provides the requested two-decimal rounding. |
| `metrics.customer_count_zero` | `result` | ["row 1: expected [String(\"2\"), String(\"0\")], actual [String(\"3\"), String(\"2\")]", "row 2: expected [String(\"3\"), String(\"2\")], actual [String(\"5\"), String(\"1\")]", "row 3: expected [String(\"4\"), String(\"1\")], actual [String(\"6\"), String(\"0\")]", "row 4: expected [String(\"5\"), String(\"1\")], actual [String(\"4\"), String(\"1\")]", "row 5: expected [String(\"6\"), String(\"0\")], actual [String(\"2\"), String(\"0\")]"] |
| `metrics.customer_totals` | `unsupported` | The governed gross_sales_minor metric does not authorize grouping by the billed customer name through the billing_customer relationship; its compatible lookup dimensions are empty. I can’t replace the named metric with an explicit sum or use an ordinary join. |
| `metrics.having` | `unsupported` | The authored gross_sales_minor metric has no compatible billing-customer lookup dimension, so it cannot be grouped with billed buyer names under the catalog's governed-metric rules. |
| `metrics.minmax` | `rejected` | request_span: Requirement source text must exactly equal its source spans joined by a space |
| `metrics.product_units` | `unsupported` | The governed signed_quantity metric does not authorize grouping by the product-name lookup dimension. |
| `metrics.refund_sum` | `needs_clarification` | How should the two global results be aligned: union, intersection, left (total refund cents), or right (count of positive refunds)? |
| `metrics.status` | `needs_clarification` | Should statuses be ordered ascending or descending? |
| `metrics.weight_sum` | `unsupported` | Calculating total shipped weight requires multiplying each matched product's weight_kg by its item quantity and summing the results; computed expressions and ordinary joins are unsupported. |

### companions (8)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `clarify.currency` | `unsupported` | The catalog has no authored executable currency-conversion rule or rate lookup for this request. The dollar currency and conversion date are also unspecified. |
| `error.division_zero` | `unsupported` | The ratio operation supports only null or zero behavior for a zero denominator; it cannot require an arithmetic error. |
| `error.invalid_cast` | `unsupported` | Casting a customer name from Utf8 to a signed bigint is an unsupported computed conversion. |
| `error.overflow` | `unsupported` | Multiplication is an unsupported computed expression, and the result also exceeds the signed 64-bit bigint range. |
| `reject.denied_scope` | `unresolved` | expansion_limit: Context expansion found no additional authoritative context |
| `reject.duplicate_rate` | `unsupported` | The requested exchange_rates_duplicate relation is a quarantined diagnostic lookup with deliberately conflicting matches. The catalog requires rejecting ambiguous rate lookups, so it cannot be used for this conversion. |
| `reject.invalid_rollup` | `unsupported` | The catalog defines weighted unit price as the ratio of signed sums and explicitly prohibits averaging subgroup averages. Averaging product-level weighted prices without weights is unsupported. |
| `reject.overlap_history` | `unsupported` | The requested as-of lookup uses the quarantined customer_region_history_overlap relation, which is explicitly marked with conflicting matches and rejects governed lookups on nonunique matches. No authored relationship or temporal lookup path is available to connect Ada to that relation. |

### concepts (5)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `concept.active` | `unsupported` | The query requires filtering on April 1, 2024, but the required Date32 literal is an epoch-day count and the request does not provide that count; deriving it is disallowed. Also, active membership is described only in prose, not as an executable authored concept, so its predicate cannot be reconstructed for this query. |
| `concept.buyers` | `needs_clarification` | Should I use the `buyer_directory` view (which exposes only ID and name), or the `buyers` view (which also exposes region so I can filter for `CH`)? And should `CH` mean the exact region value `CH`? |
| `concept.gross_by_region` | `needs_clarification` | Should sales for orders whose billed client has no matching customer record be included with a null region, or excluded? |
| `concept.purchases` | `result` | ["column count: expected 2, actual 3"] |
| `concept.zero_active` | `unsupported` | The requested date must be compared as a Date32 literal, but the request does not supply its epoch-day count. Converting the calendar date to that count would require an epoch conversion not permitted by this protocol. |

### existence (7)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `absence.buyers` | `unsupported` | The catalog defines the billing_customer relationship from orders to customers, but not an authored relationship from customers to orders for the requested absence check. Reversing it would require an unsupported ordinary join. |
| `absence.completed_buyers` | `unsupported` | The request requires finding customers with no matching completed orders, but the catalog authors the billing_customer relationship only from orders to customers. There is no authored customer-to-orders relationship for an absence requirement, and an ordinary join is unsupported. |
| `absence.no_subscriptions` | `unsupported` | The catalog does not author a relationship from customers to subscriptions for a related absence check. The available subscriber relationship is directed from subscriptions to customers; reversing it would require an unsupported join. |
| `absence.unsold_products` | `unsupported` | Finding products without qualifying sale lines requires traversing from products to order_items and then to completed orders. The catalog does not provide an executable relationship path from products for that absence query; ordinary joins are unsupported. |
| `existence.only_ch_shipping` | `needs_clarification` | Should I exclude a billed customer if any of their orders was shipped to a recipient whose region is known and not exactly “CH”? Is “CH” the intended code in the recipient’s region field? |
| `existence.refunded_clients` | `unsupported` | Determining whether each refund belongs to a completed order requires linking refunds to orders, but the catalog has no authored relationship for that link. Ordinary joins are unsupported. |
| `existence.sold_products` | `unsupported` | The authored completed concept is defined on orders, but the item-to-order existence condition cannot apply concept_filter to the related order. Reconstructing the concept as a status predicate is not permitted. |

### historical (6)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `history.all` | `unsupported` | Historical region lookup by customer and order date requires a temporal relationship that is not authored in the supplied catalog. |
| `history.boundary` | `unresolved` | expansion_limit: Context expansion found no additional authoritative context |
| `history.changed` | `unsupported` | The catalog does not define an executable temporal relationship or lookup from orders’ billing customers to customer_region_history. Comparing current and historical payer regions requires that lookup at each order’s date, so the requested result cannot be expressed with the authored relationships. |
| `history.current_vs_then` | `unsupported` | The catalog does not define an authored temporal relationship or lookup from the payer to customer_region_history at the order date. The requested historical region cannot be expressed with the supported operations. |
| `history.missing` | `unsupported` | The catalog has no authored temporal relationship or lookup from orders to customer_region_history. Returning the historical payer region while retaining orders with unknown history requires an unsupported temporal join. |
| `history.region_sales` | `unsupported` | The catalog has no authored temporal relationship or lookup from orders to customer_region_history. Retrieving each payer’s historical region as of the order date requires that unsupported lookup; the catalog’s history prose is not an executable relationship. |

### multiple_facts (10)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `fanout.line_count` | `unsupported` | The catalog has no authored relationship connecting refunds to orders for counting refund events per order. The available refunds-to-items relationship requires a multi-hop join, which this request cannot execute as an ordinary row query. |
| `fanout.line_refunds` | `needs_clarification` | Should the results include every line in order 101, with 0 cents for lines with no refunds, or only lines that have refunds (and should a missing refund total be shown as null instead)? |
| `fanout.net` | `unsupported` | This requires combining completed order totals with linked refunds and then summing the net amounts. The catalog has no executable governed net-sales metric, and the available multi-fact composition does not support the required post-composition rollup. |
| `fanout.order_item` | `unsupported` | The completed-orders concept is authored on orders, but the item-to-order existence requirement cannot apply that concept to its related order scope. Producing both totals while preserving the completed-order constraint would require an unsupported join or an unfaithful reconstruction of the concept. |
| `fanout.order_refund_count` | `unsupported` | Attributing refund events to buyers requires traversing the authored refund-to-order-item and order-item-to-order relationships. The available query operations do not support that ordinary multi-hop join or an equivalent authored relationship. |
| `fanout.orders_subscriptions` | `unsupported` | This request requires returning the full customer population—including customers with no matching facts—alongside measures from two independently aggregated fact relations. The available composition operations do not support the required customer-domain scaffold. The year for “April 1” is also unspecified. |
| `fanout.per_buyer_net` | `unsupported` | The request requires attributing refunds to billed buyers through refunds → order_items → orders, but the catalog has no authored refunds-to-orders relationship or supported multi-hop aggregate composition for that path. |
| `fanout.product_net` | `unsupported` | The request requires combining completed-item sales and line refunds as independently aggregated facts by product, then returning the product name. The catalog does not provide a supported authored relationship or lookup path to compose those product-level aggregates and include the name without an ordinary join or an unauthorized governed-metric dimension. |
| `fanout.refund_fraction` | `unsupported` | The requested result requires combining refunds with completed orders and rolling the matched amounts up by billed buyer ID; the catalog does not provide an executable path for that multi-fact aggregation. Also, explicit ratios return Decimal128(38,18), and no supported operation converts that result to float. |
| `fanout.weighted_price` | `unsupported` | The requested half-away-from-zero rounding to six decimal places is not supported by the available ratio operation, which truncates toward zero at 18 decimal places. The authored completed-order concept also cannot be applied through the related-existence predicate supported for order items. |

### projection (4)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `projection.boolean_scope` | `unsupported` | The authored VIP concept can only be applied as a row-stage concept_filter, which combines conjunctively; the request requires VIP as one branch of an OR condition. |
| `projection.float` | `needs_clarification` | The catalog does not author a unit for products.weight_kg. Should I use that field for the requested kilograms value, or is a different source or definition intended? |
| `projection.negation` | `needs_clarification` | Should “outside CH” mean region is not exactly “CH”, and should IDs be sorted in ascending order? |
| `projection.unicode` | `needs_clarification` | By “French-region,” do you mean customers whose region value is exactly “France,” or customers in French-speaking regions of Switzerland? If the latter, which regions should be included? |

### relationships (11)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `relationships.all_customers` | `unsupported` | Returning each customer with their billed order IDs while retaining customers without orders requires a left join from customers to orders. The catalog only authors the billing-customer relationship from orders to customers, and ordinary joins are unsupported. |
| `relationships.calendar` | `unsupported` | Mapping each order to the authored fiscal calendar requires joining orders.order_date to business_calendar.calendar_date, but no authored relationship supports that lookup and ordinary joins are unsupported. |
| `relationships.left_scope` | `unsupported` | Returning customer IDs with all matching completed billed order IDs requires a one-to-many left join. The catalog query operations do not support ordinary joins. |
| `relationships.product_filter` | `unsupported` | The completed concept is authored on orders, but the available item-to-order relationship only supports an existence predicate; it cannot apply concept_filter to the related order. The request also requires product lookup and category filtering, so the whole request cannot be represented without bypassing the authored concept. |
| `relationships.products` | `rejected` | request_span: Requirement source text must exactly equal its source spans joined by a space |
| `relationships.refund_composite` | `rejected` | path_lookup_keys: The two-hop executable path profile requires one key per hop |
| `relationships.shipping_totals` | `result` | ["column count: expected 2, actual 3"] |
| `relationships.subscribers` | `needs_clarification` | If a subscription has no matching subscriber record, should it still be listed with a null subscriber name, or should it be excluded? |
| `roles.both` | `needs_clarification` | Should the orders be sorted by ID in ascending or descending order? |
| `roles.different` | `unsupported` | The request requires comparing the payer and recipient ID fields to each other. Row predicates support comparisons to literals, while graph slot comparisons require Int64 inputs; both catalog ID fields are Int32. |
| `roles.same` | `unsupported` | The query requires comparing the billing and shipping customer IDs across two relationship paths. The available row-query operations do not support field-to-field comparisons, and the graph comparison operation requires Int64 inputs while these customer IDs are Int32. |

### sets (8)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `calculation.exchange` | `unsupported` | The requested currency conversion and half-away-from-zero rounding require computed arithmetic, but the catalog provides no executable authored conversion for exchange_rates. |
| `calculation.line_allocation` | `unsupported` | The catalog has no executable authored allocation rule for this request. Its broader residual/rounding policy for proportional integer-cent allocations is unavailable, so exact integer line allocations cannot be proposed safely. |
| `calculation.tax` | `needs_clarification` | Should the order IDs be sorted in ascending or descending order? |
| `calculation.zero_ratio` | `unsupported` | The authored `pending` concept can only be applied to the selected relation, `orders`. Computing the weighted unit price requires `order_items`, and the related-existence predicate cannot apply or pin the authored concept there. |
| `sets.except` | `unsupported` | The request requires combining completed orders, buyer/customer identities, and subscriptions. The authored relationships do not support this traversal from a single base relation, and the row-query operations cannot express the required distinct buyer IDs without ordinary joins or an unsupported multi-fact composition. |
| `sets.intersect` | `unresolved` | expansion_limit: Context expansion found no additional authoritative context |
| `sets.union` | `needs_clarification` | Which year’s April 1 do you mean, and should an order’s buyer ID refer to the billed customer (billing) or the recipient (shipping)? |
| `sets.union_all` | `needs_clarification` | For completed orders, should buyer IDs mean the billed customer or the shipping recipient? Which year’s April 1 should be used for the active-subscription date? |

### temporal (6)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `calendar.business_day` | `unsupported` | Matching orders to business_calendar dates requires an ordinary join, and no authored relationship or executable calendar concept supports this lookup. |
| `calendar.fiscal` | `unsupported` | Assigning order sales to fiscal years using business_calendar requires joining orders to the calendar, and no authored relationship or supported operation provides that mapping. |
| `calendar.leap` | `needs_clarification` | Please provide the Date32 day count since 1970-01-01 for February 29, 2024, so I can filter orders on that date. |
| `calendar.missing_month` | `unsupported` | This request requires matching calendar months from business_calendar to completed orders by order date, including zero-filling months without sales. The catalog provides no authored relationship for that match, and the available month-grouping operation does not support the catalog’s Date32 order_date field. |
| `calendar.monthly` | `unsupported` | Grouping completed sales by calendar month requires a month-level mapping from order_date to calendar month_key, but the catalog provides no authored relationship or calendar grouping profile that supports this mapping. |
| `calendar.year_boundary` | `needs_clarification` | Should the orders be sorted by ID ascending or descending? |

### windows (7)

| Acceptance ID | Observed outcome | Exact diagnostic or comparator differences |
|---|---|---|
| `window.lag` | `unsupported` | The requested previous order cents requires a lag/previous-row window operation, which is not supported by the available window functions. |
| `window.per_buyer` | `unsupported` | The requested row number within each payer requires a row_number window function, which is not supported by the available window operations. |
| `window.rank` | `needs_clarification` | Should order IDs be sorted ascending or descending when cents are tied? |
| `window.rank_gaps` | `needs_clarification` | Should the order ID tie-break be ascending or descending? |
| `window.region_running` | `unsupported` | The requested final ordering starts with the related customer region. Row ordering is limited to source fields on the input relation, and this query language does not support ordering by a lookup output. |
| `window.running` | `unsupported` | Row-grain ROWS window frames are unsupported. |
| `window.top_per_buyer` | `needs_clarification` | Should “buyer” mean the billing customer or the shipping recipient? If a buyer has multiple completed orders tied for the largest total, should I return all tied order IDs or choose one? If choosing one, what tie-break rule should I use? |

## SQL failures

| Acceptance ID | Expected | Actual evidence |
|---|---|---|
| `error.overflow` | Execution error containing `overflow` | `{"columns": [{"name": "overflow_value", "type": "int64", "tolerance": null, "nullable": false, "physical_nullable": null}], "rows": [["-2"]]}`; diagnostic `None` |

## Public interface failures

The separately selected public followup passed 7 of 9 checks. These failures are additional interface evidence, not extra Ask acceptance IDs.

- `roles.billing` / `http_typed_pg`: HTTP typed compilation did not produce an artifact
  - `/home/dorranh/dev/semantic-db/.semantic-eval/commerce-luna-public-followup/run-175403-18db54ebf54ea47e/public-roles.billing-http_typed_pg.server.stderr.log`
  - `/home/dorranh/dev/semantic-db/.semantic-eval/commerce-luna-public-followup/run-175403-18db54ebf54ea47e/public-roles.billing-http_typed_pg.compilation.json`
  - `/home/dorranh/dev/semantic-db/.semantic-eval/commerce-luna-public-followup/run-175403-18db54ebf54ea47e/public-roles.billing-http_typed_pg.server.stdout.log`
- `calendar.previous_month` / `cli_typed_ask`: public CLI failed with exit status: 1 (see retained logs)
  - `/home/dorranh/dev/semantic-db/.semantic-eval/commerce-luna-public-followup/run-175403-18db54ebf54ea47e/public-calendar.previous_month-cli_typed_ask.stdout.log`
  - `/home/dorranh/dev/semantic-db/.semantic-eval/commerce-luna-public-followup/run-175403-18db54ebf54ea47e/public-calendar.previous_month-cli_typed_ask.stderr.log`

HTTP `roles.billing` retained compilation reports `request_span` rejection and `request_spans_validated=false`; no executable artifact was produced. CLI `calendar.previous_month` stderr reports typed Ask did not produce an executable result; the high-level failure alone does not prove a clock or date arithmetic defect.

## Question/catalog precision defects

These findings require author corrections preserving intended golds and feature coverage. They are not accepted excuses for failure. The reviewer did not edit the bundle.

- `window.top_per_buyer`: question says “largest completed order ID”, while SQL ranks `total_minor DESC, order_id ASC`. State highest completed order amount and its tie policy explicitly.
- `sets.union`, `sets.union_all`: question says April 1 without a year, while SQL fixes 2024-04-01. Explicitly state 2024 and billed buyer identity; the catalog billing default applies to sales attribution metrics, not all identity sets.
- `projection.unicode`: “French-region” is not an explicit equality to `region=FR`. State the intended region code or author its exact meaning.
- `fanout.line_refunds`: SQL retains all order lines and zero-fills missing refund totals; the question should state both policies.
- `concept.gross_by_region`: SQL excludes orders with unknown billed customer using an inner join; state that population policy explicitly.
- `relationships.subscribers`: SQL excludes subscriptions with unknown subscribers; state that retention policy explicitly.

Additional representation gaps: history, active membership, conversion and allocation rules are expressed in authored AI instructions, but reports cite absent executable catalog profiles; governed metrics expose source dimensions yet no compatible name lookup dimensions. These need faithful structured declarations and interpreter/compiler support, not silently weakened business rules. `projection.float` asks for kilograms but the weight field lacks a structured unit declaration. This is a catalog precision gap to repair rather than infer solely from its physical name.

Earlier final artifact review did not surface all wording defects above. This ledger corrects that omission; prior independent arithmetic checks remain evidence for the intended golds, not proof of unambiguous natural language.

## General overflow integration recommendation

`error.overflow` is `SELECT CAST(9223372036854775807 AS BIGINT) * CAST(2 AS BIGINT)`. Its checked-overflow requirement is explicit and internally consistent. Wrapped `-2` is a genuine engine behavior failure.

DataFusion 55 physical `BinaryExpr::new` defaults `fail_on_overflow=false`. Its arithmetic implementation selects Arrow checked add/subtract/multiply only when that flag is true; otherwise multiplication wraps. The physical planner creates native binary expressions without enabling the flag. `execution.enable_ansi_mode` is documented as experimental and relevant only to Spark built-in functions, so setting that configuration is insufficient.

Rewrite integer arithmetic into checked operations before constant simplification, using a SQL expression planner or an early logical analyzer and exact typed checked UDFs. Preserve coercion/output width and null propagation. A final physical-plan rewrite alone misses constants already folded by `ExprSimplifier` through the unchecked physical planner. Ensure the policy covers raw SQL, generated SQL, views, parameter substitution and typed graph lowering. Preserve arithmetic failure classification even if constant evaluation discovers overflow during planning; do not report a wrapped result or coerce to float.

Regression coverage should include constant and column operands; signed extrema, negative multiplication and subtraction; unsigned underflow and maximum multiplication; mixed-width coercion; nulls; nested expressions; bound parameters; partitioned execution; and views. Existing checked sum aggregation is separate from scalar overflow. Do not change monetary decimals or temporal operators by an indiscriminate integer rewrite.

Engine `with_memory_limit` builds the primary query context; `materialization.rs` builds a second execution context with its own session state. Keep the same arithmetic rule/UDF registration in both. Source/deferred and read-projection helper contexts currently project columns only; review them for future expression paths, but no evidence here proves they execute the failing arithmetic.
