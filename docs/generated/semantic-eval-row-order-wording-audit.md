# Presentation wording audit: BIRD 978 and sparse calendar months

The BIRD audit remains read-only. Independently reviewed commerce clarification is now authored in v1.0.3 under parent authorization. Current BIRD v1.0.3 case 978 has comparison.ordered=false. Its expected file does not independently require ordering. Although reference SQL orders location,lat,lng, reversed venue rows do not explain an acceptance failure under bag comparison. Effective NL already explicitly requires total count first, then location/latitude/longitude; a result with count last remains a real column-order failure. No 978 row-sort erratum is required to make the current comparison fair. If a future task intentionally requires ordered rows, that is a separate visible presentation change rather than an explanation of the observed current failure.

Commerce v1.0.2 calendar.missing_month has comparison.ordered=true and reference ORDER BY m.month_key ascending. Its current question ends “by month,” which can describe grouping rather than ascending row presentation. Published minimal commerce effective NL:

> Return each distinct authored calendar month then completed sales cents, zero for no completed sales, ordered by month ascending.

This states the existing comparison/reference presentation without giving any expected values or altering sparse authored population, cents, zero filling or columns. Independent reviewer approval and parent authorization are required before editing. Preserve previous wording in visible erratum provenance; generator, manifest version and hash evidence must record any approved revision. SQL, gold, data, tolerances and comparison settings remain unchanged.

Commerce v1.0.3 digest `da95e740c30ebcc8325417ef8682f633809a0b3f6e861a5345f3435c6aff4c7d`; previous wording remains in its question-errata.json. Bundle refrozen. BIRD wording and comparison remain unchanged.
