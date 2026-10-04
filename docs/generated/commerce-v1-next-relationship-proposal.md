# Commerce relationship revision and deferred proposals: exhaustive fixture audit

Both dataset bundles remain frozen. The minimal product_items reverse equality relationship is now published in commerce v1.0.1, with independently reviewed regeneration and fresh SQL validation. The optional refund_order/order_refunds objects below remain unloaded review proposals. The JSON records the original proposal; published product_items instructions describe actual item/order facts without claiming nested capability is implemented. All objects use existing equality syntax and real source columns. Cardinality remains Unknown under the current importer; observed fixture multiplicities do not establish enforced uniqueness.

## Published minimal edge and deferred optional proposals

```json
[
  {
    "name": "product_items",
    "from": "products",
    "to": "order_items",
    "from_columns": [
      "product_id"
    ],
    "to_columns": [
      "product_id"
    ],
    "ai_context": {
      "instructions": "Item lines referencing this catalog product. Multiple lines or no matching lines are possible. Existence qualification by positive quantity and completed order uses declared nested scopes; this equality edge does not itself imply either condition.",
      "synonyms": [
        "product item lines"
      ]
    }
  },
  {
    "name": "refund_order",
    "from": "refunds",
    "to": "orders",
    "from_columns": [
      "order_id"
    ],
    "to_columns": [
      "order_id"
    ],
    "ai_context": {
      "instructions": "The order referenced by this refund event. Refund amounts aggregate at refund event grain; looking up an order must not repeat its total as a refund measure."
    }
  },
  {
    "name": "order_refunds",
    "from": "orders",
    "to": "refunds",
    "from_columns": [
      "order_id"
    ],
    "to_columns": [
      "order_id"
    ],
    "ai_context": {
      "instructions": "All refund events referencing this order, independent of its item-line population. Multiple or no matching events are possible; aggregate refunds independently before combining order and item measures."
    }
  }
]
```

`product_items` is the missing inverse of existing item_product. Its source provenance is the preserved products CSV and order_items PostgreSQL fact table. `refund_order` and `order_refunds` use refund.order_id and order.order_id already present in preserved PostgreSQL facts; they are genuine direct domain edges, not case-specific derived answer fields. They supplement the composite refund_item/line_refunds pair rather than replacing either component. No temporal/customer-history edge is proposed as ordinary equality: the half-open contract needs a real supported temporal-path profile.

## Canonical fixture audit

Every source row was checked against every target row using SQL equality semantics: any null key component prevents matching. Empty strings remain ordinary values. Counts below are exhaustive for this canonical revision, not product output. Source-row keys are identity fields; composite item keys preserve order_id and line_no. Parent reader/schema fixtures enforce physical contracts independently.


### billing_customer

`orders(billing_customer_id) → customers(customer_id)`: 10 source rows; null-key rows 0; unmatched nonnull rows 1; rows matching multiple targets 0.

