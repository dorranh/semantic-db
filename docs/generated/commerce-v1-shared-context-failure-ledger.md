# Commerce full shared-context run: terminal evidence and complete failure ledger

Frozen artifact `fbcc754e5ebe6e3cb854c34abf65fd0c1a49e18819027c64065346225f6e530b` passed SQL 103/103 and Ask 51/115, totaling 154/218 passing attempts. All 218 planned attempts finished. Finalized and full coverage are true; complete is false because five Ask attempts remain unresolved with expansion_limit: calendar.monthly, fanout.net, history.missing, history.all and history.boundary. No provider errors occurred. Setup, cleanup and artifact errors are null. The 64 failed Ask attempts remain acceptance failures; SQL success does not establish Ask acceptance.

Full report: `.semantic-eval/commerce-luna-shared-context/run-258387-18db5d74eb787342/report.json`. SHA-256: `31140fb55ca6a8084d2f01541fecbe8c9d45d88c081a21d0665953bde15dc7d2`.

Ask outcomes are result 50, rejected 5, unsupported 44, needs_clarification 11 and unresolved 5. Outcome counts differ from pass counts because companions expect nonresult outcomes. Per-call reported input usage has median 33015, maximum 33927, across 122 calls. Reduced input size does not establish an accuracy causation claim. Historical baseline evidence is preserved.

The separate focused integer-width report passed 2/4 attempts: both SQL attempts passed, while roles.same Ask returned an invalid proposal and roles.different Ask returned nine rows rather than the expected five and failed the requested payer/recipient comparison. The reference SQL has no completed-order filter; the exact proposal failure remains subject to the separate debug capture. There are no provider errors. This does not prove comparison Ask failures are fixed. Focused report: `.semantic-eval/commerce-integer-widths/run-354597-18db5f4da501a828/report.json`. SHA-256: `3b622ca345cd70e6633d5ac0174765c53d48ddc295bdb113d518a2e4b6309008`. Its filtered scope is not a new full score.

## Priority disposition

The frozen-run reverse relationship gap is authentic: products→order_items is missing while item_product exists. A product_items equality role can be authored honestly, with multiple or absent right rows and no forged uniqueness. It still needs general nested existence and concept qualification to identify positive-quantity completed sales without fanout.

Currency asks need an exact Decimal/date/literal monetary conversion profile and requested half-away rounding; signed weighted ratios need exact finalization rather than a nonnegative weighted-average substitute. Historical lookup requires a supported governed half-open temporal path, preserving missing histories and detecting overlapping matches; ordinary equality alone must not imply as-of semantics. Calendar zero-filled month population remains distinct from a Date32 or UTC date mapping. Expansion-limit outcomes require bounded compiler investigation, not expected-failure flags or changed golds.

Clarification outcomes and wrong results need individual fidelity review. The ledger records observations without treating model explanations as proven product root causes. General proposals are in [the contract draft](semantic-eval-catalog-contract-drafts.md). Artifact authoring remains frozen until the parent's focused debug capture is terminal.

The following blocks copy all failed Ask case IDs, full diagnostics, provider errors, differences and actual rows without truncation.

## roles.different — unsupported

