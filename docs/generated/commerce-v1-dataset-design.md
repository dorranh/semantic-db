# Commerce v1 dataset design

Version 1.0.0, authored 2026-10-04. This standalone artifact contains 100 paired
answerable cases and 15 companion outcomes. Both SQL and typed Ask must independently
match the same typed gold. There are no expected-failure labels or feature waivers.

## Business and source placement

The Swiss shop has four production CSV dimensions (customers, products,
exchange_rates, business_calendar), five PostgreSQL facts/history relations
(orders, order_items, refunds, subscriptions, customer_region_history), and two
quarantined CSV diagnostic lookups for duplicate rates and overlapping history.
The diagnostic relations are not joined into ordinary business facts.

Canonical records are in data/canonical.json. schemas.json records every physical
column type, data-domain nullability, declared key and row count. The separate
physical_nullable contract follows explicit CSV non_nullable_columns enforced
by the CSV reader, and PostgreSQL DDL for fact fields. Columns permitting null
remain physically nullable; canonical non-null domain checks remain enforced. CSV null is literal
`\N`; empty strings remain known empty text. Signed 16/32/64-bit integers,
Float32/Float64 measurements, exact decimals, Boolean, Unicode/apostrophes, dates,
time-of-day, local timestamps and fractional UTC instants have selected cases.
Currency is never compared using floating tolerance.

Orders are one row per order, items per (order_id,line_no), refunds per refund.
Order 101 has two lines and two refunds on line 1: joining all facts before summing
changes results. Completed gross sales are 11900 CHF cents; linked refunds are
1600; net sales are 10300. Negative order 108 is a completed credit adjustment.
Billing and shipping relationships differ. Order 101 bills Ada and ships Béatrice;
105 has unknown recipient; 109 has unmatched payer 99 and product 999.

Historical intervals are half open, [valid_from,valid_to), with null upper bound
open ended. Ada changes DE to CH at January 1; Chen lacks history before March 1.
Missing history remains unknown, never replaced with current customer region.
No ordinary equality relationship falsely promises to implement temporal lookup.
The diagnostic history intentionally has two matching intervals at January 1.

UTC placed_at contains instants; local_placed_at contains Zurich wall time.
March orders 106/107 are local 01:30/03:30, but UTC instants one hour apart.
October orders 109/110 share local 02:30 and have distinct UTC instants. Fiscal
years begin April 1 and use the year containing that start. Quarters are
Apr-Jun 1, Jul-Sep 2, Oct-Dec 3, Jan-Mar 4. The authored May calendar entry has
no sales and tests zero filling; unlisted months are not invented.

The fixed host context is 2024-04-01T00:00:00Z, Europe/Zurich. Active memberships
require active status, start <= date, and date < end where an end is present.
Membership 2 ends exactly April 1; 5 starts April 1; paused 3 stays excluded;
free membership 4 remains active. Customer 6 has no orders or subscriptions.

## Governed semantics

Core Ossie defines keys, descriptions, synonyms, role relationships and a composite
refund-to-item key. Supported SEMANTIC_DB extensions declare identities, CHF-cent
units, Zurich Gregorian date context, seven metrics, five concepts and two nested
buyer directory views. Counts have zero empty behavior; sums have null behavior.
Declared identities do not claim provider-enforced uniqueness for CSV.

Precise additional governed rules are serialized as version 1 JSON text inside
supported AI instructions. They define active-at-date subscriptions, independent
net sales aggregation, weighted prices, ratio rollups, as-of history, fiscal
mapping, conversion and allocation. Their dedicated structured executable profiles
are product gaps where the existing importer does not represent those contracts.
The runner must not inject them or silently reduce the model. Reference SQL
expands the rules explicitly, so SQL correctness remains separately testable.

