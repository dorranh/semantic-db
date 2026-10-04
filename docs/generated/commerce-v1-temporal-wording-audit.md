# Read-only complete commerce temporal wording audit

All 115 questions/reference contracts inspected; no artifact edits while the frozen full Ask run is active. This review is independent of the new FX operation's supported date grammar and separate from the four role/window planned clarifications. Exact-date question precision must reflect the intended reference task, not accommodate an implementation limitation.

The request context pins 2024-04-01T00:00:00Z and Europe/Zurich. That can reasonably resolve an April 1 mention to the current-year date, but there is no authored universal partial-date resolution policy. “Leap day” may similarly be contextually 2024, or describe recurring February 29; it does not automatically mean all leap days or prove a contradiction. The exact reference asks one date. Explicit dates are therefore proposed as visible precision improvements, with independent reviewer adjudication required; no model refusal is automatic proof of ambiguity.

## Proposed date precision candidates


### fanout.orders_subscriptions

Before: For every customer return name, completed billed sales cents (zero if none), then active monthly cents on April 1, by ID.

Proposed: For every customer return name, completed billed sales cents (zero if none), then active monthly cents on April 1, 2024, by ID.

Reason: Partial-date precision: pinned clock is itself April 1, 2024 in Zurich, so contextual current-year resolution is reasonable. It is not universally mandatory to clarify a missing year. An explicit 2024 makes the independently authored reference date unambiguous without changing scope.

### calculation.exchange

Before: For each leap-day exchange rate, return currency, exact CHF value of 100 foreign units, then value of negative 100 units; round half away from zero to two decimals and order by currency.

Proposed: For each exchange rate on February 29, 2024, return currency, exact CHF value of 100 foreign units, then value of negative 100 units; round half away from zero to two decimals and order by currency.

Reason: Exact-date precision: leap-day rates can denote all February 29 dates across years or a contextual single date. The reference restricts exactly 2024-02-29. No all-leap-days definition or nearest/current-leap-day resolution is authored. State the intended date explicitly; do not choose this solely because a new operation supports exact dates.

### reject.duplicate_rate

Before: Convert 100 EUR to CHF on leap day using the quarantined exchange_rates_duplicate lookup.

Proposed: Convert 100 EUR to CHF on February 29, 2024 using the quarantined exchange_rates_duplicate lookup.

Reason: Same leap-day date precision as the row-wise request, independently of the planned stronger execution-stage negative metadata. Preserve current expected outcome until its separately approved migration.

## Other temporal semantics

Explicit 2024 dates in active/zero_active/active_before, leap and union cases agree with SQL; previous_month explicitly anchors March to the fixed April 1, 2024 clock. Histories use the order_date under the existing authored half-open history definition, not request time; no new date qualifier is required there. Fiscal/calendar month/business-day queries request existing authored mappings and populations, not an inferred contiguous date range. DST wall/instant questions distinguish actual types and selected IDs. Running/lag windows explicitly request date and ID ordering; window.per_buyer already has a separate completed-population correction plan and is not bundled here. As-of overlap and unavailable future-rate companions specify exact dates.

This audit does not introduce an active-now default, nearest available FX date, historical current-region fallback, inferred date ranges or timezone reinterpretation of Date32. Monetary signs, rounding, column order, status/grain and null retention remain unchanged. A future date clarification must record original/effective wording and independently prove unchanged SQL, golds, tolerances, data, comparison settings and all other questions.

## Exhaustive coverage

