# PostgreSQL connector: supported behavior and operations

Implementation and validation: 2026-09-28. This accompanies the original
[gap analysis](postgres-connector-gap-analysis.md). The connector now provides
parameterized scan predicates, guarded SQL federation, bounded execution,
metadata refresh, additional value codecs, and execution reporting.

## Configuration

Configured connections accept the following options alongside
`connection_string_env`, certificate secret references, pool/batch sizes, and
`write_enabled`:

| Option | Default | Meaning |
| --- | --- | --- |
| `filter_pushdown` | `true` | Exact predicates and safe conjunction prefilters. Setting `false` also disables federation, providing a local baseline. |
| `federation` | `true` | Delegate qualified ordinary-read subplans. Set `false` to test scan predicates alone. |
| `allow_insecure_transport` | `false` | Explicit exception to the `verify-full` transport policy. |
| `connect_timeout_ms` | 10000 | Native connection establishment. |
| `acquire_timeout_ms` | 10000 | Pool acquisition, including health verification. |
| `statement_timeout_ms` | 30000 | Server statement upper bound, capped by the remaining execution deadline. |
| `idle_transaction_timeout_ms` | 30000 | Server protection for idle transactions, including pinned sessions. |
| `query_timeout_ms` | 30000 | Connector execution upper bound, including queueing; the engine deadline can be shorter. |
| `max_batch_bytes` | 8388608 | Conservative admission budget for one fetch and its Arrow conversion. |
| `max_session_bytes` | 67108864 | Pinned-session collection limit, including conversion admission. |
| `max_in_list` | 256 | Scan predicate parameter count cap for an individual IN list (maximum 4096). Federation additionally caps lists at 256. |

Timeouts must be positive and at most one day. Batch admission must be at least
1024 bytes; session capacity must be at least batch capacity. `batch_size` remains
1–8192 rows (default 1024), and `pool_size` defaults to 8.

```yaml
connections:
  warehouse:
    connector: postgres
    connection_string_env: WAREHOUSE_POSTGRES_URL
    ca_pem_env: WAREHOUSE_POSTGRES_CA
    pool_size: 8
    batch_size: 1024
    query_timeout_ms: 60000
    statement_timeout_ms: 60000
    max_batch_bytes: 8388608
    max_session_bytes: 67108864
```

The connection secret must contain `sslmode=verify-full`. Certificate PEMs and
optional client certificate/key pairs stay in the host secret resolver. Weaker
modes require `allow_insecure_transport: true`; `allow` and `prefer` can use
plaintext, and `require` without custom trust does not authenticate the server.
The development package-maintenance fixtures explicitly opt into this exception
for their local Docker database.

Rust callers can use `Postgres::new_with_options` and `PostgresOptions` for the
same strict default. The compatibility constructors `new` and `new_with_tls`
continue to treat their mandatory explicit `sslmode` as the caller's exception;
use `new_with_options` to enforce the production policy.

Connections use a read-only default, UTC time zone, and `pg_catalog` search path.
Only explicit write sessions switch to READ WRITE. Connection-string server
options are replaced by these controlled settings. Bind explicit schema/table
names; unqualified names inside user-defined server functions should not depend
on a custom search path.

## Operator contract

| Operation | Remote qualification |
| --- | --- |
| Projection | Named columns, empty projection preserving row count, typed literal parameters, qualified expressions. |
| Predicates | Direct typed comparisons of booleans, signed integers, dates and microsecond timestamps; null tests; bounded IN/NOT IN; complete AND/OR/NOT expressions. |
| Partial conjunction | Supported AND terms can prefilter with `Inexact`; the complete residual remains local. Partial OR/NOT is never pushed. |
| Limit | After every residual filter. Ordered limit/offset delegates only with qualified sort keys and expressions. |
| Grouping, joins, ordering | Boolean/integer/date/timestamp keys with qualified expressions. Text collation, float total-order behavior, decimals, UUIDs and other unproven key semantics stay local. |
| Aggregates | COUNT; qualified one-argument COUNT DISTINCT; signed integer SUM; integer/date/timestamp MIN/MAX. Aggregate FILTER is supported when its expression qualifies. |
| Joins | Same configured connection only: inner, left/right/full outer, left/right semi/anti equijoins. Semi/anti joins use EXISTS/NOT EXISTS to preserve multiplicity. Cross-connection joins stay local. |
| UNION ALL | Qualified primitive schemas with matching DataFusion input types. |
| Windows | Builtin ROW_NUMBER, RANK and DENSE_RANK with qualified partition/order expressions; checked bigint-to-UInt64 decoding. |
| Scalars/casts | Builtin COALESCE and NULLIF on qualified primitive operands; widening signed integer casts. Other functions/casts stay local. |

