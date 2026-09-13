# Performance and federation

For federated queries, the decisive cost is often the work performed before rows
reach DataFusion: source scans, API pagination, network transfer, and decoding.
A small result does not imply a small scan. This guide describes the current
implementation; older connector assessments record earlier limitations.

## Where a query runs

| Path | Work at the source | Work in the application |
| --- | --- | --- |
| CSV | Local file reads with DataFusion's provider | DataFusion operators |
| GitHub | Fixed GraphQL selections; exact `issues.state = 'OPEN'` / `'CLOSED'` filters | Other filters, projections, joins, and aggregation; streamed pages can stop when a safe limit is satisfied |
| ClickHouse federated subplan | Qualified projections, filters, joins, aggregates, and other allowlisted expressions on the same resolved connection | Remaining DataFusion operators |
| ClickHouse scan fallback | Selected columns, supported exact filters, and safe limits from the optimizer | Unsupported expressions and cross-connection joins |
| Materialized relation | A complete relation fill on a miss or refresh | Queries over the cached relation on a hit |

ClickHouse federation is expression- and type-specific. Its current
[policy](../../crates/semantic-clickhouse/src/policy.rs) includes selected
scalar functions, widening casts, and window forms, with restrictions on
floating-point semantics and dialect differences. Unsupported candidates can
fall back to local work; smaller eligible subtrees may still federate. Do not use
an old all-or-nothing pushdown list as the current contract.

## Common gotchas

**A final LIMIT does not cap source work.** `ORDER BY`, an aggregate, a residual
filter, or a join may need many rows before producing ten results. A GitHub page
still contains its fixed fields even if SQL selects one column. Label enumeration
can add nested pagination requests, and an issue predicate does not automatically
restrict the separate label scan.

**A same-server join is not necessarily a remote join.** ClickHouse providers
share federation context only when they share the resolved connection. Separate
connections to the same endpoint remain distinct. Cross-source joins execute
locally. Put selective predicates on each side where the query semantics allow;
check that they actually reach each provider. Do not move a predicate across an
outer join without checking its NULL and multiplicity semantics.

**Runtime filters are opportunistic.** ClickHouse's remote execution can snapshot
eligible dynamic positive `IN` filters when the scan starts. The defaults enable
this with at most 1,000 keys and 65,536 bytes of predicate text. Keys must have
supported integer/string representations. Filters that arrive too late, exceed
the caps, or have unsupported shapes are skipped; the local join stays
authoritative. This is not a guaranteed indexed lookup or a way to eliminate
all cross-source transfer. See [runtime filters](../../crates/semantic-clickhouse/src/runtime_filter.rs).

**SQL that looks equivalent may have different pushdown.** Casts, string
escaping/collation, timezones, NULL behavior, and floating-point edge cases affect
whether a connector can reproduce DataFusion's result. A local UDF may require
transferring all its input rows. Inspect plans after changing expressions.

**Aggregation grain matters.** Joining two one-to-many relations can multiply
rows and corrupt a metric as well as increase memory. Aggregate at a deliberate
grain and test against known answers. ClickHouse aggregate states need a view
that merges states by key; averaging per-part averages is not equivalent.

**Streaming does not make every operator bounded.** Joins, sorting, aggregation,
and retained output can consume substantial memory. `Engine::query` collects all
result batches; use `Engine::execute` and consume its stream for large results.
Invalidate partial results if a later batch fails. Consumer-retained batches and
all third-party allocations are not guaranteed to fit inside DataFusion's pool.

## Budgets and concurrency

The standard engine starts with a 512 MiB DataFusion memory pool. Embedded hosts
can construct `Engine::with_memory_limit(bytes)`. This is a pool limit, not a hard
process RSS ceiling. Shared query options have these defaults:

| `QueryOptions` field | Default | Meaning |
| --- | --- | --- |
| `timeout_seconds` | 30 | Query deadline guarded through execution, including cache fills |
| `max_remote_bytes` | 268,435,456 (256 MiB) | Cumulative bytes charged by participating remote connectors |
| `max_remote_requests` | 256 | Cumulative requests charged by participating remote connectors |
| `max_decoded_bytes` | 1,073,741,824 (1 GiB) | Cumulative decoded bytes charged through the runtime; not peak live memory |

Use `Engine::set_query_options` for default queries or pass options directly to
`Engine::execute`. The CLI exposes `--query-timeout-seconds`; it does not expose
every Rust budget as a flag. Connector-specific timeouts and response/scan caps
still apply alongside the shared limits. A custom provider must cooperate with
`QueryContext` for its remote work to be charged; metadata discovery happens
outside row execution and is not an end-to-end query metric.

ClickHouse also has a per-connection semaphore, configured through
`max_concurrent_requests`. Increasing concurrency can increase source load,
network pressure, and queued work without improving latency. Start with defaults
and measure under the intended number of simultaneous queries. GitHub has no
automatic retry/rate-limit sleep; plan repository scope and request budgets.

## Materialization and freshness

Configured projects can define `cache` storage and a `materialization` policy on
a source or view. These are opt-in relation snapshots, not a general SQL-result
cache. The policy uses `max_age_seconds` and `max_fill_bytes`; cache storage has
separate memory/disk budgets. A cold query can fill the entire configured relation
even when its own SQL contains a restrictive filter or `LIMIT`.

Treat the fill cost and staleness window as part of the design. Source descriptors
carry identity/scope/schema information into cache keys; embedded callers must
update them when those semantics change. A snapshot of one relation does not
provide a shared transaction across sources. A live query can also observe
different upstream moments across scans or paginated requests.

The CLI supports `--cache-status`, `--cache-refresh <relation>`, and
`--bypass-cache`. Compare a cold fill, a warm hit, and a bypassed query separately.
Inspect [materialization policies](../../crates/semantic-materialization/src/lib.rs)
and [engine cache execution](../../crates/semantic-engine/src/materialization.rs)
before tuning them. Plain `EXPLAIN` neither fills nor accurately predicts the
eventual cache-hit path.

## A repeatable diagnostic workflow

1. Establish correct output on a small fixture, including NULLs, duplicates,
   unmatched joins, and empty aggregates.
2. Inspect the optimized plan with `EXPLAIN`; the CLI can also plan without
   reading query rows using `--dry-run`. Source binding may still fetch metadata.
3. Execute a bounded workload and inspect `EXPLAIN ANALYZE` when actual operator
   timings/counters are needed. It executes the query and can read remote data.
4. Record elapsed time, time to first batch, source requests, bytes, decoded bytes,
   output rows, and memory under the same concurrency and cache conditions.
   `QueryExecution.context.metrics()` exposes shared counters;
   `ClickHouse::metrics()` exposes cumulative connection counters excluding metadata.
5. Compare with federation/filter pushdown disabled and caching bypassed as
   appropriate. Check result equivalence as well as transfer reduction.
6. Repeat at representative cardinality. Source statistics and runtime-filter
   arrival can change join choices; fixture performance is not a production forecast.

For the local example:

```sh
just cli --config examples/geospatial/semantic-db.yaml \
  --query 'EXPLAIN SELECT basin, COUNT(*) FROM wells GROUP BY basin'
just cli --config examples/geospatial/semantic-db.yaml \
  --query 'EXPLAIN ANALYZE SELECT basin, COUNT(*) FROM wells GROUP BY basin'
```

The [controlled ClickHouse results](clickhouse-controlled-benchmark.json) and
[public endpoint results](clickhouse-public-benchmark.json) capture particular
benchmark runs. Their timings are not SLAs or comparisons with competing engines;
network conditions, dataset versions, server load, and cache state all matter.