| Case | Temporal scope disposition |
| --- | --- |
| projection.vip | No independent temporal date restriction requested |
| projection.empty | No independent temporal date restriction requested |
| projection.null | No independent temporal date restriction requested |
| roles.billing | No independent temporal date restriction requested |
| roles.shipping | No independent temporal date restriction requested |
| metrics.gross | No independent temporal date restriction requested |
| relationships.products | No independent temporal date restriction requested |
| absence.buyers | No independent temporal date restriction requested |
| fanout.net | No independent temporal date restriction requested |
| history.boundary | Order-date governed as-of or explicit companion date |
| projection.unicode | No independent temporal date restriction requested |
| projection.apostrophe | No independent temporal date restriction requested |
| projection.negation | No independent temporal date restriction requested |
| projection.boolean_scope | No independent temporal date restriction requested |
| projection.top | No independent temporal date restriction requested |
| projection.negative | No independent temporal date restriction requested |
| projection.empty_result | No independent temporal date restriction requested |
| projection.float | No independent temporal date restriction requested |
| projection.width | No independent temporal date restriction requested |
| concept.completed | No independent temporal date restriction requested |
| concept.active | Explicit date/year/reference anchor |
| concept.active_before | Explicit date/year/reference anchor |
| concept.buyers | No independent temporal date restriction requested |
| concept.nested | No independent temporal date restriction requested |
| concept.purchases | No independent temporal date restriction requested |
| concept.gross_by_region | No independent temporal date restriction requested |
| concept.zero_active | Explicit date/year/reference anchor |
| roles.both | No independent temporal date restriction requested |
| roles.different | No independent temporal date restriction requested |
| roles.same | No independent temporal date restriction requested |
| roles.unmatched_billing | No independent temporal date restriction requested |
| roles.shipping_fr | No independent temporal date restriction requested |
| relationships.product_filter | No independent temporal date restriction requested |
| relationships.refund_composite | No independent temporal date restriction requested |
| relationships.subscribers | No independent temporal date restriction requested |
| relationships.calendar | Authored calendar/population or explicit selected-row temporal projection |
| relationships.all_customers | No independent temporal date restriction requested |
| relationships.left_scope | No independent temporal date restriction requested |
| relationships.shipping_totals | No independent temporal date restriction requested |
| existence.completed_buyers | No independent temporal date restriction requested |
| absence.completed_buyers | No independent temporal date restriction requested |
| existence.sold_products | No independent temporal date restriction requested |
| absence.unsold_products | No independent temporal date restriction requested |
| existence.refunded_clients | No independent temporal date restriction requested |
| absence.no_subscriptions | No independent temporal date restriction requested |
| existence.only_ch_shipping | No independent temporal date restriction requested |
| metrics.order_count | No independent temporal date restriction requested |
| metrics.distinct_buyers | No independent temporal date restriction requested |
| metrics.customer_totals | No independent temporal date restriction requested |
| metrics.customer_count_zero | No independent temporal date restriction requested |
| metrics.minmax | No independent temporal date restriction requested |
| metrics.average | No independent temporal date restriction requested |
| metrics.empty_sum | No independent temporal date restriction requested |
| metrics.empty_count | No independent temporal date restriction requested |
| metrics.note_counts | No independent temporal date restriction requested |
| metrics.status | No independent temporal date restriction requested |
| metrics.having | No independent temporal date restriction requested |
| metrics.refund_sum | No independent temporal date restriction requested |
| metrics.product_units | No independent temporal date restriction requested |
| metrics.weight_sum | No independent temporal date restriction requested |
| fanout.order_item | No independent temporal date restriction requested |
| fanout.per_buyer_net | No independent temporal date restriction requested |
| fanout.line_refunds | No independent temporal date restriction requested |
| fanout.orders_subscriptions | Date precision candidate; contextual reading may already be reasonable |
| fanout.product_net | No independent temporal date restriction requested |
| fanout.weighted_price | No independent temporal date restriction requested |
| fanout.order_refund_count | No independent temporal date restriction requested |
| fanout.refund_fraction | No independent temporal date restriction requested |
| fanout.line_count | No independent temporal date restriction requested |
| calendar.leap | Explicit date/year/reference anchor |
| calendar.previous_month | Explicit date/year/reference anchor |
| calendar.fiscal | Authored calendar/population or explicit selected-row temporal projection |
| calendar.business_day | Authored calendar/population or explicit selected-row temporal projection |
| calendar.monthly | Authored calendar/population or explicit selected-row temporal projection |
| calendar.missing_month | Authored calendar/population or explicit selected-row temporal projection |
| calendar.spring_wall | Authored calendar/population or explicit selected-row temporal projection |
| calendar.autumn_instant | Authored calendar/population or explicit selected-row temporal projection |
| calendar.fraction | Authored calendar/population or explicit selected-row temporal projection |
| calendar.year_boundary | Authored calendar/population or explicit selected-row temporal projection |
| history.missing | Order-date governed as-of or explicit companion date |
| history.current_vs_then | Order-date governed as-of or explicit companion date |
| history.all | Order-date governed as-of or explicit companion date |
| history.region_sales | Order-date governed as-of or explicit companion date |
| history.changed | Order-date governed as-of or explicit companion date |
| window.rank | Explicit window ordering/grain; no new date restriction |
| window.rank_gaps | Explicit window ordering/grain; no new date restriction |
| window.per_buyer | Separate completed-population correction planned; not a date erratum |
| window.running | Explicit window ordering/grain; no new date restriction |
| window.region_running | Explicit window ordering/grain; no new date restriction |
| window.lag | Explicit window ordering/grain; no new date restriction |
| window.top_per_buyer | Explicit window ordering/grain; no new date restriction |
| window.final_limit | Explicit window ordering/grain; no new date restriction |
| sets.union_all | Explicit date/year/reference anchor |
| sets.union | Explicit date/year/reference anchor |
| sets.intersect | No independent temporal date restriction requested |
| sets.except | No independent temporal date restriction requested |
| calculation.line_allocation | No independent temporal date restriction requested |
| calculation.zero_ratio | No independent temporal date restriction requested |
| calculation.tax | No independent temporal date restriction requested |
| calculation.exchange | Date precision candidate; contextual reading may already be reasonable |
| clarify.customer_role | No independent temporal date restriction requested |
| clarify.healthy | No independent temporal date restriction requested |
| clarify.currency | No independent temporal date restriction requested |
| clarify.period | No independent temporal date restriction requested |
| clarify.top_metric | No independent temporal date restriction requested |
| reject.denied_scope | No independent temporal date restriction requested |
| reject.invalid_rollup | No independent temporal date restriction requested |
| reject.duplicate_rate | Date precision candidate; contextual reading may already be reasonable |
| reject.overlap_history | Order-date governed as-of or explicit companion date |
| unsupported.payment | No independent temporal date restriction requested |
| unsupported.future_rate | Explicit date/year/reference anchor |
| unsupported.residual | No independent temporal date restriction requested |
| error.invalid_cast | No independent temporal date restriction requested |
| error.division_zero | No independent temporal date restriction requested |
| error.overflow | No independent temporal date restriction requested |