Weighted unit prices use signed sum(line_minor)/sum(quantity), null for zero
quantity, and half-away rounding to six fractional cent digits when requested.
Average order cents are exact decimal 1487.50. Dimensionless refund fractions have
explicit Float64 tolerance; monetary amounts have exact integer/decimal contracts.
Conversion multiplies exact units by the exact dated rate and rounds half away
from zero to two CHF decimals. EUR rate 0.950050 yields both 95.01 and -95.01 for
positive/negative 100 units. Missing rates are unknown; duplicate governed lookups
are rejected. The selected proportional allocation divides net order cents after all refunds
using original signed line weights. Order 101 net 2000 allocates 800/1200, which differs
from raw line cents 1000/1500. No broader residual-cent policy is invented.

## Coverage and oracle

| Primary group | Paired cases |
| --- | ---: |
| Projection/filter/sort/limit | 12 |
| Concepts/synonyms/views | 8 |
| Relationships/roles | 15 |
| Existence/absence | 8 |
| Aggregation/governed metrics | 15 |
| Multiple facts/fan-out | 10 |
| Calendar/temporal | 10 |
| Historical relationships | 6 |
| Windows/ranking/running frames | 8 |
| Sets/conversion/calculation | 8 |

53 cases genuinely join or correlate CSV dimensions and PostgreSQL facts.
Companions comprise five clarifications, four rejections, three unavailable-data
outcomes, and three execution errors. SQL is retained on the execution-error
companions; only SQL plus result expectation defines paired cases.

Gold construction uses Python's independent SQLite relational engine over
canonical records, never Semantic DB output. SQL date literals have equivalent
SQLite oracle predicates. Exact financial division/conversion uses Python Decimal
and explicit rounding; gold integer/decimal values are lossless strings. Nulls
are JSON null, empty results retain schema, and ordered duplicates retain their
multiplicity. Each case names distinguishing records and its business policy.
The authoring script is not invoked by acceptance setup; it is an auditable offline
oracle source. Any substantive change requires renewed independent review.

## Reproduction and integrity

Compose pins PostgreSQL 16.6-bookworm, waits on a readiness healthcheck, and publishes
a random loopback host port. A one-shot bootstrap verifies checked-in data SHA-256
hashes, then transactionally recreates the dedicated commerce schema and COPYs
canonical CSV exports. Repeated setup cannot append duplicates. Runtime connection
URL uses manifest endpoint binding EVAL_DATABASE_URL; no credentials need logging.
Fixture queries check all eleven relation row counts. Canonical row counts and
all production declared-key tuples were independently checked at authoring time.

Core Ossie was validated against the repository's vendored JSON schema. Cargo,
Docker-backed execution, typed model compilation and live Ask validation belong
to the parent orchestrator; artifact correctness is not a claim that release
acceptance has passed. Final reviewed digests are recorded in the separate
review document by the independent reviewer.

The catalog fidelity revision after the first live evaluation adds explicit reverse billing/shipping/member/item/refund roles, the authoritative kilogram unit for measured product mass, and an exact FR-to-France region concept. Reverse traversals declare equality keys and possible many-row/missing populations without asserting target uniqueness. Active customer reporting explicitly starts with the complete customer dimension, retaining customers with no active matching membership when zero filling is requested. The request clock is interpreted in Europe/Zurich for business dates. This revision changes catalog authority only; all canonical rows, questions, SQL and gold results remain unchanged. Further exact allocation/currency profiles require compatible structured importer contracts and real declared source fields; no harness-injected catalog facts substitute for them.

Executable-profile contracts and remaining source requirements:

- `business_calendar`: existing exact `BusinessCalendarRule` maps orders.order_date (Date32) to business_calendar.calendar_date, fiscal_year(Int32), fiscal_quarter(Int16) and business_day(Boolean), timezone Europe/Zurich, with mapping_revision pinned to the actual CSV SHA256. UTC-instant mapping is a separate authored rule using placed_at; local wall timestamps do not qualify as UTC instants.
- Active membership is a parameterized local concept: status equals active, start_date <= typed `as_of_date`, and (end_date is null or end_date > typed `as_of_date`). The catalog now publishes this compare_parameter predicate; execution requires an explicit prepared date binding. Hardcoding a specific test date would change its meaning.
- Published mean completed order amount is SUM(total_minor) plus version2 sum_count_average state, result Decimal128(38,18), empty null, source order grain. Signed weighted unit price is SUM(line_minor)/SUM(quantity), zero denominator null. The existing nonnegative weighted_average state multiplies value by weight and rejects negative quantities, so it cannot honestly represent this signed ratio. A generic signed ratio contract with explicit rounding remains required; original quantity stays Int32 for width coverage.
- Allocation is over real net-order source amounts, complete expected line membership and weight totals, with original signed line amounts as eligible weights for order101. The published contract requires declared source expectation fields and exact Int64 bridge target dimensions; these are absent from the current base tables. A genuine bootstrap-derived source view must calculate them under the database source, retaining original facts. No case-specific harness facts may supply them.
- Dated currency conversion requires authored rational rate numerator/denominator, source/target currency and a half-open validity interval. The current rate CSV has decimal rate plus one effective date, so the full published currency-rate rule cannot honestly be bound without a real source enrichment or derived rate relation. Existing rational scalar conversion requires an Integer input; measured Float32 kilogram mass is correctly annotated with unit kg but cannot use that Integer conversion profile.

These are representation requirements discovered by checking actual catalog publication validation, not acceptance exemptions. Every existing result-required case and independently reviewed gold remains required.

Reviewed-profile candidate freeze model SHA256 `1535572968edb9b6368408c9431328cb1a28f943748b6e20eac3e9881a1ecc54`; questions/SQL/golds remain at cases SHA256 `eaedce47fc4a69759529fd3737341c80df2e3e2d7a125a305f4117ecb9183f47`. The model publishes completed MIN/MAX row filters, exact mean state, role lookup whitelists, parameterized active membership, and separate Swiss calendar contracts for business dates and UTC instants. Product verification is owned by the parent orchestrator.

The role-vocabulary precision revision explicitly maps payer/paying customer/paying buyer/billed buyer to billing_customer and recipient/receiving customer/received to shipping_customer, including reverse-role synonyms. Clarification is required only when customer involvement is actually unspecified; explicit payer or recipient language already determines the role. This is general catalog meaning, independent of case answers. Questions, SQL and golds remain unchanged. Model SHA256 `946b48c5d07ad94268031f1776104f6041c6168cd160a9514933d8611e5be9fc`.

Verified product SQL evidence after role-vocabulary refinement: **103/103** SQL-bearing cases passed, including the three authored execution-error companions. Report `.semantic-eval/commerce-sql-roles/run-251955-18db5cb4c88a0f55/report.json` has complete/finalized/full_coverage true and no setup/cleanup/artifact errors; artifact digest `79bc641044762043ad84c5eee984cc299c7f55425753c505f028be46be993cf0`. This is full SQL-path evidence, not a claim of Ask completion. Readonly reproducibility through a regenerated temporary copy matched every frozen artifact byte-for-byte, preserving the payer/recipient vocabulary, all cases and golds.

Seven independently reviewed NL population errata make existing reference INNER JOIN contracts explicit: roles.both requires matching payer and recipient customer records; calendar.leap and previous_month require matching billed customer records; history.current_vs_then requires matching current customer identity while retaining missing history; window.region_running requires completed orders with matching billed records; product_net and weighted_price require matching catalog product records. Matching identity is distinct from requiring a nonnull label: a matched dimension record with a null name/region still belongs. No global missing-identity default was invented. Explicit unknown-retention questions and clarification companions remain unchanged. Only seven question strings changed; SQL, all gold files, tolerances and feature coverage are byte-identical. Cases freeze SHA256 `ae766425740387c6f69659956964a72f8e479a212a534e9e72aba02f67b15ce6`.

The preceding focused Luna Ask run was complete/finalized for its filtered five-case scope and passed 4/5 without provider errors: `.semantic-eval/commerce-span-contracts/run-253650-18db5cc95d168111/report.json`. MIN/MAX, billing roles, active memberships and active_before passed. active membership used two recorded unique exact span normalizations; the other four requests used none. previous_month correctly asked for an unspecified missing-payer population, motivating the reviewed NL erratum above. The original failure remains preserved; the filtered run does not establish full 115-case Ask coverage.


## Terminal commerce shared-context evidence