```json
[
  {
    "source_identity": [
      101
    ],
    "key": [
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      102
    ],
    "key": [
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      103
    ],
    "key": [
      2
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      104
    ],
    "key": [
      3
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      105
    ],
    "key": [
      2
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      106
    ],
    "key": [
      4
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      107
    ],
    "key": [
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      108
    ],
    "key": [
      5
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      109
    ],
    "key": [
      99
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      110
    ],
    "key": [
      3
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

### shipping_customer

`orders(shipping_customer_id) → customers(customer_id)`: 10 source rows; null-key rows 1; unmatched nonnull rows 0; rows matching multiple targets 0.

```json
[
  {
    "source_identity": [
      101
    ],
    "key": [
      2
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      102
    ],
    "key": [
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      103
    ],
    "key": [
      3
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      104
    ],
    "key": [
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      105
    ],
    "key": [
      null
    ],
    "matching_rows": 0,
    "null_key": true
  },
  {
    "source_identity": [
      106
    ],
    "key": [
      2
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      107
    ],
    "key": [
      4
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      108
    ],
    "key": [
      5
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      109
    ],
    "key": [
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      110
    ],
    "key": [
      3
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

### item_order

`order_items(order_id) → orders(order_id)`: 11 source rows; null-key rows 0; unmatched nonnull rows 0; rows matching multiple targets 0.

```json
[
  {
    "source_identity": [
      101,
      1
    ],
    "key": [
      101
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      101,
      2
    ],
    "key": [
      101
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      102,
      1
    ],
    "key": [
      102
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      103,
      1
    ],
    "key": [
      103
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      104,
      1
    ],
    "key": [
      104
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      105,
      1
    ],
    "key": [
      105
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      106,
      1
    ],
    "key": [
      106
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      107,
      1
    ],
    "key": [
      107
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      108,
      1
    ],
    "key": [
      108
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      109,
      1
    ],
    "key": [
      109
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      110,
      1
    ],
    "key": [
      110
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

### item_product

`order_items(product_id) → products(product_id)`: 11 source rows; null-key rows 0; unmatched nonnull rows 1; rows matching multiple targets 0.

```json
[
  {
    "source_identity": [
      101,
      1
    ],
    "key": [
      10
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      101,
      2
    ],
    "key": [
      20
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      102,
      1
    ],
    "key": [
      10
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      103,
      1
    ],
    "key": [
      10
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      104,
      1
    ],
    "key": [
      20
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      105,
      1
    ],
    "key": [
      30
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      106,
      1
    ],
    "key": [
      30
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      107,
      1
    ],
    "key": [
      20
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      108,
      1
    ],
    "key": [
      10
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      109,
      1
    ],
    "key": [
      999
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      110,
      1
    ],
    "key": [
      10
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

### refund_item

`refunds(order_id, line_no) → order_items(order_id, line_no)`: 5 source rows; null-key rows 0; unmatched nonnull rows 0; rows matching multiple targets 0.

```json
[
  {
    "source_identity": [
      1
    ],
    "key": [
      101,
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      2
    ],
    "key": [
      101,
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      3
    ],
    "key": [
      104,
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      4
    ],
    "key": [
      106,
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      5
    ],
    "key": [
      110,
      1
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

### subscriber

`subscriptions(customer_id) → customers(customer_id)`: 5 source rows; null-key rows 0; unmatched nonnull rows 0; rows matching multiple targets 0.

```json
[
  {
    "source_identity": [
      1
    ],
    "key": [
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      2
    ],
    "key": [
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      3
    ],
    "key": [
      2
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      4
    ],
    "key": [
      3
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      5
    ],
    "key": [
      5
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

### billed_orders

`customers(customer_id) → orders(billing_customer_id)`: 6 source rows; null-key rows 0; unmatched nonnull rows 1; rows matching multiple targets 3.

```json
[
  {
    "source_identity": [
      1
    ],
    "key": [
      1
    ],
    "matching_rows": 3,
    "null_key": false
  },
  {
    "source_identity": [
      2
    ],
    "key": [
      2
    ],
    "matching_rows": 2,
    "null_key": false
  },
  {
    "source_identity": [
      3
    ],
    "key": [
      3
    ],
    "matching_rows": 2,
    "null_key": false
  },
  {
    "source_identity": [
      4
    ],
    "key": [
      4
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      5
    ],
    "key": [
      5
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      6
    ],
    "key": [
      6
    ],
    "matching_rows": 0,
    "null_key": false
  }
]
```

### received_orders

`customers(customer_id) → orders(shipping_customer_id)`: 6 source rows; null-key rows 0; unmatched nonnull rows 1; rows matching multiple targets 3.

```json
[
  {
    "source_identity": [
      1
    ],
    "key": [
      1
    ],
    "matching_rows": 3,
    "null_key": false
  },
  {
    "source_identity": [
      2
    ],
    "key": [
      2
    ],
    "matching_rows": 2,
    "null_key": false
  },
  {
    "source_identity": [
      3
    ],
    "key": [
      3
    ],
    "matching_rows": 2,
    "null_key": false
  },
  {
    "source_identity": [
      4
    ],
    "key": [
      4
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      5
    ],
    "key": [
      5
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      6
    ],
    "key": [
      6
    ],
    "matching_rows": 0,
    "null_key": false
  }
]
```

### customer_subscriptions

`customers(customer_id) → subscriptions(customer_id)`: 6 source rows; null-key rows 0; unmatched nonnull rows 2; rows matching multiple targets 1.

```json
[
  {
    "source_identity": [
      1
    ],
    "key": [
      1
    ],
    "matching_rows": 2,
    "null_key": false
  },
  {
    "source_identity": [
      2
    ],
    "key": [
      2
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      3
    ],
    "key": [
      3
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      4
    ],
    "key": [
      4
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      5
    ],
    "key": [
      5
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      6
    ],
    "key": [
      6
    ],
    "matching_rows": 0,
    "null_key": false
  }
]
```

### order_lines

`orders(order_id) → order_items(order_id)`: 10 source rows; null-key rows 0; unmatched nonnull rows 0; rows matching multiple targets 1.

```json
[
  {
    "source_identity": [
      101
    ],
    "key": [
      101
    ],
    "matching_rows": 2,
    "null_key": false
  },
  {
    "source_identity": [
      102
    ],
    "key": [
      102
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      103
    ],
    "key": [
      103
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      104
    ],
    "key": [
      104
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      105
    ],
    "key": [
      105
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      106
    ],
    "key": [
      106
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      107
    ],
    "key": [
      107
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      108
    ],
    "key": [
      108
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      109
    ],
    "key": [
      109
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      110
    ],
    "key": [
      110
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

### line_refunds

`order_items(order_id, line_no) → refunds(order_id, line_no)`: 11 source rows; null-key rows 0; unmatched nonnull rows 7; rows matching multiple targets 1.

```json
[
  {
    "source_identity": [
      101,
      1
    ],
    "key": [
      101,
      1
    ],
    "matching_rows": 2,
    "null_key": false
  },
  {
    "source_identity": [
      101,
      2
    ],
    "key": [
      101,
      2
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      102,
      1
    ],
    "key": [
      102,
      1
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      103,
      1
    ],
    "key": [
      103,
      1
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      104,
      1
    ],
    "key": [
      104,
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      105,
      1
    ],
    "key": [
      105,
      1
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      106,
      1
    ],
    "key": [
      106,
      1
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      107,
      1
    ],
    "key": [
      107,
      1
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      108,
      1
    ],
    "key": [
      108,
      1
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      109,
      1
    ],
    "key": [
      109,
      1
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      110,
      1
    ],
    "key": [
      110,
      1
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

### product_items

`products(product_id) → order_items(product_id)`: 4 source rows; null-key rows 0; unmatched nonnull rows 1; rows matching multiple targets 3.

```json
[
  {
    "source_identity": [
      10
    ],
    "key": [
      10
    ],
    "matching_rows": 5,
    "null_key": false
  },
  {
    "source_identity": [
      20
    ],
    "key": [
      20
    ],
    "matching_rows": 3,
    "null_key": false
  },
  {
    "source_identity": [
      30
    ],
    "key": [
      30
    ],
    "matching_rows": 2,
    "null_key": false
  },
  {
    "source_identity": [
      40
    ],
    "key": [
      40
    ],
    "matching_rows": 0,
    "null_key": false
  }
]
```

### refund_order

`refunds(order_id) → orders(order_id)`: 5 source rows; null-key rows 0; unmatched nonnull rows 0; rows matching multiple targets 0.

```json
[
  {
    "source_identity": [
      1
    ],
    "key": [
      101
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      2
    ],
    "key": [
      101
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      3
    ],
    "key": [
      104
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      4
    ],
    "key": [
      106
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      5
    ],
    "key": [
      110
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

### order_refunds

`orders(order_id) → refunds(order_id)`: 10 source rows; null-key rows 0; unmatched nonnull rows 6; rows matching multiple targets 1.

```json
[
  {
    "source_identity": [
      101
    ],
    "key": [
      101
    ],
    "matching_rows": 2,
    "null_key": false
  },
  {
    "source_identity": [
      102
    ],
    "key": [
      102
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      103
    ],
    "key": [
      103
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      104
    ],
    "key": [
      104
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      105
    ],
    "key": [
      105
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      106
    ],
    "key": [
      106
    ],
    "matching_rows": 1,
    "null_key": false
  },
  {
    "source_identity": [
      107
    ],
    "key": [
      107
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      108
    ],
    "key": [
      108
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      109
    ],
    "key": [
      109
    ],
    "matching_rows": 0,
    "null_key": false
  },
  {
    "source_identity": [
      110
    ],
    "key": [
      110
    ],
    "matching_rows": 1,
    "null_key": false
  }
]
```

## Independently checked qualifying populations

```json
{
  "10": [
    [
      101,
      1
    ],
    [
      102,
      1
    ],
    [
      110,
      1
    ]
  ],
  "20": [
    [
      101,
      2
    ],
    [
      104,
      1
    ],
    [
      107,
      1
    ]
  ],
  "30": [
    [
      106,
      1
    ]
  ],
  "40": []
}
```

Product 40 has no item matches and is genuinely unsold; the other catalog products each have qualifying completed positive-quantity items. Unknown product 999 is an item-side orphan and cannot invent a catalog product row. Item 105/1 has zero quantity, while 108/1 has negative quantity; neither qualifies. Multiple qualifying lines for a product must produce a single existence decision, not duplicated catalog rows. This dataset does not separately distinguish every pending-only/credit-only product population; those are meaningful generic compiler test fixtures, not coverage claims about these golds.

## Compiler and minimum next batch disposition

Current Related binds one declared relationship/role/instance plus optional RowPredicate. ConceptFilter resolves concepts on the current root relation; the predicate grammar does not compose nested relationship scopes or invoke a pinned concept in a related occurrence. Therefore the smallest complete unsold-product fix is the product_items author edge plus general bounded nested existence and related concept expansion. Expand the existing completed concept on the order occurrence; preserve host-bound concept arguments, source definition revisions and SQL null semantics. EXISTS keeps the starting product population independent of item fanout.

Existing direct row lookup/filter-output/order capabilities should be checked before adding any new answer-bearing metadata: full BIRD model refusals about related birth-date filters and ordering may reflect proposal/catalog presentation gaps rather than wholly absent execution features. Check supported physical types and occurrence scopes case by case. Calendar previous-month and active-date contracts are already authored; their remaining failures need actual debug evidence, not redundant invented concepts. Percentage requests need a generic scale factor in ratio finalization; numeric text and duration requests need guarded typed parsing. These capabilities cannot be repaired by pretending unparsed strings are numeric or authoring one metric per expected answer.

Exact rate/date/literal conversion and signed ratio half-away rounding remain larger genuine general-profile work. Implementer confirmed the current Ossie importer has no RelationshipPath/as_of wrapper. Existing compiler temporal-path contracts need genuine generic importer support before authoring; do not publish equality as an as-of guarantee. SQL, golds, tolerances and source facts remain unchanged throughout this proposal.

Provenance: canonical JSON SHA-256 `44c4481f6280b0dfe14ba67d3a9d9a410f6402ad71e8257bed236489a2b9cb50`; model SHA-256 `946b48c5d07ad94268031f1776104f6041c6168cd160a9514933d8611e5be9fc`.

## Approved minimal patch and next-capability checks

Reviewer independently reproduced all 14 exhaustive edge audits, qualifying memberships and source hashes. Parent approved only product_items for the next artifact author revision; refund_order/order_refunds remain optional deferred proposals until a capability consumes them. The product_items edge has now been authored through the generator and published in v1.0.1; the optional refund edges have not been loaded. Do not commit independently. No source, question, SQL, expected result, tolerance or comparison setting changes accompany the edge.

The approved general compiler design is conjunctive Related.target_requirements containing existing Filter, ConceptFilter and Related requirements, with globally unique requirement IDs and strict request evidence. The following test outline uses existing acceptance cases; it does not add expected exemptions:

| Existing case | Scoped requirements | Preserved expected behavior |
| --- | --- | --- |
| absence.unsold_products | product_items absence; target item quantity > 0 AND nested item_order existence; target order completed ConceptFilter | Product 40 only; qualifying products occur once despite multiple matches. |
| absence.completed_buyers | billed_orders absence; target order completed ConceptFilter | Exclude a customer only when a completed billed order exists; pending orders alone do not qualify. |
| absence.buyers | billed_orders absence without target filters | Any-status billed order prevents inclusion; preserves original population. |
| absence.no_subscriptions | customer_subscriptions absence without target filters | Any-status subscription prevents inclusion. |
| concept.zero_active | customer_subscriptions existence; target monthly_minor = 0 AND active_at_date ConceptFilter(as_of_date April 1, 2024) | Match active zero-priced subscriptions under exact half-open membership dates; current case's customer grain and output order remain unchanged. |

For prepared-path generic tests, reject duplicate requirement IDs anywhere in the tree, invalid child operation kinds, undeclared roles, wrong occurrence field scope and mismatched concept parameter types. Validate evidence against the original request on every child; a parent evidence span must not authorize invented child conditions. A single unrelated failed child must make the conjunction false; separate related occurrences must not satisfy different conditions that are required on the same item/order. Nested NOT EXISTS retains the starting population and does not multiply rows. Missing/null equality keys must not match. Bind Date32 arguments through the host parameter mechanism, with pinned concept and relationship definition references retained in compilation provenance.

When authoring is released, record the pre/post model and whole-artifact digests, generator change and declared dataset version policy. Verify the regenerated proposal is exactly the approved product_items object and compare all SQL strings, question strings, expected-file bytes, tolerances and canonical fixture bytes to the frozen baseline. The new artifact digest must be reported explicitly; historical run digests remain associated with their original frozen artifacts.

## Published revision verification

Independent review regenerated all 148 files byte-identically to the authored v1.0.1 bundle. Parent offline validation passed. Fresh full SQL evaluation passed 103/103, with complete/finalized true and no setup, cleanup or artifact errors: `.semantic-eval/commerce-product-items-sql/run-382985-18db6199c5bd232c/report.json`, report SHA-256 `7d2df4776e0d10cdd24bbdbae3e80d7bc543a81df1e755d0b9e717ede46a3a27`. Artifact digest is `d435dffd97a15fd0994de19ffc595d56c571494404a6dee65704f79626d07a7a`. This verifies SQL regression behavior; nested-related Ask acceptance remains a separate capability and runtime check. Bundle remains frozen.