```json
{
  "id": "roles.different",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request requires comparing payer and recipient fields on the same order row, but field-to-field comparison supports only Int64 graph slots; both customer ID fields are Int32.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## absence.unsold_products — unsupported

```json
{
  "id": "absence.unsold_products",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The query requires finding products with no matching positive-quantity completed sale. The catalog does not author a relationship from products to qualifying sales; the available item_product relationship is defined from order_items to products. Reversing it would require an unsupported ordinary join.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## window.rank_gaps — needs_clarification

```json
{
  "id": "window.rank_gaps",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Should the final ordering use order ID ascending or descending after cents descending?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## existence.sold_products — unsupported

```json
{
  "id": "existence.sold_products",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The authored completed-order concept can only be applied to the selected relation, but the available product lookup starts from order_items and its related-order existence predicate cannot apply that concept. Reconstructing it as a status filter is not permitted.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## existence.only_ch_shipping — unsupported

```json
{
  "id": "existence.only_ch_shipping",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The condition requires checking each billed order’s recipient region through a nested order-to-recipient relationship. The available existence operation cannot express that nested lookup, and ordinary joins are unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## metrics.average — unsupported

```json
{
  "id": "metrics.average",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog supports the exact average of completed order totals in CHF cents, but the query protocol has no operation to round the result to two decimal places.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## metrics.customer_totals — result

```json
{
  "id": "metrics.customer_totals",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "column count: expected 2, actual 3"
  ],
  "actual": {
    "columns": [
      {
        "name": "customer_id",
        "type": "int32",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      },
      {
        "name": "billed_customer_name",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "completed_gross_sales_cents",
        "type": "int64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "5",
        "Éva",
        "-100"
      ],
      [
        "1",
        "Ada O'Brien",
        "6000"
      ],
      [
        "3",
        "Chen",
        "3500"
      ],
      [
        "99",
        null,
        "500"
      ],
      [
        "4",
        "Dario",
        "2000"
      ]
    ]
  }
}
```

## relationships.calendar — unsupported

```json
{
  "id": "relationships.calendar",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request requires a row-level fiscal-year lookup from the authored business calendar, but no supported query operation performs that calendar mapping.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## roles.same — unsupported

```json
{
  "id": "roles.same",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The query requires comparing the billing and shipping customer IDs on each order. Those fields are Int32, while the supported graph slot comparison is limited to Int64; an ordinary join or other field-to-field comparison is unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## sets.union — result

```json
{
  "id": "sets.union",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 5, actual 3"
  ],
  "actual": {
    "columns": [
      {
        "name": "buyer_id",
        "type": "int32",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "1"
      ],
      [
        "3"
      ],
      [
        "5"
      ]
    ]
  }
}
```

## fanout.refund_fraction — unsupported

```json
{
  "id": "fanout.refund_fraction",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The refunded and completed gross amounts require a multi-hop, multi-fact composition through order items that the authored relationships do not support as a single checked composition. The requested float result also requires an unsupported conversion.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## concept.gross_by_region — result

```json
{
  "id": "concept.gross_by_region",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 3, actual 4"
  ],
  "actual": {
    "columns": [
      {
        "name": "region",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "gross_booked_sales_cents",
        "type": "int64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "AT",
        "-100"
      ],
      [
        "CH",
        "9500"
      ],
      [
        "DE",
        "2000"
      ],
      [
        "FR",
        "900"
      ]
    ]
  }
}
```

## fanout.orders_subscriptions — needs_clarification

```json
{
  "id": "fanout.orders_subscriptions",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Which year do you mean for April 1 when determining active subscriptions?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## sets.except — needs_clarification

```json
{
  "id": "sets.except",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "For completed orders, should buyer IDs mean the billing customer IDs or the shipping/recipient customer IDs?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## relationships.shipping_totals — needs_clarification

```json
{
  "id": "relationships.shipping_totals",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Should completed sales with a missing or unmatched recipient keep a row under the recipient ID with a null name, or should those sales be excluded?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## window.lag — unsupported

```json
{
  "id": "window.lag",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request requires the cents amount from the previous completed order in date-and-ID order, which needs a lag window operation that is unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## fanout.weighted_price — unsupported

```json
{
  "id": "fanout.weighted_price",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested half-away-from-zero rounding to six decimal places is unsupported; the available ratio operation returns Decimal128(38,18) truncated toward zero and there is no authored rounding operation.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## projection.boolean_scope — unsupported

```json
{
  "id": "projection.boolean_scope",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The authored VIP concept cannot be composed as one branch of the requested OR predicate using the supported concept_filter operation.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## concept.zero_active — result

```json
{
  "id": "concept.zero_active",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "column count: expected 1, actual 2"
  ],
  "actual": {
    "columns": [
      {
        "name": "client_id",
        "type": "int32",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      },
      {
        "name": "client_name",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "3",
        "Chen"
      ]
    ]
  }
}
```

## calendar.missing_month — unsupported

```json
{
  "id": "calendar.missing_month",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Returning every authored calendar month with completed sales cents zero-filled for months without sales requires aligning the calendar-month rows with separately aggregated order sales. The catalog provides no authored relationship or supported calendar-scaffold operation for that alignment.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## existence.refunded_clients — unsupported

```json
{
  "id": "existence.refunded_clients",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "A distinct list of billed customer IDs is requested, but the supported row operations have no deduplication operation; qualifying order items can produce repeated customer IDs.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## reject.duplicate_rate — unsupported

```json
{
  "id": "reject.duplicate_rate",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog has no executable currency-conversion or currency-rate definition, and exchange_rates_duplicate is quarantined because its matches deliberately conflict.",
  "provider_errors": [],
  "differences": [
    "expected rejected, actual unsupported"
  ],
  "actual": null
}
```

## calculation.zero_ratio — unsupported

```json
{
  "id": "calculation.zero_ratio",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Computing the weighted unit price requires aggregating order-item fields while applying the authored pending-order concept on orders. The catalog supports the concept on orders and related existence checks, but not aggregating item rows through that relationship without an ordinary join.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## concept.purchases — result

```json
{
  "id": "concept.purchases",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "column count: expected 2, actual 3"
  ],
  "actual": {
    "columns": [
      {
        "name": "client_name",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "booked_purchase_count",
        "type": "int64",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      },
      {
        "name": "client_id",
        "type": "int32",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "Dario",
        "1",
        "4"
      ],
      [
        "Éva",
        "1",
        "5"
      ],
      [
        "Ada O'Brien",
        "3",
        "1"
      ],
      [
        "Chen",
        "2",
        "3"
      ],
      [
        null,
        "1",
        "99"
      ]
    ]
  }
}
```

## history.region_sales — unsupported

```json
{
  "id": "history.region_sales",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Historical payer region requires a temporal lookup from each completed order through its billing customer to customer_region_history, but no authored relationship or temporal path lookup defines that mapping. An ordinary join is unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## window.running — unsupported

```json
{
  "id": "window.running",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Row-grain ROWS window frames are unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## reject.invalid_rollup — needs_clarification

```json
{
  "id": "reject.invalid_rollup",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Should I calculate the governed overall weighted unit price as total line amount divided by total quantity, or the unweighted average of each product’s weighted unit price? These are different calculations.",
  "provider_errors": [],
  "differences": [
    "expected rejected, actual needs_clarification"
  ],
  "actual": null
}
```

## fanout.order_refund_count — unsupported

```json
{
  "id": "fanout.order_refund_count",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog has no authored relationship connecting refund events to buyers. Attributing linked refund counts by buyer would require a multi-hop join beyond the supported authored graph composition.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## fanout.per_buyer_net — unsupported

```json
{
  "id": "fanout.per_buyer_net",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog does not provide an authored relationship or supported lookup path to attribute refunds linked to completed orders to billed buyers for the requested net-by-customer calculation. Resolving this would require an unsupported multi-hop join.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## reject.overlap_history — unsupported

```json
{
  "id": "reject.overlap_history",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested customer_region_history_overlap relation is a quarantined diagnostic lookup with deliberately conflicting matches; the catalog directs rejecting governed lookups on nonunique matches.",
  "provider_errors": [],
  "differences": [
    "expected rejected, actual unsupported"
  ],
  "actual": null
}
```

## history.changed — unsupported

```json
{
  "id": "history.changed",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Comparing current and historical payer regions requires an authored temporal relationship to retrieve the history row and a field-to-field comparison. Neither is supported by the supplied catalog and query operations.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## calendar.monthly — unresolved

```json
{
  "id": "calendar.monthly",
  "interface": "ask",
  "passed": false,
  "incomplete": true,
  "outcome": "unresolved",
  "diagnostic": "expansion_limit: Context expansion found no additional authoritative context",
  "provider_errors": [],
  "differences": [
    "expected result, actual unresolved"
  ],
  "actual": null
}
```

## error.division_zero — unsupported

```json
{
  "id": "error.division_zero",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested zero-denominator behavior—raising an arithmetic error—is not supported. Ratio calculations allow only null or zero for a zero denominator.",
  "provider_errors": [],
  "differences": [
    "expected execution_error, actual unsupported"
  ],
  "actual": null
}
```

## fanout.product_net — unsupported

```json
{
  "id": "fanout.product_net",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog does not support combining line sales and line refunds and then re-aggregating their net amounts by product. An ordinary join or fanout could duplicate amounts, so this result cannot be safely produced with the available operations.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## metrics.customer_count_zero — rejected

```json
{
  "id": "metrics.customer_count_zero",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "rejected",
  "diagnostic": "composition_key_origin: Output group keys do not implement the authored relationship",
  "provider_errors": [],
  "differences": [
    "expected result, actual rejected"
  ],
  "actual": null
}
```

## calculation.line_allocation — unsupported

```json
{
  "id": "calculation.line_allocation",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "No executable authored allocation rule is defined for order_items. The catalog’s prose gives the net-allocation formula but states that an exact integral allocation is only established for a selected case; it does not provide an executable rule or authorize an exact-integer allocation for order 101.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## fanout.line_refunds — unsupported

```json
{
  "id": "fanout.line_refunds",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Refunds may have multiple events per line, so their total must be aggregated independently and aligned to order lines using both order_id and line_no. The authored graph composition can align on those keys but provides no supported operation to remove the extra order_id key from the final result, so it cannot return exactly the requested columns.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## calculation.exchange — unsupported

```json
{
  "id": "calculation.exchange",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog has no executable currency-conversion operation or rule for filtering exchange-rate rows to leap days. Producing the requested rounded CHF values requires unsupported computation.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## metrics.weight_sum — unsupported

```json
{
  "id": "metrics.weight_sum",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Calculating shipped weight requires multiplying each positive completed item quantity by its related product weight; computed multiplication is unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## window.top_per_buyer — rejected

```json
{
  "id": "window.top_per_buyer",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "rejected",
  "diagnostic": "invalid_proposal: Return exactly one documented typed proposal JSON shape",
  "provider_errors": [],
  "differences": [
    "expected result, actual rejected"
  ],
  "actual": null
}
```

## relationships.all_customers — unsupported

```json
{
  "id": "relationships.all_customers",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Returning each billed order ID alongside every customer while retaining customers without orders requires a one-to-many left join. The authored relationship supports existence or absence checks, not projecting related order rows.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## relationships.product_filter — unsupported

```json
{
  "id": "relationships.product_filter",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "This request requires combining order-item rows with related order and product data. The catalog provides no authored operation that can apply the completed concept to related orders while also returning product names; an ordinary join or reconstructed concept predicate is not permitted.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## fanout.net — unresolved

```json
{
  "id": "fanout.net",
  "interface": "ask",
  "passed": false,
  "incomplete": true,
  "outcome": "unresolved",
  "diagnostic": "expansion_limit: Context expansion found no additional authoritative context",
  "provider_errors": [],
  "differences": [
    "expected result, actual unresolved"
  ],
  "actual": null
}
```

## calendar.fiscal — unsupported

```json
{
  "id": "calendar.fiscal",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Applying the authored fiscal calendar to order dates requires mapping each order to its calendar row, but no executable calendar-lookup operation is available in this query protocol.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## window.region_running — unsupported

```json
{
  "id": "window.region_running",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested running window must partition by the billed customer’s looked-up region. Row-query windows support source-field inputs, but the cataloged lookup-region output is not an authorized row-window partition input.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## history.missing — unresolved

```json
{
  "id": "history.missing",
  "interface": "ask",
  "passed": false,
  "incomplete": true,
  "outcome": "unresolved",
  "diagnostic": "expansion_limit: Context expansion found no additional authoritative context",
  "provider_errors": [],
  "differences": [
    "expected result, actual unresolved"
  ],
  "actual": null
}
```

## metrics.empty_count — needs_clarification

```json
{
  "id": "metrics.empty_count",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Do you mean count orders whose status is exactly “lost”, or count orders with a missing status?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## sets.union_all — result

```json
{
  "id": "sets.union_all",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 11, actual 4"
  ],
  "actual": {
    "columns": [
      {
        "name": "customer_id",
        "type": "int32",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "1"
      ],
      [
        "3"
      ],
      [
        "5"
      ],
      [
        "5"
      ]
    ]
  }
}
```

## history.all — unresolved

```json
{
  "id": "history.all",
  "interface": "ask",
  "passed": false,
  "incomplete": true,
  "outcome": "unresolved",
  "diagnostic": "expansion_limit: Context expansion found no additional authoritative context",
  "provider_errors": [],
  "differences": [
    "expected result, actual unresolved"
  ],
  "actual": null
}
```

## calendar.business_day — unsupported

```json
{
  "id": "calendar.business_day",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog authors the Swiss business-day mapping, but the supported query operations cannot apply that mapping to orders. Doing so would require a join between orders.order_date and business_calendar.calendar_date.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## error.overflow — unsupported

```json
{
  "id": "error.overflow",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Scalar multiplication is not a supported query operation.",
  "provider_errors": [],
  "differences": [
    "expected execution_error, actual unsupported"
  ],
  "actual": null
}
```

## roles.unmatched_billing — rejected

```json
{
  "id": "roles.unmatched_billing",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "rejected",
  "diagnostic": "request_span: Requirement at index 1 source_text must equal the request substrings at UTF-8 byte ranges [RequestSpan { start: 19, end: 29 }, RequestSpan { start: 46, end: 78 }], joined by a space. Repair source_text or the spans without changing the requirement",
  "provider_errors": [],
  "differences": [
    "expected result, actual rejected"
  ],
  "actual": null
}
```

## projection.top — rejected

```json
{
  "id": "projection.top",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "rejected",
  "diagnostic": "request_span: Requirement at index 1 source_text must equal the request substrings at UTF-8 byte ranges [RequestSpan { start: 44, end: 46 }], joined by a space. Repair source_text or the spans without changing the requirement",
  "provider_errors": [],
  "differences": [
    "expected result, actual rejected"
  ],
  "actual": null
}
```

## history.current_vs_then — unsupported

```json
{
  "id": "history.current_vs_then",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog has no authored temporal relationship or path lookup from the billed customer to customer_region_history. The historical region lookup cannot be expressed safely without an ordinary join.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## fanout.line_count — unsupported

```json
{
  "id": "fanout.line_count",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog has no authored relationship connecting order-level refund counts to orders. Computing item and refund-event counts per order requires rolling up and combining facts across the item-to-refund relationship, which the supported composition operations cannot express at that grain.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## relationships.refund_composite — unsupported

```json
{
  "id": "relationships.refund_composite",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The authored refund_item relationship uses a composite key, but the two-hop lookup profile requires one key per hop. The requested product-name lookup cannot be represented by this profile while preserving the refund rows.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## absence.completed_buyers — unsupported

```json
{
  "id": "absence.completed_buyers",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The authored concept for “completed” can only be applied with concept_filter on the selected relation. The related-existence predicate for billed orders cannot invoke that concept, and reconstructing its predicate as an ordinary filter would bypass the authored definition.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## metrics.product_units — needs_clarification

```json
{
  "id": "metrics.product_units",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "May I include product ID as an additional result column so the grouped results can be ordered by it?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## clarify.currency — unsupported

```json
{
  "id": "clarify.currency",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Currency conversion requires applying a date-specific exchange rate, but no executable currency-conversion operation or authored rate lookup is available for this request.",
  "provider_errors": [],
  "differences": [
    "expected needs_clarification, actual unsupported"
  ],
  "actual": null
}
```

## relationships.left_scope — unsupported

```json
{
  "id": "relationships.left_scope",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request requires returning completed order IDs alongside every customer while preserving customers with no completed orders. The authored billed_orders relationship supports existence or absence conditions, but not a row-level one-to-many outer lookup that projects matching order IDs.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## metrics.having — result

```json
{
  "id": "metrics.having",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "column count: expected 2, actual 3"
  ],
  "actual": {
    "columns": [
      {
        "name": "customer_id",
        "type": "int32",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      },
      {
        "name": "buyer_name",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "sales_cents",
        "type": "int64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "1",
        "Ada O'Brien",
        "6000"
      ],
      [
        "3",
        "Chen",
        "3500"
      ]
    ]
  }
}
```

## error.invalid_cast — unsupported

```json
{
  "id": "error.invalid_cast",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Casting a Utf8 customer name to a signed bigint is not a supported operation.",
  "provider_errors": [],
  "differences": [
    "expected execution_error, actual unsupported"
  ],
  "actual": null
}
```

## history.boundary — unresolved

```json
{
  "id": "history.boundary",
  "interface": "ask",
  "passed": false,
  "incomplete": true,
  "outcome": "unresolved",
  "diagnostic": "expansion_limit: Context expansion found no additional authoritative context",
  "provider_errors": [],
  "differences": [
    "expected result, actual unresolved"
  ],
  "actual": null
}
```

## window.per_buyer — unsupported

```json
{
  "id": "window.per_buyer",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested row number requires a row_number window function, which is not supported by the available window operations.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## Separate focused integer-width failures

```json
{
  "id": "roles.same",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "rejected",
  "diagnostic": "invalid_proposal: Return exactly one documented typed proposal JSON shape",
  "provider_errors": [],
  "differences": [
    "expected result, actual rejected"
  ],
  "actual": null
}
```

```json
{
  "id": "roles.different",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 5, actual 9"
  ],
  "actual": {
    "columns": [
      {
        "name": "order_id",
        "type": "int64",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "101"
      ],
      [
        "102"
      ],
      [
        "103"
      ],
      [
        "104"
      ],
      [
        "106"
      ],
      [
        "107"
      ],
      [
        "108"
      ],
      [
        "109"
      ],
      [
        "110"
      ]
    ]
  }
}
```
