# Documentation-only commerce v1.0.4 seven-question revision plan

Both datasets remain frozen during parent verification. This plan does not edit a generator or artifact. The accompanying commerce-v1-seven-question-revision-plan.json records exactly seven before/effective questions and baseline/reference/gold hashes. Four reviewed changes comprise three billed-role precision insertions and one real completed-order population correction; three date edits are optional precision, not necessarily original ambiguity or a capability workaround.


## fanout.order_refund_count

Before: Return each known buyer name, completed order count, then linked refund event count, by buyer ID.

Effective: Return each known billed buyer name, completed order count, then linked refund event count, by buyer ID.

Reason: Role precision only: counts/refunds use billed buyer attribution, without a newly invented global customer default.

## window.per_buyer

Before: Return payer name, order ID, then newest-first row number within each known payer, ordered by payer ID then row number.

Effective: For completed orders, return payer name, order ID, then newest-first row number within each known payer, ordered by payer ID then row number.

Reason: Population contradiction correction: prior wording lacks completed restriction while reference SQL filters status completed; pending/cancelled rows otherwise change both row population and ranks.

## sets.intersect

Before: Return distinct known customer IDs that are both completed buyers and subscribers of any status, ascending.

Effective: Return distinct known customer IDs that are both completed billed buyers and subscribers of any status, ascending.

Reason: Role precision: ordinary buyer usually means payer, but explicit billed binds the existing intended role under current catalog vocabulary. Model clarification is not proof the original was necessarily ambiguous.

## sets.except

Before: Return known completed buyer IDs who have no subscription of any status, ascending.

Effective: Return known completed billed buyer IDs who have no subscription of any status, ascending.

Reason: Role precision only: make the existing billing participation role explicit, without changing known customer/subscription populations.

## fanout.orders_subscriptions

Before: For every customer return name, completed billed sales cents (zero if none), then active monthly cents on April 1, by ID.

Effective: For every customer return name, completed billed sales cents (zero if none), then active monthly cents on April 1, 2024, by ID.

Reason: Optional explicit-year precision consistent with independently authored reference date; current pinned clock may reasonably resolve the previous partial date.

## calculation.exchange

Before: For each leap-day exchange rate, return currency, exact CHF value of 100 foreign units, then value of negative 100 units; round half away from zero to two decimals and order by currency.

Effective: For each exchange rate on February 29, 2024, return currency, exact CHF value of 100 foreign units, then value of negative 100 units; round half away from zero to two decimals and order by currency.

Reason: Optional explicit-date precision; pinned 2024 clock permits contextual reading, so this is not a proven original contradiction or an IR workaround.

## reject.duplicate_rate

Before: Convert 100 EUR to CHF on leap day using the quarantined exchange_rates_duplicate lookup.

Effective: Convert 100 EUR to CHF on February 29, 2024 using the quarantined exchange_rates_duplicate lookup.

Reason: Optional explicit-date precision only; its coarse negative expectation is untouched pending separate typed-runtime migration.

## Minimal author workflow after release

Capture the existing v1.0.3 full file hashes and artifact identity, preserving historical full and incomplete run reports independently. Change only seven generator question strings and the v1.0.4 manifest version; append seven visible erratum entries to question-errata.json without overwriting the existing calendar.missing_month entry. Preserve previous/effective text and independent reasons. Negative duplicate-rate expected metadata remains unchanged.

Regenerate an isolated copy first. Require exactly seven question-field differences and no other case-field changes. All SQL, gold files, tolerances, comparisons, canonical data, schemas, source configuration and model bytes must remain unchanged. Expected artifact allowlist is generator, cases, manifest and existing question-errata only; investigate any additional drift. Publish pre/post hashes and independent actual-delta/reproducibility approval before parent validates/commits. No old run score is retroactively replaced.

## Separate genuine row-wise FX profile authoring

The source now contains an exact_decimal_rate Ossie importer, pending parent tests and final support confirmation. A later separately reviewable author patch can publish the genuine exchange_rates profile: actual currency/date/chf_per_unit fields, constant CHF target, positive-only policy and unavailable-null policy. No numeric rate values, literal amounts, dates, expected results or case IDs enter the model contract. Host request literals remain typed bindings. Version/provenance and generator changes must be separate from these seven NL edits. The row-wise operation does not supply exact one-match lookup guards; neither reject.duplicate_rate nor unsupported.future_rate gold may migrate in that batch.

## Current concept alias support audit

Read-only current executable_profile.rs ConceptContract accepts kind/dataset/name/description/predicate with deny_unknown_fields; it has no aliases field. Published ConceptDefinition still hardcodes aliases to an empty vector. Therefore do not author booked aliases yet or claim they publish. The independently reviewed local booked-orders/purchases/sales definition needs a genuine bounded generic importer alias extension or a supported descriptive-instructions statement, explicitly distinguished from executable aliases. This is separate from v1.0.4 NL and row-wise FX work. No source edits or Cargo were performed by the analyst.
