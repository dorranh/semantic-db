# Authored advanced Ossie contracts

The `SEMANTIC_DB` metric extension retains its required `kind: metric`,
`dataset`, `source_grain`, `dimensions`, `unit`, and `empty` fields. It additionally
accepts optional `lookup_dimensions`, `sum_rollup_dimensions`, `state`,
`row_filters`, and `result_type`. These use the corresponding catalog structures;
unknown fields remain errors. Omitting `result_type` preserves the prior Int64
contract. Explicit result types use Arrow JSON, such as `"Int64"` or
`{"Decimal128":[38,18]}`, and must agree with the Ossie logical `datatype`.

`lookup_dimensions` lists exact `{relationship, field, missing}` permissions.
It grants no permission for other roles, fields, or missing-match policies.
`row_filters` contains typed `{field, operator, value}` predicates applied only
to that metric. `state` uses metric-state version 2. An exact sum/count mean is
authored as `SUM(field)` with `sum_count_average` state and Decimal128(38,18)
result. Existing weighted state requires Int64 values and nonnegative Int64
weights; it cannot represent a signed quantity ratio. State merge dimensions
must lie inside the metric's authored dimension whitelist.

The expression parser accepts one field-only SUM, COUNT, AVG, MIN, or MAX.
Arbitrary expressions, casts inside aggregates, DISTINCT, SQL filters, windows,
and trailing statements remain outside this importer profile. Physical types
and executable result/state compatibility are validated at catalog publication
and compilation; logical declarations do not cast source fields automatically.

Model-level business calendars use this wrapper:

```json
{"kind":"business_calendar","dataset":"events","name":"fiscal","rule":{
  "version":1,"id":"calendars/events-fiscal",
  "source_relation":"events","calendar_relation":"days",
  "source_date_field":"day","calendar_date_field":"day",
  "fiscal_year_field":"year","fiscal_period_field":"period",
  "business_day_field":"business","source_basis":"date32",
  "timezone":"Europe/Zurich","mapping_revision":"authored-data-revision",
  "source_refs":[]
}}
```

The source basis is Date32 or `utc_instant_micros`. Calendar dates, fiscal year,
period, and business-day fields require non-null Date32, Int32, Int16, and Boolean
provider fields respectively. A mapping revision and valid IANA timezone are
mandatory. The importer replaces supplied source references with the actual
normalized document origins. Execution retains the compiler's mapping coverage
and uniqueness checks.

Authored concepts may contain `compare_parameter` leaves with declared local
fields and bounded parameter names. Typed `concept_filter.arguments` supply the
values; missing, extra, or incompatible arguments remain compiler failures.
The importer does not derive a request clock or invent a parameter value.

Float fields may carry named measurement units, preserved for physical Float32
or Float64 providers. Currency tags on Float fields remain rejected. The existing
exact rational conversion profile still requires signed integer source values.
