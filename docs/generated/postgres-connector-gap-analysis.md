# PostgreSQL connector gap analysis

_Code review snapshot: 2026-09-28. Scope: the built-in `postgres` source and its
read, write, and snapshot paths. This is an implementation backlog, not a claim
that every PostgreSQL feature should be translated into DataFusion SQL._

## Assessment

The connector is a useful, correctness-first baseline. It binds an explicitly
named schema/table, discovers a schema with `SELECT * ... LIMIT 0`, fetches rows
through a bounded native portal, pushes projected columns and planner-supplied
limits, checks for schema drift, and closes an incomplete scan's connection.
Writes require explicit opt-in and have separate transaction, snapshot, and
reconciliation contracts. None of those strengths imply that ordinary analytical
queries are delegated: filters, aggregates, joins, ordering, and expressions are
currently evaluated by DataFusion after Postgres supplies rows.

The ClickHouse connector provides a useful *pattern*, not a reusable dialect
policy: it has an expression-specific scan fallback, guarded SQL federation for
qualified subplans, transport budgets, execution metrics, and optimized-versus-
baseline tests. PostgreSQL needs its own equivalence rules, parameter handling,
resource controls, and transaction-aware execution.

| Capability | PostgreSQL today | ClickHouse reference | PostgreSQL gap |
| --- | --- | --- | --- |
| Scan projection and limit | Columns are selected by name; empty projection preserves row count; `scan` adds a supplied `LIMIT`. | Scan fallback does the same. | Keep `LIMIT` above residual filters unless pushing it further is proven equivalent; preserve `COUNT(*)` behavior. |
| Filter pushdown | `PgTable::scan` ignores `filters`; no `supports_filters_pushdown`. Session scans do the same. | Per-expression `Exact`/`Unsupported` policy and SQL filter rendering. | Add a conservative, typed predicate translator in both ordinary and session scans. |
| Whole-subplan delegation | Plain `TableProvider`; no SQL federation source or executor. | Federated provider and guarded remote plan for supported operators. | Push qualified same-connection projection, filters, sort/limit, aggregates, and joins; split at unsupported nodes. |
| Join-time filtering | None on PostgreSQL scans. | Optional bounded runtime key filters; local join remains authoritative. | Consider only after basic delegation, with key and byte caps. |
| Types and schema | Text/varchar, boolean, signed integers, floats, date, timestamp, timestamptz; every field marked nullable. Other types reject binding. | Arrow schema and broader type/codec handling, with some source-specific exclusions. | Add a declared PostgreSQL type matrix and accurate nullability, then test conversion and SQL semantics. |
| Resource control | Pool size, fetch size, 10-second connect timeout, fixed 30-second server statement/idle timeout, query deadline, decoded-byte accounting. | Configurable timeouts, concurrency and response budgets, memory reservation, execution counters. | Make limits configurable and observable; bound allocations before decoding and distinguish wire from decoded bytes. |
| Secure transport | Explicit `sslmode` supports plaintext, fallback, encrypted, CA-verified, and hostname-verified modes; custom CA and client certificate/key PEMs are supported. | HTTPS, custom CA/client identity, controlled endpoint options. | Define and enforce a production-safe mode policy; `disable`/`allow`/`prefer` can use plaintext, and `require` without a CA does not authenticate the server. |
| Transactional reads/writes | Explicit write opt-in, pinned read sessions, receipts; session scan collects all batches before yielding. | Read-only analytical connector; no equivalent writes. | Preserve transaction identity during pushdown and bound session buffering. |
| Metadata and planning | Schema at bind time; no provider statistics or refresh API. | Table metadata, approximate row counts, invalidation/refresh. | Add safe statistics, capability/version discovery, and explicit refresh behavior. |

### Evidence in the current tree

- [Ordinary Postgres scans](../../crates/semantic-postgres/src/lib.rs) implement
  projection/limit and the portal stream. The SQL has no `WHERE`, `ORDER BY`,
  `GROUP BY`, or join; `arrow_type` shows the supported type set.
- [Session scans](../../crates/semantic-postgres/src/write.rs) build the same
  projection/limit SQL and collect batches before yielding to avoid interleaved
  cursor deadlocks on a pinned connection. This is a distinct execution path.
- [Source configuration](../../crates/semantic-sources/src/postgres.rs) exposes
  a connection-string secret reference, optional CA/client certificate/key PEM
  secret references, pool size, batch size, and write opt-in. There is no
  pushdown, timeout, or resource-budget option.
- [Postgres TLS](../../crates/semantic-postgres/src/tls.rs) requires an explicit
  `sslmode`: `disable`, `allow`, `prefer`, `require`, `verify-ca`, or
  `verify-full`. `verify-full` checks the trust chain and hostname; `verify-ca`
  checks the chain; `require` checks the chain only when a custom CA is supplied.
  `allow` and `prefer` may fall back to plaintext. The
  [TLS integration tests](../../crates/semantic-postgres/tests/tls.rs) cover the
  modes, invalid trust/name, plaintext fallback, mutual TLS, and a TLS-backed
  write. They are opt-in Docker tests; some also require OpenSSL.
