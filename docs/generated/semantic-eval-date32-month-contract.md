# Proposed Gregorian Date32 month grouping and separate fill contract

This is an unloaded general capability proposal. No production code, dataset artifact, model, case, SQL, gold or tolerance changes accompany it. Both artifact bundles remain frozen. The next bounded operation is civil Gregorian Date32 month bucketing; authored business-calendar population and historical as-of lookup are distinct obligations.

## Current restriction and smallest honest extension

RowOperation.CalendarGroup currently takes field, grain, timezone and alias, but compiler validation accepts UTC microsecond timestamps only. The engine semantic_utc_month_us_v1 returns the first UTC instant of the month, propagates nulls and rejects representability failures. CalendarFill currently accepts first-of-month UTC microsecond bounds, one observed month group and one non-distinct COUNT(*), zero fill only. It does not represent Date32 input or completed-sales SUM over a sparse authored month population.

Propose a physically stable civil Gregorian Date32 month profile: input Date32 days from 1970-01-01; output Date32 for the first civil date of that month. Preserve the input field's nullability. Month selection uses the actual Gregorian year/month of the civil date, never truncating day count by an average month length, casting through an instant, or applying a machine-local timezone. Use checked date conversion and reject unrepresentable calendar boundaries; the full Arrow Int32 day range must not silently overflow a narrower date library.

Do not reuse a required timezone string to pretend a date is an instant. Prefer an explicit civil_date32 basis in the generic month operation or a separate versioned bounded profile, subject to API review. Legacy UTC behavior stays unchanged. The Date32 profile accepts Gregorian calendar semantics only; authored fiscal periods remain BusinessCalendar mappings. Output formatting to YYYY-MM is a separate typed presentation operation and produces Utf8; the structural grouping key remains Date32. For an extended/BCE year, reject an unsupported YYYY-MM presentation range instead of silently emitting misleading four-digit text.

## Clock and timezone behavior

A stored Date32 order_date is already the authoritative civil date. Grouping it has no clock or timezone dependency. Europe/Zurich remains relevant when resolving a relative request into explicit Date32 filter bounds, or when converting a UTC placed_at through the authored Swiss calendar; it does not shift stored order_date. With the pinned 2024-04-01T00:00:00Z request clock, previous month is March 2024 in Zurich, with Date32 bounds 2024-03-01 inclusive and 2024-04-01 exclusive. Bind those host values exactly. Local wall timestamps and UTC instants remain separate physical inputs and cannot enter Date32 grouping via an implicit conversion.

History valid_from/valid_to are Date32 interval endpoints, with nullable valid_to. Bucketing those dates does not establish half-open historical matching, uniqueness, retention or overlap checks. None of history.boundary/missing/all/current_vs_then/region_sales/changed is repaired by month grouping alone.

## Existing acceptance cases and exact typed shapes

calendar.monthly is the direct target: existing gold is Utf8 month_key followed by Int64 completed sales cents. Its SQL joins business_calendar by exact order_date before grouping, so replacing that join with a civil month bucket requires an authored equivalent-representation contract and preserved population and uniqueness checks (coverage failure only where an existing governed BusinessCalendar operation requires it). Canonical rows currently satisfy month_key = YYYY-MM(calendar_date), independently checked below; observed equality alone is not permission to bypass business-calendar authorization or silently invent a field. A simpler faithful plan can expose the existing calendar month_key through the already-authored calendar mapping and group it; that is a separate catalog mapping-field extension from the Date32 month profile.

calendar.missing_month benefits only after a distinct authored month population plus SUM zero-fill is supported. Its Utf8/Int64 gold includes May but excludes June through September: continuous min/max fill would give the wrong population. calendar.fiscal and relationships.calendar remain fiscal mapping queries. calendar.leap, previous_month, year_boundary and fraction already use Date32 projection/filtering, not month grouping; they are regressions, not new month-profile coverage. BIRD 884/901 require extracting/comparing date components, not grouping; Date32 month buckets may be useful primitives but do not alone solve their complete requests.

Observed completed-month gold (Utf8, Int64; integers serialized losslessly):

