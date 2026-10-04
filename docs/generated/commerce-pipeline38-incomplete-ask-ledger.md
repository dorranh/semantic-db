# Commerce pipeline 38 Ask: incomplete resource-failure checkpoint

This is an INCOMPLETE checkpoint, not a full Ask score. The attempted full run stopped after 78 of 115 planned Ask attempts: 48 passing and 30 failing observed attempts. Report finalized=false and complete=false. The remaining 37 planned attempts are not evaluated evidence and cannot be treated as passes, failures or waived cases. Do not infer a full-suite lift or compare this partial pass fraction as a complete benchmark score.

Report `.semantic-eval/commerce-pipeline38-full-ask/run-412962-18db65503734e628/report.json`, SHA-256 `7ecb41d599d205853f2fcea4494a46c01ed0cb6df2ee4863625b3ce8c1876e08`; artifact `da95e740c30ebcc8325417ef8682f633809a0b3f6e861a5345f3435c6aff4c7d`. Parent reports process exit 1 before finalization after concurrent broad Cargo work exhausted disk. Log `/tmp/semantic-eval-commerce-pipeline38-full-ask.log` records NoSpaceLeft. Parent cleanup freed 99.4 GiB and manually removed the dataset-owned Compose project `eval-412962-18db655037354b82` through sg docker, exit 0 with cleanup verified. Manual cleanup is external evidence, not a retroactively finalized report.

The frozen artifacts remain unchanged while workspace verification references them. A fresh full run must use a new verified build and explicitly reviewed artifact revision. A filtered rerun or selected successful subset cannot replace this checkpoint or become a full score. Earlier full finalized reports remain separately preserved.

## All 30 observed failures

The following fields are copied without truncation from the checkpoint. They record observed outcomes, not independently proven causes; these failures remain genuine checkpoint evidence despite the later resource interruption.

### sets.intersect — needs_clarification

