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