Literal values are bound parameters, including those in delegated SQL. Identifiers
are quoted independently. Operator qualification checks builtin function
implementations, not names. Unsupported nodes split the remote plan into smaller
qualified subplans; they do not reject the whole query. EXPLAIN shows
`PostgresExec delegated=true/false` or `PostgresSessionExec pinned=true`, with SQL
containing placeholders rather than parameter values.

Integer SUM uses NUMERIC accumulation and explicit signed-64 modular normalization
to match DataFusion 55 wrapping-addition semantics, including boundary values and
parent comparisons. Its base-10000 decoder performs exact checked conversion into
the planned Int64 result; other unrepresentable NUMERIC results reject. NUMERIC
NaN/infinity and inexact rescaling are rejected. AVG remains local: PostgreSQL's
numeric integer average and DataFusion's floating accumulation have different
rounding behavior. Floating SUM, decimal aggregates, text ordering/comparisons,
locale-sensitive string functions, arithmetic with unproven overflow behavior,
and arbitrary UDFs also remain local. These are deliberate qualification limits,
not advertised remote capabilities.

## Type matrix

| PostgreSQL | Arrow | Value policy |
| --- | --- | --- |
| text, varchar | Utf8 | No implicit text predicate/collation equivalence. |
| boolean | Boolean | Native boolean values. |
| smallint, integer, bigint | Int16, Int32, Int64 | Native signed values. |
| real, double precision | Float32, Float64 | Values supported; nonfinite-sensitive operators stay local. |
| date | Date32 | Finite values representable by chrono; out-of-range parameters reject without panic. |
| timestamp, timestamptz | Timestamp(Microsecond), Timestamp(Microsecond, UTC) | Finite chrono-representable values. |
| numeric(p,s) | Decimal128(p,s) | 1 ≤ p ≤ 38 and 0 ≤ s ≤ p; exact conversion with precision checks. |
| unconstrained numeric | Decimal128(38,10) | Explicit fixed representation; values outside its precision/scale fail, never round. Cast in a PostgreSQL view to choose another contract. |
| uuid | FixedSizeBinary(16) | The native 16 bytes, not an implicit string. |
| bytea | Binary | Native bytes. |
| jsonb | Utf8 | Explicit UTF-8 JSON representation; binary JSONB version validated. |
| time | Time64(Microsecond) | Times in [00:00, 24:00); PostgreSQL's 24:00 endpoint is rejected. |
| interval | Interval(MonthDayNano) | Months/days preserved; microseconds convert with overflow checks. Write nanoseconds must be divisible by 1000. |
| boolean[], smallint[], integer[], bigint[], text[], varchar[] | List of corresponding primitive | One dimension, lower bound 1; NULL elements, NULL arrays and empty arrays supported. Other dimensions/bounds reject. |
| domains, enums, extension/custom types, other arrays | Unsupported | Binding fails explicitly; expose a view with an intentional supported cast. |

The added codecs also support typed write parameters. Arrow Utf8 by itself does
not distinguish text from JSONB: existing JSONB targets use their native parameter
type; creating a new table from Utf8 creates text. FixedSizeBinary(16) creates UUID,
and Decimal128 creates a declared numeric(p,s). The existing mutation safety and
reconciliation checks still apply.

Catalog `attnotnull` is used for base-column nullability. Views remain nullable
unless PostgreSQL supplies a reliable constraint. Delegated output uses the
planned schema, including outer-join nullability, and rejects unexpected NULLs.

## Resource and transaction behavior

Ordinary scans and remote subplans receive the execution-time TaskContext and
share its QueryContext. Deadlines, cancellation, remote-request and byte budgets
are cumulative across delegated and fallback work. No query state is stored on
the configured connection. The generic federation executor is disabled because
it cannot receive TaskContext in federation 0.5.6.

Native portal fetches are bounded by rows. Raw row lengths are inspected while
receiving the fetch, and memory is reserved before constructing Arrow arrays.
A fetch that exceeds its conservative byte admission limit fails the whole query;
reduce batch_size for wide rows. Query decoded-budget admission includes scratch,
array headers, validity and offsets; per-plan decoded_bytes reports completed
Arrow allocation size. The driver must allocate an individual protocol row before
its length can be inspected; this is not a hard cap on driver/socket buffers or
server-side memory. Exact network-byte accounting is unavailable.