```json
{
  "id": "sets.intersect",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Should a customer count as a completed buyer when they are the billed/paying customer on a completed order, or when they are the shipping/recipient customer?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

### metrics.average — unsupported

```json
{
  "id": "metrics.average",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The authored average metric computes an exact Decimal128(38,18) mean, but the query operations do not support rounding its result to two decimal places.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### metrics.customer_totals — result

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
    "row count: expected 4, actual 5"
  ],
  "actual": {
    "columns": [
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
        "Éva",
        "-100"
      ],
      [
        "Dario",
        "2000"
      ],
      [
        "Ada O'Brien",
        "6000"
      ],
      [
        "Chen",
        "3500"
      ],
      [
        null,
        "500"
      ]
    ]
  }
}
```

### sets.union — result

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

### fanout.refund_fraction — unsupported

```json
{
  "id": "fanout.refund_fraction",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested ratio can be represented by the supported ratio operation only as Decimal128(38,18), not as Float64. A conversion to Float64 is unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### concept.gross_by_region — needs_clarification

```json
{
  "id": "concept.gross_by_region",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Should “booked sales” include only completed orders, or should it include orders of every status?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

### fanout.orders_subscriptions — needs_clarification

```json
{
  "id": "fanout.orders_subscriptions",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Which year’s April 1 should define the date for active monthly cents?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

### relationships.shipping_totals — result

```json
{
  "id": "relationships.shipping_totals",
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
        "name": "recipient_name",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "completed_sales_cents",
        "type": "int64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "recipient_id",
        "type": "int32",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "Éva",
        "-100",
        "5"
      ],
      [
        "Béatrice",
        "4500",
        "2"
      ],
      [
        "Ada O'Brien",
        "5500",
        "1"
      ],
      [
        "Dario",
        "1500",
        "4"
      ],
      [
        "Chen",
        "500",
        "3"
      ]
    ]
  }
}
```

### window.lag — unsupported

```json
{
  "id": "window.lag",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request for the previous completed order's cents requires a lag/previous-row calculation, which is not supported by the available window operations.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### fanout.weighted_price — unsupported

```json
{
  "id": "fanout.weighted_price",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested half-away-from-zero rounding to six decimal places is not supported by the available ratio operation, which truncates toward zero at 18 decimal places.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### reject.duplicate_rate — unsupported

```json
{
  "id": "reject.duplicate_rate",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested quarantined exchange_rates_duplicate lookup is a diagnostic source with conflicting matches, and the catalog requires rejecting governed lookups on nonunique matches. No executable currency-conversion definition authorizes this conversion.",
  "provider_errors": [],
  "differences": [
    "expected rejected, actual unsupported"
  ],
  "actual": null
}
```

### metrics.refund_sum — needs_clarification

```json
{
  "id": "metrics.refund_sum",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Should the total refund cents include all refund events while the count includes only positive-valued events, with the two results combined as independent totals?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

### concept.purchases — result

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
    "row 0: expected [String(\"Ada O'Brien\"), String(\"3\")], actual [String(\"Éva\"), String(\"1\")]",
    "row 1: expected [String(\"Chen\"), String(\"2\")], actual [String(\"Dario\"), String(\"1\")]",
    "row 2: expected [String(\"Dario\"), String(\"1\")], actual [String(\"Ada O'Brien\"), String(\"3\")]",
    "row 3: expected [String(\"Éva\"), String(\"1\")], actual [String(\"Chen\"), String(\"2\")]"
  ],
  "actual": {
    "columns": [
      {
        "name": "client name",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "booked purchase count",
        "type": "int64",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "Éva",
        "1"
      ],
      [
        "Dario",
        "1"
      ],
      [
        "Ada O'Brien",
        "3"
      ],
      [
        "Chen",
        "2"
      ]
    ]
  }
}
```

### history.region_sales — unsupported

```json
{
  "id": "history.region_sales",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Historical payer region requires an authored temporal relationship or lookup from orders to customer_region_history; none is defined in the supplied catalog.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### window.running — unsupported

```json
{
  "id": "window.running",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "A ROWS-frame running sum is unsupported for row-grain window calculations.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### reject.invalid_rollup — needs_clarification

```json
{
  "id": "reject.invalid_rollup",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Should I calculate the catalog-defined overall weighted unit price as total line amount divided by total quantity, or do you want an unweighted average of per-product weighted prices? The catalog defines the former and says not to average subgroup prices.",
  "provider_errors": [],
  "differences": [
    "expected rejected, actual needs_clarification"
  ],
  "actual": null
}
```

### fanout.order_refund_count — unsupported

```json
{
  "id": "fanout.order_refund_count",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Counting refund events per buyer requires aggregating refunds through the composite refund-to-item relationship and then through items to orders, while preserving completed-order qualification and regrouping by buyer. The authored relationships and graph operations do not support this multi-hop many-to-many aggregation without ordinary joins.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### fanout.per_buyer_net — needs_clarification

```json
{
  "id": "fanout.per_buyer_net",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Should the result include every known billed buyer, including those with no completed sales, or only buyers with completed sales? For buyers with no completed sales, should net sales be null or zero?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

### reject.overlap_history — unsupported

```json
{
  "id": "reject.overlap_history",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested lookup uses a quarantined diagnostic relation with deliberately conflicting matches, which the catalog says must be rejected for governed lookups. No authored temporal relationship supports this lookup.",
  "provider_errors": [],
  "differences": [
    "expected rejected, actual unsupported"
  ],
  "actual": null
}
```

### history.changed — unsupported

```json
{
  "id": "history.changed",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog has no authored temporal relationship or lookup path from the billing customer to customer_region_history. Resolving the historical payer region requires an unsupported ordinary join.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### error.division_zero — unsupported

```json
{
  "id": "error.division_zero",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The ratio operation supports only null or zero for a zero denominator; it cannot require an arithmetic error.",
  "provider_errors": [],
  "differences": [
    "expected execution_error, actual unsupported"
  ],
  "actual": null
}
```

### fanout.product_net — unsupported

```json
{
  "id": "fanout.product_net",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested sales-minus-refunds calculation requires subtracting independently aggregated measures. The supported graph calculation operation provides ratios, not subtraction.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### metrics.customer_count_zero — result

```json
{
  "id": "metrics.customer_count_zero",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row 0: expected [String(\"1\"), String(\"3\")], actual [String(\"6\"), String(\"0\")]",
    "row 1: expected [String(\"2\"), String(\"0\")], actual [String(\"1\"), String(\"3\")]",
    "row 3: expected [String(\"4\"), String(\"1\")], actual [String(\"5\"), String(\"1\")]",
    "row 4: expected [String(\"5\"), String(\"1\")], actual [String(\"4\"), String(\"1\")]",
    "row 5: expected [String(\"6\"), String(\"0\")], actual [String(\"2\"), String(\"0\")]"
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
        "name": "completed_billed_order_count",
        "type": "int64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "6",
        "0"
      ],
      [
        "1",
        "3"
      ],
      [
        "3",
        "2"
      ],
      [
        "5",
        "1"
      ],
      [
        "4",
        "1"
      ],
      [
        "2",
        "0"
      ]
    ]
  }
}
```

### calculation.line_allocation — unsupported

```json
{
  "id": "calculation.line_allocation",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog has no executable authored allocation rule for order_items. Its descriptive allocation guidance does not define a checked rule guaranteeing exact integer allocations for order 101.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### calculation.exchange — unsupported

```json
{
  "id": "calculation.exchange",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog does not provide an executable currency-conversion rule or a supported operation for multiplying the rate by ±100 and rounding the result half away from zero to two decimals. Selecting all leap-day rate dates would also require date-part filtering, which is unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### metrics.weight_sum — unsupported

```json
{
  "id": "metrics.weight_sum",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Calculating total shipped weight requires multiplying each product's weight_kg by its item quantity and summing the results; this computed expression is unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### window.top_per_buyer — rejected

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

### relationships.all_customers — unsupported

```json
{
  "id": "relationships.all_customers",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Returning each billed order ID requires expanding customers to multiple matching orders while retaining customers with no matches. The authored billed_orders relationship is one-to-many; row lookups require a checked unique match, and ordinary multiplying joins are unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### fanout.net — unsupported

```json
{
  "id": "fanout.net",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog supports aggregating completed sales and linked refunds separately, but the query protocol has no operation for subtracting those aggregate totals.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

### calendar.fiscal — unsupported

```json
{
  "id": "calendar.fiscal",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The governed gross-sales metric does not authorize grouping by the calendar lookup’s fiscal-year field.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```
