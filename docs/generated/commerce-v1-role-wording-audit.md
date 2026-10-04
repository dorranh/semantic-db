# Read-only complete commerce role wording audit

All 115 cases were inspected against their questions and reference SQL. No artifact change is made while the full pipeline 38 Ask run is active. A model clarification alone is not proof of ambiguity: already explicit billed/payer/recipient wording must be honored. Current buyers/clients are customer entity aliases, and the authored billing default applies to customer sales attribution metrics. That does not automatically establish a universal billing role for every set/count request.

## Three minimal clarification candidates for independent review

Purchasing “buyer” often naturally means payer; these candidates therefore need independent business adjudication rather than assuming every model refusal is justified. The minimal word billed states the existing intended role without altering grain, known-identity checks, status, subscription population, counts or output. No expected values are supplied.


### fanout.order_refund_count

Before: Return each known buyer name, completed order count, then linked refund event count, by buyer ID.

Proposed: Return each known billed buyer name, completed order count, then linked refund event count, by buyer ID.

Reason: Buyer names in an order/refund count report select an identity role; existing billing default is scoped to sales attribution metrics, not all customer counts. Reference SQL uses orders.billing_customer_id.

### sets.intersect

Before: Return distinct known customer IDs that are both completed buyers and subscribers of any status, ascending.

Proposed: Return distinct known customer IDs that are both completed billed buyers and subscribers of any status, ascending.

Reason: Completed buyers names an order participation role; customer entity synonyms alone do not select billing instead of shipping. Reference SQL uses orders.billing_customer_id.

### sets.except

Before: Return known completed buyer IDs who have no subscription of any status, ascending.

Proposed: Return known completed billed buyer IDs who have no subscription of any status, ascending.

Reason: Completed buyer participation is not an explicitly authored universal billing default. Reference SQL uses orders.billing_customer_id.

## Already specified roles and other semantics

existence.completed_buyers and absence.completed_buyers explicitly say completed billed order; no wording change is justified. sets.union/union_all explicitly say billed buyer IDs. roles.billing/both/same/different/unmatched_billing identify billed/payer and recipient roles, including known-identity and missing-record populations where requested. Shipping FR and shipping totals explicitly say recipient. Historical questions specify payer, even where a later projection says buyer name, so current vs historical payer linkage is clear. Buyer directories are named views of customers and do not join orders; directory buyer vocabulary does not introduce billing/shipping selection.

Membership cases join subscription.customer_id to its subscriber; there is only one authored membership customer role, so they do not need a billing/shipping qualifier. Customer-only projections and note/VIP metrics likewise have no order-role ambiguity. The intentionally generic clarify.customer_role question remains ambiguous by design and must not receive a billed qualifier or changed expected outcome. Healthy/top-customer companions retain their authored ambiguity. No new global default is proposed.

This role audit does not assert that all nonrole wording or feature failures are resolved. For example completed-order population in window.per_buyer is a separate potential fidelity review; it cannot justify a role-only edit or be silently bundled into these proposals.

## Exhaustive coverage table

Every case appears below. Classification refers to order customer-role selection, not overall semantic correctness.

