# Read-only adjudication: booked orders and completed population

Frozen commerce artifact remains unchanged. This is separate from the four planned question clarifications and from compiler FX work.

## Current authored meaning and mismatch

concept.gross_by_region requests “current billed-client region then gross booked sales cents,” excluding unmatched billed clients. Its SQL filters orders.status='completed'. The gross_sales_minor metric is a sum over order rows and explicitly says to apply completed only when requested; it does not itself impose completed status. The completed concept description says “Orders booked as completed, including signed credit adjustments,” but its published definition has no explicit booked aliases. AI instructions define completed orders, without explicitly defining booked sales/orders as that local completed population.

Other authored questions use booked purchases: concept.completed explicitly adds “using the completed-order definition,” whereas concept.purchases asks booked purchase count without that extra phrase. These terms already recur in the business-author dataset; the reference intent is consistent, but only one request explicitly states the vocabulary equivalence. A model clarification is understandable given the incomplete formal vocabulary; it does not establish that ordinary “booked” always means completed across businesses.

## Preferred local author repair for independent review

Define, only within this commerce model, booked orders/booked purchases/booked sales as the orders completed concept: status completed, including signed credit adjustments. Add supported concept aliases for booked orders and booked purchases, and booked sales if the importer concept-alias contract genuinely supports that semantic label. Accompany aliases with a precise local definition so booked sales selects the completed order population but does not redefine the metric's arithmetic or add refund subtraction. Preserve gross_sales_minor as an additive order-grain total; net sales remains a separate after-refund meaning.

This is a truthful missing vocabulary contract if the business author adopts it explicitly, not a universal accounting rule or a per-case answer hint. It applies consistently to all existing booked questions and future requests without case-ID conditions. Do not treat bare gross sales or every order as completed solely because this alias exists; use the concept only when requested. Ensure alias resolution stays pinned to the orders relation and does not hijack subscriptions or arbitrary metrics.

## Alternative question-only repair

If the author does not wish to establish that local booked vocabulary, minimally state completed orders in concept.gross_by_region and independently audit concept.purchases for the same undefined population. That preserves the reference result but avoids a business-language contract. It is less complete than defining the already recurring intended term, and must remain a visible NL erratum rather than a silent reinterpretation. Neither option is authorized while the live bundle is frozen.

Independent reviewer should choose the local-domain definition versus question-only precision, verify compatible importer alias fields and check that gross sums, refund handling, signed credits and matching-client population stay unchanged. Any later author patch needs separate parent authorization, exact before/after hashes, independent regeneration and unchanged SQL/golds/tolerances/data. Existing live refusals stay recorded against the old artifact; no acceptance waiver is introduced.