Pinned sessions collect under their single-connection lock to avoid interleaved
cursor deadlocks. Collection is memory-reserved and byte-bounded, and failure or
abandonment poisons/discards the session. Pinned reads use the same predicate
translator but retain local higher operators, so they never escape their original
transaction or snapshot. Ordinary multi-scan queries do not promise one shared
snapshot; use the engine snapshot API when that guarantee is required.

Abandoned or failed ordinary execution releases pool capacity immediately, sends
a native cancel request using the same TLS policy (bounded to two seconds), then
closes its leased connection. Successful work rolls back its read
transaction before recycling. Pool recycling verifies liveness, acquisition is
bounded, and `pool_health()` reports capacity/availability/waiters. No statements
are automatically replayed: writes, pinned snapshots and partial reads are never
retried on another connection. Native configured host selection applies only when
establishing a connection, not to replay an active statement.

For credential rotation, re-resolve secrets and construct/rebind a new Postgres
instance, then `close()` the old pool. Closing rejects queued/future acquisitions;
existing leases finish on their original connection. Every newly constructed
instance receives a distinct federation identity even if its URL is identical.
Clones share the original connection, authorization/settings and identity.

## Metadata and privileges

Binding executes a LIMIT 0 read to verify SELECT privileges, then performs bounded
catalog inspection. Scans acquire ACCESS SHARE locks and validate the relation
OID, column names/types/typmods/nullability/collations and view definition before
execution. Drift detected at that boundary fails closed. `refresh_table()` returns a new provider; replace
catalog bindings or reload the project explicitly. Existing providers are not
silently mutated. `metadata()` exposes the schema revision and catalog estimates;
`capabilities()` reports server version and read-only default.

`reltuples` and `relpages × block_size` feed DataFusion **inexact** statistics.
They may be stale; run ANALYZE as the database owner when useful. Unknown row
counts remain unknown. No expensive COUNT or unrestricted table enumeration is
performed. This establishes planner inputs, not a universal join-planning speedup.

Readers need CONNECT, schema USAGE, SELECT and access to ordinary pg_catalog
metadata. ACCESS SHARE locks are required during validation/execution. Write users
also need the operation-specific privileges and the existing staging/reconciliation
privileges. Roles, RLS and server-side permissions remain enforced by PostgreSQL.
No privilege escalation or role changes are introduced by federation.

## Observability and release gate

`Postgres::metrics()` returns cumulative remote execution, row, fetch, decoded-byte
and estimated-wire-byte counters, excluding metadata. Execution plans additionally
report queue time, first batch and elapsed time. Session plans expose the same
counters. Metrics contain no SQL, parameter values or credentials.

The wire estimate counts DataRow payloads and framing, excluding TLS/TCP and other
protocol messages. The shared QueryMetrics exposes estimated_remote_bytes as the
estimated portion of remote_bytes, and bounded read reports include these metrics
and completion status. Any late error marks the read Failed; partial batches are
not a complete answer.

Run the opt-in Docker/OpenSSL release gate:

```sh
just test-postgres
cargo clippy -p semantic-postgres -p semantic-sources -p semantic-runtime -p semantic-engine --all-targets --all-features --locked -- -D warnings
```

The gate includes TLS verification/hostname/CA/fallback/mutual-TLS tests,
TLS-backed writes, independent local/baseline/optimized equivalence, operator
placement, duplicate joins, nulls and empty aggregates, schema drift, codec
boundaries, partial-delivery failures, privileges, pool saturation, cancellation,
cumulative mixed-plan budgets, pinned collection limits, snapshots, receipts,
read-your-writes and reconciliation. Live infrastructure remains opt-in.

A bounded 10,000-row fixture in `tests/optimization.rs` checks useful PostgreSQL
index access with EXPLAIN (ANALYZE, BUFFERS) and measures selective/unselective
transfer and latency. Run with `--nocapture` to see timings. One local Docker run
reported:

| Query | Optimized rows / estimated bytes / time | Baseline rows / estimated bytes / time |
| --- | --- | --- |
| id=9999 | 1 / 87 / 14 ms | 10000 / 870000 / 238 ms |
| id>0 | 10000 / 870000 / 236 ms | 10000 / 870000 / 207 ms |

These timings are observations, not a performance guarantee. The selective transfer
reduction and index access are asserted; latency is not. The unselective case
shows the additional planning/metadata overhead can outweigh any benefit.

Prepared-statement caching, binary COPY/Arrow transport, parallel partitioned
reads, shared-snapshot parallelism, runtime join-key filters, broader function
qualification and spill-to-disk session collection remain profiling-driven
follow-ups, as identified in the original backlog. This implementation uses a
bounded failure policy for collection and explicit local fallback for unproven
operators.
