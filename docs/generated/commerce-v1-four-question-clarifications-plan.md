# Planned four visible commerce question clarifications

Documentation-only proposal. All four intended deltas have independent conceptual review; actual artifact authoring remains unauthorized while the frozen full Ask run is active. This plan preserves the previous and proposed wording, reference SQL/gold hashes and current manifest/case hashes in commerce-v1-four-question-clarifications-plan.json. No fixture answers are added to the question text.

The three billed-word additions are precision improvements to the existing intended role. Ordinary buyer often naturally means payer; a model clarification does not prove those original questions were necessarily ambiguous. No universal billing default is introduced. The window population correction is a distinct real NL/SQL contradiction: omitting completed orders would admit pending/cancelled orders and change rank assignments. It is not a role-ambiguity edit.


## fanout.order_refund_count

Before: Return each known buyer name, completed order count, then linked refund event count, by buyer ID.

Planned effective: Return each known billed buyer name, completed order count, then linked refund event count, by buyer ID.

Reason: Role precision only: counts/refunds use billed buyer attribution, without a newly invented global customer default.

## window.per_buyer

Before: Return payer name, order ID, then newest-first row number within each known payer, ordered by payer ID then row number.

Planned effective: For completed orders, return payer name, order ID, then newest-first row number within each known payer, ordered by payer ID then row number.

Reason: Population contradiction correction: prior wording lacks completed restriction while reference SQL filters status completed; pending/cancelled rows otherwise change both row population and ranks.

## sets.intersect

Before: Return distinct known customer IDs that are both completed buyers and subscribers of any status, ascending.

Planned effective: Return distinct known customer IDs that are both completed billed buyers and subscribers of any status, ascending.

Reason: Role precision: ordinary buyer usually means payer, but explicit billed binds the existing intended role under current catalog vocabulary. Model clarification is not proof the original was necessarily ambiguous.

## sets.except

Before: Return known completed buyer IDs who have no subscription of any status, ascending.

Planned effective: Return known completed billed buyer IDs who have no subscription of any status, ascending.

Reason: Role precision only: make the existing billing participation role explicit, without changing known customer/subscription populations.

## Authoring and review gates

After parent confirms the full old run is terminal, preserve that complete report and artifact digest independently. Snapshot every artifact file and retain the immutable before wording in a visible question-errata history; do not overwrite the existing month-order erratum. Make only these four question replacements in generator/cases, a reviewed dataset version increment and the required erratum/provenance metadata. No SQL, gold, tolerance, canonical data, schemas, model, comparison settings, scope or companion expectation changes.

Regenerate in an isolated copy and prove exactly four question fields differ. Compare every SQL string and expected file byte, all comparison objects, numeric tolerances and source-data bytes. Record pre/post generator/cases/manifest and whole-artifact hashes, alongside the unchanged file allowlist. Reviewer checks actual four deltas and independent reproducibility before parent commits or runs the new artifact. Historical full-run failures remain visible; a subsequent corrected-question run cannot retroactively replace the old run or its score. Compiler row-wise FX implementation remains separate and grants no permission to edit this frozen dataset.