The frozen full run passed SQL 103/103 and Ask 51/115 (154/218 total), with no provider errors and null setup/cleanup/artifact errors. Finalized/full coverage are true; complete is false due to five unresolved expansion-limit outcomes. All 64 failed Ask cases and full diagnostics/differences appear in [the terminal commerce failure ledger](commerce-v1-shared-context-failure-ledger.md), alongside separate integer-width focused evidence (2/4, SQL only passing). The 122 provider calls have input median 33015 and maximum 33927. Historical baselines remain preserved; these results do not claim full Ask acceptance.


## Commerce v1.0.1 reverse product relationship

Added only the authentic product_items equality role from products.product_id to order_items.product_id. Multiple and absent matches remain possible; no enforced uniqueness/cardinality or implemented nested-concept capability is claimed. The author generator reproduces this edge and manifest version 1.0.1. All 145 other artifact files remain byte-identical, including questions, SQL, golds, tolerances, schemas and canonical data. Model SHA-256 is `ff53d7c83b5ce7775de918da9fa66c074da1a4083732a30e870a14bed7098e25`; whole-artifact digest is `d435dffd97a15fd0994de19ffc595d56c571494404a6dee65704f79626d07a7a`. Full before/after file hashes are recorded in commerce-v1-product-items-revision.json and commerce-v1-product-items-baseline-hashes.json. Earlier runtime reports retain their original digests. Independent review passed and regenerated all 148 artifact files byte-identically. Parent offline validation passed. Fresh full SQL passed 103/103 with complete/finalized true and no setup/cleanup/artifact errors: `.semantic-eval/commerce-product-items-sql/run-382985-18db6199c5bd232c/report.json`, SHA-256 `7d2df4776e0d10cdd24bbdbae3e80d7bc543a81df1e755d0b9e717ede46a3a27`. This is SQL regression evidence, not a claim of nested-related Ask acceptance. The bundle remains frozen.


## Commerce v1.0.2 authored calendar lookup route

Added genuine orders_calendar date equality and calendar_same_month self-month equality, with honest multiple/missing semantics and no inferred cardinality. The gross_sales_minor governed metric now permits only the added month_key lookup under missing exclude, preserving original inner-join population. The implemented existing checked lookup/composition contracts were confirmed read-only; runtime success is still pending. Generator and manifest version changed to 1.0.2; all 145 other files remain byte-identical, including questions, SQL, golds, tolerances and data. Artifact digest `06471654721c975d5f139d5f6988aca3a22e50df470c1220ea00c7c0c55f5f72`; detailed hashes in commerce-v1-calendar-revision.json and calendar-baseline-hashes.json. Bundle refrozen for independent review and parent validation. No Date32 computation or new physical column was introduced.


## Commerce v1.0.3 visible month ordering clarification

calendar.missing_month now explicitly ends “ordered by month ascending,” clarifying the existing ordered comparison/reference SQL. Previous and effective wording are preserved in question-errata.json. No other question or any SQL/gold/tolerance/comparison/data/model changed; all 145 other existing files remain byte-identical. Generator and manifest reproduce v1.0.3. Artifact digest `da95e740c30ebcc8325417ef8682f633809a0b3f6e861a5345f3435c6aff4c7d`; detailed evidence in commerce-v1-month-order-revision.json and month-order-baseline-hashes.json. Refrozen for independent review and parent validation.


Fresh v1.0.3 SQL verification passed 103/103; focused pipeline 38 SQL+Ask passed 13/14, with both month queries passing and concept.zero_active retaining an extra client_id. Exact report paths/hashes, artifact identity and filtered coverage limits are recorded in [the pipeline 38 receipt](commerce-v1-pipeline38-verification.md). No full Ask success is claimed.


## Commerce v1.0.5 genuine row-wise FX profiles

The separately reviewed author revision adds exact_decimal_rate profiles over actual production and diagnostic Decimal rate relations, without synthetic source fields or answer constants. Only generator/model/manifest changed; all 146 other artifact files are identical. Detailed objects, hashes and boundaries are recorded in [the FX author receipt](commerce-v1-fx-profile-revision.md). Negative oracle migration and exact one-match execution remain separate. Bundle refrozen for review/parent validation.
