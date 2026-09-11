# Wells example and Ossie mapping

[wells.ossie.yaml](wells.ossie.yaml) describes the seven columns in
[wells.csv](wells.csv), using the Ossie `0.2.0.dev0` core schema pinned at
[`28365cd638f3833765c5b940ada5b8cbc65f1c42`](https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/core-spec/ossie-schema.json).
It is an authored semantic model for this synthetic fixture. The CLI still loads
the CSV; it does not yet load Ossie files.

## Source and row contract

The proposed application binding is:

| Property | Value |
| --- | --- |
| Model | `geospatial_wells` |
| SQL dataset | `wells` |
| Ossie source reference | `fixtures.geospatial.wells` |
| Local backend binding | `examples/geospatial/wells.csv` |
| Grain | One row per well |
| Declared primary key | `well_id` |

The source reference is our binding convention. It does not encode a CSV, REST,
or database connector. A future backend could bind it to a ClickHouse table or
API-backed provider that satisfies the same row contract.

| Fields | Ossie logical type | Meaning |
| --- | --- | --- |
| `well_id`, `well_name` | `String` | Identifier and display name |
| `basin` | `String` | Basin category, including `North Basin` |
| `latitude_deg`, `longitude_deg` | `Float` | Coordinates in degrees; CRS and accuracy unspecified |
| `total_depth_m` | `Float` | Total depth in metres; depth measurement convention unspecified |
| `status` | `String` | Source status label, including `active` |

Each field expression is an identity reference to the corresponding CSV column,
in CSV header order. Logical `Float` does not prescribe Arrow width; the backend
must resolve physical types and nullability. The five fixture IDs are nonempty
and unique, but the primary-key declaration does not make the engine enforce
uniqueness on future data.

## Meaning of the existing query

The model documents these interpretations without creating extra columns:

| Request fragment | SQL predicate |
| --- | --- |
| Active wells | `status = 'active'` |
| In North Basin | `basin = 'North Basin'` |
| At least 2500 metres deep | `total_depth_m >= 2500` |

The complete query remains in [query.sql](query.sql). From the repository root:

```sh
cargo run -p semantic-cli -- --csv wells=examples/geospatial/wells.csv \
  --file examples/geospatial/query.sql
```

Expected result:

| well_id | well_name | total_depth_m |
| --- | --- | --- |
| W-001 | Juniper-1 | 3200.0 |
| W-004 | Birch-4 | 2750.0 |

“Deep” alone still needs a cutoff. The model's example request explicitly selects
2500 metres; it does not define a reusable `deep_well` concept. A request to exclude
uncertain locations remains unsupported because the source lacks location quality.

## What this mapping establishes

The YAML expresses dataset identity, field mappings, logical types, descriptions,
a primary key, and AI context. Units, observed status/basin labels, and grain are
documented in prose; they are not machine-enforced unit types or enum constraints.
The model does not invent a CRS, depth datum, relationships, or metrics.

Descriptions and `ai_context` are authored metadata, not executable predicates.
The current compiler does not read this file. This model intentionally goes beyond
the proposed minimal orders importer profile by including keys, field descriptions,
and AI context. An importer must preserve them and report unsupported semantics;
it must not claim complete support merely because all seven fields can be scanned.

The mapping was checked against the upstream validator, CSV headers and values,
and a DataFusion projection built from its seven field expressions. Running the
existing query over that projection returned the same result as the direct CSV
query. That verifies this mapping's query equivalence, not an implemented importer
or LLM consumption of the annotations.

For structural validation, use a checkout at the pinned commit and the Python
dependencies described in the [integration assessment](../../docs/ossie-integration.md):

```sh
python /tmp/ossie/validation/validate.py examples/geospatial/wells.ossie.yaml
```
