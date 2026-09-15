# Built-in connectors and file configuration

## Capability matrix

All listed sources can participate in semantic projections, authored SQL views,
cross-source joins, and Ask over supported catalog types. Pushdown means doing
work at the source rather than in Semantic DB. Core support does not imply every
upstream type, function, or operation is implemented.

| Connector | Physical schema | Source optimization | Transport / limits |
| --- | --- | --- | --- |
| `postgres` | Database metadata; validate against Ossie | Column selection and limits; filters/joins/aggregates execute locally | Native pooled cursor reads; currently `sslmode=disable`, no TLS |
| `clickhouse` | Metadata / Arrow; validate against Ossie | Qualified projections, predicates, aggregates, sorting, limits, same-connection joins | HTTP(S), bounded Arrow streams; unsupported SQL falls back locally |
| `csv` | Ossie-guided parsing plus inference | Scan projection; sequential parsing | Local files/directories; CSV/TSV and configurable delimiter |
| `json` | Ossie-guided parsing plus inference | Scan/local execution | Local newline-delimited JSON (`.json`, `.jsonl`, `.ndjson`) |
| `parquet` | Embedded schema; validate against Ossie | Column pruning and metadata-based predicate skipping where applicable | Local files/directories; internal compression |
| `avro` | Embedded schema; validate against Ossie | Scan/local execution | Local Avro object-container files |
| `arrow` | Embedded schema; validate against Ossie | Column selection/local execution | Local Arrow IPC files; no Flight or IPC stream promise |

`file` selects one of the five file formats automatically. PostgreSQL currently
maps text/varchar, boolean, int2/4/8, float4/8, date, timestamp and timestamptz.
Unsupported types fail explicitly. ClickHouse has its own qualified type and
query policy; see the source connector reference for the exact implementation.
File types are limited by the DataFusion reader and executable Ossie profile;
an upstream format's ability to store nested values does not establish Ossie
nested-type support.

CSV/JSON support whole-file gzip, bzip2, xz and zstd compression. Extensions
`.gz`, `.bz2`, `.xz`, `.zst`/`.zstd` select the decompressor. Parquet, Avro and
Arrow use their built-in compression rather than an outer compressed file.
The binary enables DataFusion's compression and Avro codec features; encrypted
Parquet is not included. Gzip is covered by the end-to-end binary smoke tests.

| Connector | Writes | Read consistency |
| --- | --- | --- |
| `postgres` | Opt-in INSERT, UPDATE, DELETE and restricted keyed MERGE; explicit library transactions | Observed reads; verified single-domain snapshots and session-scoped commit receipts |
| `clickhouse` | Read-only | Observed reads; no snapshot/session guarantee |
| `file`, `csv`, `json`, `parquet`, `avro`, `arrow` | Read-only | Observed reads; no filesystem snapshot guarantee |

PostgreSQL connections stay read-only unless `write_enabled: true`. Writable
Ossie projections must be reversible and expose a supported target key; authored
views remain read-only targets. See [writes and reconciliation](writes-and-reconciliation-implementation.md)
for the precise API, supported statements, and checked-input restrictions.
File readers use DataFusion batch execution; CLI/server result buffering still applies. File support does
not imply filesystem snapshots, watched directories, or query-time refresh.

## Minimal project

```yaml
ossie: model.ossie.yaml
connections:
  local:
    connector: file
sources:
  local.products:
    connection: local
    path: data/products.csv
```

Ossie `source` values are binding keys, not file paths. Paths are relative to the
project config. File format inference uses the filename; it does not inspect
bytes during offline validation. Database connections still require explicit
connection options and credential references. File connectors reject remote URLs.

Existing `connector: csv` configurations work unchanged. Explicit `parquet`,
`json`, `avro`, and `arrow` connectors also work. For a directory or an ambiguous
extension, supply `format` under the source, or select the explicit connector:

```yaml
sources:
  local.products:
    connection: local
    path: data/products/
    format: json
    extension: .jsonl
```

Directories must contain files with compatible physical schemas. The `extension`
filter defaults to the format suffix (`.json` for JSON); select `.jsonl` or
`.ndjson` explicitly when a directory uses those suffixes. Mixed formats are
separate sources. Use an explicit extension when selecting custom suffixes.

## Ossie guides CSV and JSON parsing

```yaml
fields:
  - name: product_id
    datatype: String
    expression:
      dialects:
        - dialect: ANSI_SQL
          expression: '"PRODUCT_ID"'
```

`PRODUCT_ID` is read as a string, preserving CSV `00123`. The loader first
discovers physical column names and undeclared types, then reopens the reader
with declared type guidance before executing rows. It does not cast already
parsed numbers back to strings. Undeclared fields retain inferred types.

Reader defaults: String → Utf8; Integer → Int64; Float → Float64;
Boolean → Boolean; Date → Date32; Time → Time64 nanoseconds;
DateTime → nanosecond timestamp without timezone; DateTimeTz → nanosecond
timestamp with UTC timezone. These are parsing representations, not claims of
greater source precision. Dates/times use the reader's standard textual syntax.
JSON remains typed JSON: a numeric token need not be accepted where a string
was declared. Invalid values fail rather than silently becoming nulls.
NDJSON integer fields require an integer token or integer string, rejecting
fractional and exponent-form number tokens rather than truncating them. This is
checked lazily while scanning, including rows beyond the inference sample.
NDJSON records are limited to 16 MiB; batches are bounded and stream through a
single-batch channel. Stopping consumption stops the reader at a record boundary.

Shared-source declarations must agree on the logical type of each physical
column. Conflicts are reported offline with model locations. For databases and
self-describing files, physical schemas remain authoritative and Ossie validates
compatibility. No implicit type rewrite is applied to those sources.

## Optional physical details

CSV options are `has_header` (true), `delimiter` (comma, or tab for `.tsv`),
`quote` (double quote), `escape` (none), and `null_regex` (reader default).
Delimiter/quote/escape each accept one ASCII character. CSV and JSON accept
`schema_infer_max_records` (1000, positive), `compression`, and `physical_types`.
Short CSV rows fail rather than being filled with nulls.

Most projects need no physical type map. Use it for information Ossie cannot
express, such as decimal precision/scale:

```yaml
sources:
  local.products:
    connection: local
    path: data/products.csv
    physical_types:
      PRICE:
        Decimal128: [18, 2]
```

Types use Arrow's serde representation. Overrides must be compatible with the
Ossie logical declaration and refer to existing physical columns. Decimal needs
explicit precision/scale; the loader does not guess a financial representation.
Reader support still constrains the selected physical type. Explicit physical
overrides select reader precision: Arrow may truncate extra decimal or timestamp
digits to the chosen scale/unit, so choose sufficient precision for the source. Embedded-schema
formats reject physical overrides and inapplicable parsing options.

`--validate` checks configuration and model conflicts without source I/O.
`--validate --connect` discovers schemas and checks mappings. Run a bounded
query as well: schema validation alone does not inspect every value in a file.