| Case | Role audit disposition |
| --- | --- |
| projection.vip | Customer/directory population; no order-role choice |
| projection.empty | Customer/directory population; no order-role choice |
| projection.null | Customer/directory population; no order-role choice |
| roles.billing | Billing/payer explicit or explicitly scoped by request |
| roles.shipping | Recipient/shipping explicit |
| metrics.gross | No order customer-role selection requested |
| relationships.products | No order customer-role selection requested |
| absence.buyers | Billing/payer explicit or explicitly scoped by request |
| fanout.net | No order customer-role selection requested |
| history.boundary | Billing/payer explicit or explicitly scoped by request |
| projection.unicode | Customer/directory population; no order-role choice |
| projection.apostrophe | Customer/directory population; no order-role choice |
| projection.negation | Customer/directory population; no order-role choice |
| projection.boolean_scope | Customer/directory population; no order-role choice |
| projection.top | No order customer-role selection requested |
| projection.negative | No order customer-role selection requested |
| projection.empty_result | No order customer-role selection requested |
| projection.float | No order customer-role selection requested |
| projection.width | Customer/directory population; no order-role choice |
| concept.completed | No order customer-role selection requested |
| concept.active | Single subscriber customer role; no order-role choice |
| concept.active_before | No order customer-role selection requested |
| concept.buyers | Customer/directory population; no order-role choice |
| concept.nested | Customer/directory population; no order-role choice |
| concept.purchases | Billing/payer explicit or explicitly scoped by request |
| concept.gross_by_region | Billing/payer explicit or explicitly scoped by request |
| concept.zero_active | Single subscriber customer role; no order-role choice |
| roles.both | Billing/payer explicit or explicitly scoped by request |
| roles.different | Billing/payer explicit or explicitly scoped by request |
| roles.same | Billing/payer explicit or explicitly scoped by request |
| roles.unmatched_billing | Billing/payer explicit or explicitly scoped by request |
| roles.shipping_fr | Recipient/shipping explicit |
| relationships.product_filter | No order customer-role selection requested |
| relationships.refund_composite | No order customer-role selection requested |
| relationships.subscribers | Single subscriber customer role; no order-role choice |
| relationships.calendar | No order customer-role selection requested |
| relationships.all_customers | Billing/payer explicit or explicitly scoped by request |
| relationships.left_scope | Billing/payer explicit or explicitly scoped by request |
| relationships.shipping_totals | Recipient/shipping explicit |
| existence.completed_buyers | Billing/payer explicit or explicitly scoped by request |
| absence.completed_buyers | Billing/payer explicit or explicitly scoped by request |
| existence.sold_products | No order customer-role selection requested |
| absence.unsold_products | No order customer-role selection requested |
| existence.refunded_clients | Billing/payer explicit or explicitly scoped by request |
| absence.no_subscriptions | Single subscriber customer role; no order-role choice |
| existence.only_ch_shipping | Billing/payer explicit or explicitly scoped by request |
| metrics.order_count | No order customer-role selection requested |
| metrics.distinct_buyers | Billing/payer explicit or explicitly scoped by request |
| metrics.customer_totals | Billing/payer explicit or explicitly scoped by request |
| metrics.customer_count_zero | Billing/payer explicit or explicitly scoped by request |
| metrics.minmax | No order customer-role selection requested |
| metrics.average | No order customer-role selection requested |
| metrics.empty_sum | No order customer-role selection requested |
| metrics.empty_count | No order customer-role selection requested |
| metrics.note_counts | Customer/directory population; no order-role choice |
| metrics.status | No order customer-role selection requested |
| metrics.having | Billing/payer explicit or explicitly scoped by request |
| metrics.refund_sum | No order customer-role selection requested |
| metrics.product_units | No order customer-role selection requested |
| metrics.weight_sum | No order customer-role selection requested |
| fanout.order_item | No order customer-role selection requested |
| fanout.per_buyer_net | Billing/payer explicit or explicitly scoped by request |
| fanout.line_refunds | No order customer-role selection requested |
| fanout.orders_subscriptions | Billing/payer explicit or explicitly scoped by request |
| fanout.product_net | No order customer-role selection requested |
| fanout.weighted_price | No order customer-role selection requested |
| fanout.order_refund_count | Independent review candidate: unqualified buyer participation uses billing SQL |
| fanout.refund_fraction | Billing/payer explicit or explicitly scoped by request |
| fanout.line_count | No order customer-role selection requested |
| calendar.leap | Billing/payer explicit or explicitly scoped by request |
| calendar.previous_month | Billing/payer explicit or explicitly scoped by request |
| calendar.fiscal | No order customer-role selection requested |
| calendar.business_day | No order customer-role selection requested |
| calendar.monthly | No order customer-role selection requested |
| calendar.missing_month | No order customer-role selection requested |
| calendar.spring_wall | No order customer-role selection requested |
| calendar.autumn_instant | No order customer-role selection requested |
| calendar.fraction | No order customer-role selection requested |
| calendar.year_boundary | No order customer-role selection requested |
| history.missing | Billing/payer explicit or explicitly scoped by request |
| history.current_vs_then | Billing/payer explicit or explicitly scoped by request |
| history.all | Billing/payer explicit or explicitly scoped by request |
| history.region_sales | Billing/payer explicit or explicitly scoped by request |
| history.changed | Billing/payer explicit or explicitly scoped by request |
| window.rank | No order customer-role selection requested |
| window.rank_gaps | No order customer-role selection requested |
| window.per_buyer | Billing/payer explicit or explicitly scoped by request |
| window.running | No order customer-role selection requested |
| window.region_running | Billing/payer explicit or explicitly scoped by request |
| window.lag | No order customer-role selection requested |
| window.top_per_buyer | Billing/payer explicit or explicitly scoped by request |
| window.final_limit | No order customer-role selection requested |
| sets.union_all | Billing/payer explicit or explicitly scoped by request |
| sets.union | Billing/payer explicit or explicitly scoped by request |
| sets.intersect | Independent review candidate: unqualified buyer participation uses billing SQL |
| sets.except | Independent review candidate: unqualified buyer participation uses billing SQL |
| calculation.line_allocation | No order customer-role selection requested |
| calculation.zero_ratio | No order customer-role selection requested |
| calculation.tax | No order customer-role selection requested |
| calculation.exchange | No order customer-role selection requested |
| clarify.customer_role | Intentional ambiguity companion; preserve |
| clarify.healthy | No order customer-role selection requested |
| clarify.currency | No order customer-role selection requested |
| clarify.period | No order customer-role selection requested |
| clarify.top_metric | No order customer-role selection requested |
| reject.denied_scope | No order customer-role selection requested |
| reject.invalid_rollup | No order customer-role selection requested |
| reject.duplicate_rate | No order customer-role selection requested |
| reject.overlap_history | No order customer-role selection requested |
| unsupported.payment | No order customer-role selection requested |
| unsupported.future_rate | No order customer-role selection requested |
| unsupported.residual | No order customer-role selection requested |
| error.invalid_cast | Customer/directory population; no order-role choice |
| error.division_zero | No order customer-role selection requested |
| error.overflow | No order customer-role selection requested |

Any approved future revision must preserve every SQL string, gold, tolerance, data file and companion expectation, record previous/effective wording and versioned artifact hashes, and regenerate independently. Parent authorization follows reviewer adjudication; this document does not authorize artifact mutation.