- [ClickHouse federation](../../crates/semantic-clickhouse/src/lib.rs),
  [qualification policy](../../crates/semantic-clickhouse/src/policy.rs), and
  [remote execution](../../crates/semantic-clickhouse/src/execution.rs) show how
  the engine's existing federation planner can delegate safe subtrees.
- [Postgres query tests](../../crates/semantic-postgres/tests/query.rs) cover
  basic values, nulls, a local join, cursor abandonment, timeout, fresh reads,
  and schema drift. They do not prove remote operator placement or reduction in
  transferred rows. [ClickHouse integration tests](../../crates/semantic-sources/tests/clickhouse.rs)
  do both.

## Recommended work, in dependency order

### 0. Establish a production transport and resource baseline (release blocker)

1. TLS implementation is present. Define a production connection policy that
   requires `verify-full` for remote deployments, or an explicit exception for
   weaker modes. In particular, `allow`/`prefer` can downgrade to plaintext,
   while `require` without a custom CA encrypts without server authentication.
   Keep certificate material in the host secret resolver and errors redacted.
   Run the existing valid-TLS, invalid-CA, hostname, fallback, mutual-TLS, and
   TLS-backed-write tests in the connector release gate; add policy tests if a
   production profile is introduced.
2. Expose bounded connection acquisition, statement, idle-transaction, and query
   timeouts. Today the connector overrides the connection's server options with
   fixed 30-second statement/idle limits, which can end an otherwise longer
   engine query. Keep server timeout at or inside the query deadline and make
   cancellation/early stream drop release server work and pool capacity.
3. Account actual protocol bytes if the driver permits it; until then label the
   reported value explicitly as an estimate. Reserve memory before or while
   building Arrow arrays, not only after a batch has been allocated. Cap a single
   fetch by both rows and a practical memory budget; test very wide rows and
   budget exhaustion after earlier batches. Bound or spill pinned-session
   collection without violating its single-connection/deadlock invariant.
4. Add per-scan counters and plan metrics for remote executions, rows fetched,
   decoded bytes, fetches, queue time, first batch, and elapsed time. Keep SQL
   text, parameters, and credentials out of ordinary metrics. Expose a way to
   compare pushed and local plans in `EXPLAIN` and bounded read reports.
5. Define pool health, backpressure, secret rotation, and connection failover
   behavior. Retry only a read that has not yielded a batch and can safely
   restart under the requested consistency level. Never replay a write or a
   pinned snapshot on a different connection without explicit semantics.

### 1. Add conservative scan-level predicate pushdown (first performance milestone)

Implement `supports_filters_pushdown` and use the same translator in `PgTable`
and `SessionTable`. Start with direct column-to-typed-literal comparisons,
`IS NULL`/`IS NOT NULL`, bounded `IN`, and conjunctions for types with proven
equivalence. Bind literal values as PostgreSQL parameters; quote identifiers
separately. Never splice model strings or literal values into SQL. An unsupported
predicate remains local; do not reject the whole query. Mark `Exact` only when
PostgreSQL and DataFusion agree on three-valued logic, casts, collation, time
zone, NaN/infinity, and overflow. Otherwise use `Inexact` only when the remote
predicate is a proven superset, retaining the local residual filter.

Split independent `AND` terms where safe. Treat `OR` and `NOT` as indivisible
unless every branch is qualified; a partial `OR` can lose rows. Check filters on
columns omitted by projection, aliased Ossie fields, and empty projections. Apply
`LIMIT` after local residual filters unless an earlier limit is proven equivalent:
limiting remote rows first can discard later matches and return too few results.
Provide a `filter_pushdown: false` baseline option for equivalence and performance
tests. Acceptance: the pushed and disabled modes match an independently authored
local reference, and a selective indexed predicate measurably reduces rows
transferred from Postgres.

### 2. Delegate qualified subplans to one PostgreSQL connection (main feature milestone)

Use the engine's existing `datafusion-federation` path with a PostgreSQL SQL
executor, dialect, bound table identity, and a conservative policy that splits
unsupported plan nodes into local work. Give only tables that share the same
configured connection and authorization/session context the same compute
identity. Do not fuse separate connections just because their URLs match. Keep
role, row-level-security, and session settings in that identity; two connections
with different effective authorization must not share a remote plan. Keep
cross-source joins local. For pinned read or write sessions, remote execution
must use the pinned transaction and snapshot; if that cannot be ensured, retain
ordinary session scans and local operators.

Require a custom `FederationPlanner` and `ExecutionPlan` that pass execution-time
`TaskContext` to remote execution, following ClickHouse's `RemotePlanner` and
`RemoteExec`. In the pinned `datafusion-federation` 0.5.6, the generic SQL execution
plan discards `TaskContext`, and `SQLExecutor::execute` does not receive it. Remote
execution must recover and use the same `QueryContext` as local scans so query
deadlines, cancellation, byte/request budgets, and accounting remain shared
across all subplans. Do not create a fresh default context per remote subplan or
store query state on the shared connection. Verify combined budget exhaustion and
cancellation in a query containing both delegated work and local scan fallback.

