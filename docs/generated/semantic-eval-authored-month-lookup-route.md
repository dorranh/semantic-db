# Existing checked lookup and composition route for authored calendar months

Read-only design audit; no artifact changes. Prefer exposing genuine existing calendar columns through authorized checked lookups before adding civil Date32 compute/presentation IR. Exact existing golds remain unchanged.

## Genuine reusable relationship authoring

Proposed orders_calendar equality: orders.order_date → business_calendar.calendar_date. Role name orders_calendar; source orders, target business_calendar; no inferred enforced cardinality. At query time checked Lookup must assert at most one target key match before grouping. The actual monthly SQL INNER JOIN omits missing mapped dates, so use missing exclude to preserve its population; do not substitute a stricter missing-date error without an authored task contract. The existing BusinessCalendar operation has stricter coverage semantics, which is a distinct contract. Source dates are physically Date32 and nonnull on both sides. The lookup value is existing month_key Utf8, not a computed or fabricated answer column.

Proposed calendar_same_month equality: business_calendar.month_key → business_calendar.month_key. This is a genuine same-calendar-period association, with many raw dates in either direction. Role name calendar_same_month; equality on existing Utf8 fields. Do not use it as a unique raw-row Lookup: target month keys intentionally repeat. Its use is alignment of independently grouped calendar-origin month populations; grouping proves one row per month on each side. Neither relationship grants arbitrary Gregorian/fiscal conversion or historical validity.

## Observed existing compiler route

RowOperation.Lookup with usage Group is accepted for aggregate queries; usage Project is rejected there. Binding records semantic_assert_single_v1 and the relationship definition/policies. Graph row-output extraction preserves a grouped Lookup key's origin as the actual right relation/value field, so orders mapped to month_key has origin (business_calendar, month_key), not (orders, order_date). Ordinary Group of calendar.month_key has that same origin. Compose validates complete grouped grain on both inputs, identical exact key types and the authored relationship's exact origin pair; calendar_same_month can therefore authorize those two origins without inventing lineage.

Suggested monthly plan: orders root → completed ConceptFilter → orders_calendar Lookup usage Group, field month_key, missing exclude → exact SUM(total_minor) or a compatible governed sum → order by month. Typed output remains Utf8/Int64. Preaggregation target-key guard prevents repeated calendar rows from multiplying order cents. No continuous spine or timestamp cast is needed.

Suggested missing_month graph: left root business_calendar grouped by month_key; right root orders filtered completed and checked calendar lookup grouped by month_key with exact cents SUM. Compose left-preserving on calendar_same_month; select the left month and right sum with MissingGroup.Zero. Retain the original key grain in graph projections and final ordering. Parent's recent graph Project/grain correction may matter; runtime validation is still required. This route represents DISTINCT authored months, preserves May zero, and does not synthesize June–September.

## Metric authorization remains an independent obligation

Current gross_sales_minor declares local dimensions status, billing_customer_id and order_date, but its authored lookup_dimensions whitelist does not include an orders_calendar/month_key entry. A governed metric proposal must explicitly add only the genuine compatible lookup dimension with missing exclude and the matching relationship. The completed concept establishes its required population. Do not infer permission because canonical fixture rows happen to match. Existing sum_rollup/state restrictions must be checked before changing any whitelist or dimensional contract. Alternatively a generic exact SUM(total_minor) is honest if existing capability publication permits that typed aggregate; do not silently bypass a governed-only policy. The sum field is real signed CHF cents and empty aggregate behavior remains null before requested sparse fill.

## Source audit and expected failure obligations

Every actual order/calendar match count is independently computed below. All orders currently map once; these fixtures do not distinguish missing or duplicate date behavior. Raw self-calendar month multiplicities demonstrate that same-month is not a unique lookup. Generic checked-lookup tests need a duplicate date causing same-query key-count rejection and a missing date excluded under missing exclude. Explicit null keys never match. Unknown regions or historical valid_to nulls have no bearing on this date equality. An empty completed fact population gives no monthly groups and all authored month keys with requested zero in missing_month; empty calendar population gives no output. Credits preserve April -100; no absolute-value or unsigned amount conversion is authorized.

```json
{
  "order_date_matches": [
    {
      "order_id": 101,
      "order_date": "2023-12-31",
      "calendar_matches": 1
    },
    {
      "order_id": 102,
      "order_date": "2024-01-01",
      "calendar_matches": 1
    },
    {
      "order_id": 103,
      "order_date": "2024-02-28",
      "calendar_matches": 1
    },
    {
      "order_id": 104,
      "order_date": "2024-02-29",
      "calendar_matches": 1
    },
    {
      "order_id": 105,
      "order_date": "2024-03-01",
      "calendar_matches": 1
    },
    {
      "order_id": 106,
      "order_date": "2024-03-31",
      "calendar_matches": 1
    },
    {
      "order_id": 107,
      "order_date": "2024-03-31",
      "calendar_matches": 1
    },
    {
      "order_id": 108,
      "order_date": "2024-04-01",
      "calendar_matches": 1
    },
    {
      "order_id": 109,
      "order_date": "2024-10-27",
      "calendar_matches": 1
    },
    {
      "order_id": 110,
      "order_date": "2024-10-27",
      "calendar_matches": 1
    }
  ],
  "raw_calendar_rows_per_month": {
    "2023-12": 1,
    "2024-01": 1,
    "2024-02": 2,
    "2024-03": 3,
    "2024-04": 1,
    "2024-05": 1,
    "2024-10": 1
  }
}
```

## Readiness gates

Ask implementer to confirm current grouped Lookup guard placement, Utf8 exact Compose support, left-preserving zero output and generic SUM capability before authoring these relationships. Independently review the precise missing policy against original SQL and the real metric whitelist. Then offline validation and unchanged full SQL must pass, followed by focused monthly/missing_month Ask. This proposal is not a claim the current model already grants either relationship or that the compiler has already executed the proposed graph successfully.