```json
[
  [
    "2023-12",
    "2500"
  ],
  [
    "2024-01",
    "2000"
  ],
  [
    "2024-02",
    "3000"
  ],
  [
    "2024-03",
    "3500"
  ],
  [
    "2024-04",
    "-100"
  ],
  [
    "2024-10",
    "1000"
  ]
]
```

Authored-population zero-filled gold with the same types:

```json
[
  [
    "2023-12",
    "2500"
  ],
  [
    "2024-01",
    "2000"
  ],
  [
    "2024-02",
    "3000"
  ],
  [
    "2024-03",
    "3500"
  ],
  [
    "2024-04",
    "-100"
  ],
  [
    "2024-05",
    "0"
  ],
  [
    "2024-10",
    "1000"
  ]
]
```

## Separate population and fill contract

Civil observed grouping returns only observed month keys and aggregates; empty filtered input produces no grouped rows. It cannot invent a spine. An explicit bounded continuous fill may be introduced separately with Date32 month-start inclusive/exclusive bounds, unique month keys, maximum generated-month budget, and an explicitly selected exact aggregate output slot. Distinguish COUNT zero from SUM null-on-empty semantics; zero filling SUM requires a requested typed zero and does not change genuine zero or negative totals. No existing UTC COUNT-only profile should silently acquire different semantics.

For commerce's actual authored-population query, read DISTINCT business_calendar.month_key under the same pinned mapping revision and policy scope, then left-match the completed-sales aggregate by month and apply exact Int64 zero to absent groups. Preserve May with zero and April with -100. Validate mapped date uniqueness/coverage where required by the fact query. An empty authored population remains empty; an empty fact population retains all authored months with requested zeros. Under the existing governed BusinessCalendar operation, an absent mapped date is a mapping failure. Under an ordinary authored equality lookup with missing exclude, absent mapped dates are excluded, faithfully preserving the reference INNER JOIN population. Neither route may synthesize a Gregorian row. This spec does not claim current fill operations support that plan.

## Independent generic test requirements

| Input or condition | Required behavior |
| --- | --- |
| 2024-03-31 and 2024-03-01 | Both Date32 output 2024-03-01; aggregate once per input fact. |
| 2024-02-29 | Date32 2024-02-01; leap date retained. |
| 2023-12-31 / 2024-01-01 | Distinct December/January buckets across year boundary. |
| Epoch day -1 (1969-12-31) | Date32 1969-12-01; negative epoch arithmetic must use calendar dates. |
| Epoch day 0 | Date32 1970-01-01. |
| Null date | Null bucket; ordinary grouping retains a null group unless an explicit filter excludes it. Null bucket must not match a nonnull fill key. |
| No observed rows | No grouped rows; no inferred months. |
| Signed amounts including credits | Exact Int64 aggregation; retain negative result, with checked overflow. |
| Date outside supported checked calendar range | Explicit representability diagnostic, no panic/wraparound. |
| Timestamp(us), timestamp(us,UTC), Date64 or Utf8 date text | Reject Date32 profile without an explicit valid typed conversion. |
| Authored sparse month population | Fill precisely those keys, including no-sales May; do not add intervening months. |
| Duplicate or missing date mapping | Preserve existing same-query mapping guards; never conceal it with a computed bucket. |

Null and negative-epoch test rows are generic compiler/engine tests, not new commerce observations. Current commerce order_date and calendar_date are physically nonnull; history.valid_to is nullable. Existing fixtures do not cover Date32 extreme bounds. Keep those coverage limitations explicit.

## Review and provenance

Implementer must choose the exact public grammar/version and maintain request evidence, output field type, grouping grain and definition provenance. Reviewer should independently verify the Date32/instant distinction, sparse fill population and gold shapes before authoring any new mapping contract. Parent owns Cargo and live evaluation. No capability is considered accepted until meaningful engine/compiler checks and unchanged full SQL plus fresh Ask evidence pass.

Canonical JSON SHA-256: `44c4481f6280b0dfe14ba67d3a9d9a410f6402ad71e8257bed236489a2b9cb50`. All authored calendar rows were independently checked for the proposed Gregorian YYYY-MM correspondence; this checks fixture truth, not an automatically inferred runtime contract.