Before enabling an aggregate overload, implement its PostgreSQL result decoding
and checked conversion to the planned DataFusion result type. PostgreSQL
[`SUM(bigint)` and `AVG(integer)` return `NUMERIC`](https://www.postgresql.org/docs/18/functions-aggregate.html),
which the current `arrow_type` and `batch` functions cannot decode. The required
result codecs and conversion rules belong in this milestone, before those
overloads are delegated; broader source-column type support can follow in
milestone 3. Until decoding, precision, rounding, and overflow semantics are
proven, retain the affected aggregate locally. Tests must cover both successful
conversion and boundary values that cannot be represented in the planned type.

Qualify operators incrementally: projections and expressions; filters; ordered
limit/offset; `COUNT`, `SUM`, `AVG`, `MIN`, `MAX` and grouping; then equijoins,
outer/semi/anti joins, and `UNION ALL`; finally selected windows and scalar
functions. Check each against DataFusion result types, null/empty-set behavior,
duplicate multiplicity, integer/decimal overflow, floating-point nonfinite
values, `ORDER BY ... NULLS`, collation, and timestamp interpretation. Avoid
function-name-only matching because user-defined functions can shadow builtins.
Keep unsupported UDFs, casts, and plan forms local. An aggregate test must show
Postgres returns grouped rows rather than raw input; a same-connection join test
must show one remote subplan and identical result multiplicity.

### 3. Expand type, metadata, and planner coverage (feature richness)

- Specify and implement the common PostgreSQL types needed by the product:
  `NUMERIC`/decimal (including precision/scale and overflow), `UUID`, `BYTEA`,
  `JSONB`, `TIME`, intervals, and appropriate arrays. Build on the aggregate
  result codecs required in milestone 2. Decide explicitly how
  domains, enums, custom types, and unsupported values behave. Keep schema and
  value conversion consistent in reads, predicates, aggregates, and writes;
  never silently stringify an unsupported type.
- Derive nullability where reliable, including outer-join result nullability
  for delegated plans. Detect table/view and column changes with an explicit
  refresh/rebind path. Fail closed on stale type or column assumptions.
- Supply appropriately labeled approximate row/size statistics to DataFusion
  and measure whether they improve join choice. Obtain them through bounded,
  permission-aware metadata queries; unknown statistics are preferable to
  misleading exact values. Add a capability/version probe for features that
  differ by server version or extension.
- Consider prepared-statement reuse, cursor versus binary `COPY`/Arrow transfer,
  parallel partitioned reads, and runtime key filters only after profiling.
  Parallel reads can change snapshot consistency and must not be enabled for
  pinned sessions without a proven shared-snapshot design.

## Verification and release gates

| Gate | Required proof |
| --- | --- |
| Correctness | Compare optimized, pushdown-disabled, and independent local results through direct bindings and the configured Ossie loader. Cover nulls, aliases, unselected filter columns, residual filter plus limit with matches beyond the first remote rows, duplicate keys, empty/all-null aggregates, integer aggregate result conversion and precision/overflow boundaries, cross-connection joins, and unsupported expressions. |
| Placement | Assert `EXPLAIN` contains the remote SQL/operator boundary for representative predicates, aggregates, and same-connection joins. Assert unsupported expressions remain in local operators. |
| Source work | Use PostgreSQL statement logs or `EXPLAIN (ANALYZE, BUFFERS)` in disposable fixtures, plus connector row/byte counters, to prove fewer transferred rows and useful index access. Compare latency at selective and unselective cardinalities; a logical plan alone is insufficient. |
| Failures | Run the existing TLS verification and fallback tests; add missing privilege, schema change, server error after partial delivery, pool saturation, cancellation, timeout, memory/byte budget, session expiry, and abandoned-stream cases. A late failure must invalidate the whole answer. |
| Query context | In a query combining delegated subplans and local scan fallback, prove that all remote work uses the same query deadline, cancellation signal, and cumulative byte/request budgets. Verify combined budget exhaustion even when each subplan would fit individually, and that cancellation releases remote work and pool capacity. |
| Transaction semantics | Re-run snapshot, read-your-writes, receipt, write, and reconciliation tests with optimization enabled. Verify pinned-session operators never escape to a fresh pooled connection and ordinary scans do not falsely promise a shared snapshot. |
| Operations | Publish supported types/operators, exact versus residual behavior, required PostgreSQL privileges, timeout/budget defaults, safe TLS configuration, and a bounded Docker-backed performance fixture under `docs/generated`. |

The existing Docker-backed PostgreSQL tests are opt-in; extend them rather than
making live infrastructure a dependency of ordinary unit tests. A practical
release target is a production TLS policy, bounded execution, exact pushdown for
a documented predicate subset, and guarded same-connection federation for the
common SQL operators above. Broader PostgreSQL syntax should remain a measured
follow-up, with explicit local fallback whenever equivalence is unproven.
