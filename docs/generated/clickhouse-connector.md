# ClickHouse connector

The first ClickHouse connector uses `datafusion-federation` 0.5.6 with DataFusion
55, the existing ClickHouse Rust client, and Arrow IPC. It does not depend on
`datafusion-table-providers`. The standard CLI includes it; embedded applications
enable the facade's `sources` and `clickhouse` features.

Run the Docker-backed integration tests with:

```sh
just test-connectors
```

These tests use testcontainers with ClickHouse `25.8.3.66`, ephemeral mapped ports,
isolated databases, and automatic container removal. Each test is marked
`#[ignore = "connector_integration: requires Docker; run just test-connectors"]`
and its name starts with `connector_integration`. Ordinary `just test` skips them.
Docker must be running; a missing daemon is an error, not a silently skipped test.
The recipe selects that name marker and passes `--ignored`, so other ignored
tests such as live model-provider tests are not selected.

## Drilling fixture

The fixture is [drilling.sql](../../examples/clickhouse/drilling.sql).
It uses fictional per-sample incremental drilled distance, duration, and nullable
load measurements. It deliberately includes unequal batch sizes, NULL loads and
a well with no measurements.

| Relation | ClickHouse storage | Query contract |
| --- | --- | --- |
| `samples` | MergeTree | One row per well/sample; semantic `depth_m` aliases physical `drilled_m`. |
| `wells` | MergeTree | Well-to-basin mapping, including an unmatched well. |
| `totals` | SummingMergeTree populated by a materialized view | Sum partial measures by well; background merging is not a prerequisite. |
| `summary` | View over AggregatingMergeTree populated by a materialized view | GROUP BY with `sumMerge`, `avgMerge`, and `countMerge` produces ordinary Arrow values. |

Raw `AggregateFunction` columns are not supported as portable Arrow values.
Define the appropriate finalized ClickHouse view and bind that view. Do not use
`finalizeAggregation` alone as a substitute for merging multiple states by key,
or average per-batch averages. The fixture's first well has loads 10, 20, 90 and
NULL: its mean is 40, not 52.5. Distance totals are 60 m and 20 m for the two
measured wells. Tests stop background merges, check results across separate
inserted parts, run `OPTIMIZE FINAL`, and verify the same results plus fresh reads
after another insert.

## Running the example manually

Create an isolated local database and load the same fixture:

```sh
docker run -d --rm --name semantic-db-clickhouse -p 127.0.0.1:8123:8123 \
  -e CLICKHOUSE_DB=drilling -e CLICKHOUSE_USER=fixture \
  -e CLICKHOUSE_PASSWORD=fixture-password -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  clickhouse/clickhouse-server:25.8.3.66
# Wait until http://localhost:8123/ping responds, then:
docker exec -i semantic-db-clickhouse clickhouse-client --user fixture \
  --password fixture-password --database drilling --multiquery < examples/clickhouse/drilling.sql
CLICKHOUSE_PASSWORD=fixture-password just cli --config examples/clickhouse/semantic-db.yaml \
  --file examples/clickhouse/query.sql
docker stop semantic-db-clickhouse
```

The credentials above belong only to the disposable example. Library callers
supply secrets explicitly; the configured connector uses `password_env` as the
name passed to the existing secret resolver. Offline inspection validates options
without resolving secrets or contacting ClickHouse.

## Connection and execution contract

Required connection options are `endpoint`, `database`, `user`, and
`password_env`. Optional options are `query_timeout_seconds` (30),
`max_response_bytes` (268435456), and `federation` (true). The timeout must be
positive and at most one day; the byte cap must be positive. Each source requires
`table`, a literal table/view identifier inside that database. Unknown options
are rejected. SQL text and inline passwords are not source configuration options.
Database, table and column identifiers containing backslashes are rejected.
Endpoints require HTTPS, except loopback HTTP for local development; URL
credentials, paths, queries and fragments are rejected.

Client construction performs no I/O. Table binding obtains an Arrow schema using
`LIMIT 0`; row execution starts only when the returned stream is polled. A fresh
stream is created per execution. Requests are read-only, use SQL-compatible
outer-join NULL behavior and ALL join multiplicity, and have a server execution
limit plus a client deadline. Dropping a stream drops its pending HTTP operation;
the connector requests server cancellation of read-only queries on disconnect.
Server-side termination timing depends on ClickHouse and transport behavior.
No detached retry/cancellation tasks are created by the connector.

The response byte budget and deadline apply per remote subplan/scan, not across
the whole federated query. Byte-budget exhaustion, timeouts, malformed/truncated
Arrow, conversion failures and remote errors fail the query. Consumers of a
stream must invalidate earlier batches if a later batch fails. Driver error
bodies are replaced with sanitized errors; connection debug output excludes
credentials. `ClickHouse::metrics` exposes cumulative executed queries, decoded
HTTP bytes and received rows, excluding metadata. SQL is not retained in metrics;
EXPLAIN may contain query literals and should be handled accordingly.

The enabled federation session retains the engine's information schema and
statement restrictions. Providers sharing the same resolved connection can fuse
subplans. Independent connections have distinct opaque identities even when
endpoint/database/user match. Ossie aliases and authored views preserve the
underlying provider through DataFusion view inlining.
The connector collapses consecutive view/query aliases before SQL generation to
work around DataFusion 55's nested-alias rendering. The engine also bypasses the
core federation rule for local-only plans and remaining `IN` subqueries, which
version 0.5.6 cannot federate.

## Initial pushdown scope

Candidate subplans support projections, ordinary comparisons/Boolean filters,
NULL checks, sorting/limits, equijoins, and builtin count/sum/avg/min/max without
DISTINCT, FILTER, or aggregate ordering. Generated sum/avg/min/max calls use
ClickHouse's `OrNull` variants so empty/all-NULL inputs retain SQL semantics;
count retains zero. Arbitrary scalar functions, casts, windows and unsupported
plan forms fall back to ordinary scans and local DataFusion execution. The
same applies to backslash-containing string literals and query aliases because
ClickHouse's escape rules differ. The fallback reads all bound columns by name
and leaves filter/projection/limit work to
DataFusion; it favors correctness over minimal transfer.

This is an experimental, qualified starting subset, not complete dialect
equivalence. Advanced types, non-finite floating-point values, decimal edge cases
and additional functions need dedicated equivalence tests before expanding
production scope. A rejected candidate currently falls back as a whole rather
than extracting smaller supported subtrees. Cross-connection joins run locally;
runtime join filters are not sent to ClickHouse. There is no cross-source
snapshot, distributed transaction, global query budget, or automatic retry.

The tests compare federated and scan-fallback results against independently
authored local data. They also inspect plans and execution counters to establish
that aggregation reduces six remote rows to two, rather than merely checking
that a query returns the expected answer.
