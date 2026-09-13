# Types and extensibility

Semantic DB uses Arrow schemas for executable relations and optional Ossie
logical types for semantic validation. These are separate contracts: an Arrow
type being representable does not mean every SQL operator, connector, or
natural-language mode supports it. This guide describes the current workspace
(DataFusion 55, Arrow 59.2).

## Ossie logical types

When a field declares `datatype`, the importer checks the resulting expression's
physical Arrow type against this table. Names are case-sensitive. The declaration
validates an expression; it does not cast values or convert units.

| Ossie `datatype` | Accepted physical Arrow types |
| --- | --- |
| `String` | `Utf8`, `LargeUtf8`, `Utf8View` |
| `Integer` | Signed and unsigned integers of 8, 16, 32, or 64 bits |
| `Float` | `Float16`, `Float32`, `Float64` |
| `Decimal` | `Decimal32`, `Decimal64`, `Decimal128`, `Decimal256`, retaining precision and scale |
| `Boolean` | `Boolean` |
| `Date` | `Date32`, `Date64` |
| `Time` | `Time32`, `Time64` |
| `DateTime` | `Timestamp` without a timezone |
| `DateTimeTz` | `Timestamp` with a timezone |
| `Opaque` | Rejected by the current importer |

The exact compatibility rules live in
[the Ossie importer](../../crates/semantic-ossie/src/document.rs).
Omitting `datatype` leaves the physical expression type to DataFusion; it does
not add a logical type mapping for binary, lists, structs, maps, dictionaries,
intervals, or application-specific values. The pinned Ossie schema also limits
which logical names can be declared. See the [Ossie reference](../ossie-reference.md).

Nullability, timestamp units/timezones, decimal precision/scale, and Arrow
metadata remain part of the schema. `Engine::register_table` and catalog loading
require the catalog schema to match the provider schema exactly, including
metadata. Declared primary and unique keys describe intent; they do not enforce
uniqueness or non-nullness.

## Sources and SQL

| Entry point | Current behavior and limits |
| --- | --- |
| Embedded `TableProvider` | Supplies an Arrow schema and batches. Operator support depends on DataFusion and the provider; there is no promise of universal Arrow-type support. |
| Configured CSV | Uses DataFusion's headered CSV reader with sampled schema inference. Bind a provider with an explicit schema in Rust when inference is insufficient. |
| GitHub | Fixed issue/label schemas use UTF-8 text, `Int64` issue numbers, and UTC millisecond timestamps, with field-specific nullability. |
| ClickHouse | Discovers an Arrow schema from the source and normalizes results to the planned schema. Raw `AggregateFunction` states are rejected: bind a finalized view using the appropriate merge functions. |

ClickHouse normalization permits selected numeric casts, string representation
conversions (including dictionary strings), and `UInt8` values 0/1 for Boolean
results. Other type changes, failed conversions, or unexpected NULLs fail the
query. This is not a guarantee that every ClickHouse native type round-trips:
qualify decimals, nested values, non-finite floats, and timezone behavior against
your actual schema. See [transport normalization](../../crates/semantic-clickhouse/src/transport.rs).

SQL functions depend on the workspace's enabled DataFusion features; the
dependency disables default Cargo features. A function existing upstream does
not establish that it is available here. Plan a representative query using
`Engine::plan_sql` or the CLI's `--dry-run` before relying on it. Planning checks
types and names, but execution tests are still needed for edge cases.

The authored-view natural-language mode has a narrower literal filter contract
than SQL: numeric, string, and Boolean request literals are checked by its
[typed lowering](../../crates/semantic-compiler/src/views.rs). Do not infer
date, nested-value, or custom-type literal support from this table. The general
SQL-generating mode validates its proposal through DataFusion; this does not
prove semantic correctness.

## Adding a UDF

A UDF requires executable Rust code, not just a name in the semantic model.
The current `Engine` keeps its `SessionContext` private and exposes no public
UDF registration hook. There is no CLI configuration or dynamic plugin loader
for UDFs. Adding one currently requires extending the engine API or its session
construction; a custom connector registry alone does not register functions.

1. Implement a DataFusion scalar, aggregate, or window function with an explicit
   signature, return type, volatility, NULL behavior, and error contract.
2. Add a deliberate registration hook around the engine's session construction,
   registering the function before planning catalog views and queries. Ensure
   execution sessions used for materializations retain that registry.
3. Test planning and execution for NULLs, empty batches, invalid inputs, and
   relevant numeric/timezone boundaries. Test both ordinary and cached execution.
4. Describe its meaning and units in the model. A function implementing distance,
   for example, must state its coordinate system and units; descriptions alone
   cannot supply those semantics.
5. Keep it local unless a connector implements and tests an equivalent remote
   translation. ClickHouse's allowlist checks builtin implementations, so matching
   a builtin name is not sufficient to authorize pushdown.

Start with [engine construction](../../crates/semantic-engine/src/lib.rs) and
[ClickHouse's expression policy](../../crates/semantic-clickhouse/src/policy.rs).
No spatial or vector-search function is registered by this project today.

## Adding a type

Prefer an existing Arrow representation plus explicit domain metadata when it
accurately represents the value. Defining a new logical name is different from
adding a physical Arrow type and operator support.

For a new domain type, define the representation, nullability, units, ordering,
equality, serialization, and conversion rules first. Then update provider schemas
and batches, Ossie validation/compatibility if a new declaration is needed,
SQL functions/casts, and any compiler literal lowering that should accept it.
The pinned upstream schema must be handled intentionally; adding a match arm
alone cannot make an invalid Ossie document valid.

Test schema registration, source round-trips, views, joins/comparisons, and Arrow
IPC materialization. Expand remote pushdown only after comparing results with
local execution on boundary values. The [connector guide](../building-connectors.md)
describes the provider integration points.
