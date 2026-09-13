ClickHouse connector implementation

Worktree: `codex/clickhouse-connector`. Implementation and validation: 2026-09-13.

## Execution and resource limits

`Engine::execute(sql, QueryOptions)` returns a `QueryExecution` containing a stream and query context. `Engine::query` uses the same path. `prepare` validates without reading rows; execution resolves any materializations before federation. Dropping execution drops in-flight streams; the context also exposes explicit cancellation.

Query contexts share deadlines, request counts, remote-byte budgets and decoded-byte budgets across ClickHouse subplans, GitHub scans, and materialization reads. Defaults are 30 seconds, 256 requests, 256 MiB received, and 1 GiB decoded. The engine's DataFusion memory pool defaults to 512 MiB; embedded callers can use `Engine::with_memory_limit`. Cache memory has its own explicit budget. These are operator/cache budgets, not a process-RSS limit.

Arrow framing and declared decompressed sizes are checked before allocation/decompression. LZ4, Zstd, uncompressed streams, dictionary batches, truncation, trailing errors, and integer conversion failures are handled explicitly. ClickHouse results and schema requests use separate bounded paths. Schema discovery uses `WHERE false LIMIT 0`, which avoids a public-instance read-limit check that can reject plain `LIMIT 0`.

Advanced callers using `plan_sql(...).collect()` directly bypass engine-level materialization and shared query options. Use `execute` for application execution.

Physical-plan metrics include remote requests, rows, bytes, decoded bytes, first-batch time, elapsed time, and applied/skipped runtime filters. Query context metrics include cache hits and misses. Request IDs connect errors to server logs; errors omit SQL, credentials, and response bodies.

## Shared materializations

`semantic-materialization` is independent of connectors. Both source datasets and authored views can opt in. Storage contains Arrow IPC batch files, a versioned manifest, and an atomically replaced `CURRENT` pointer.

- Fills complete successfully before publication. Late errors, cancellation and budget failures leave no published partial generation.
- Refresh is blocking. There is no background refresh or stale-on-error success path.
- A per-entry file lock coalesces concurrent fills; a writer lock coordinates disk admission across processes.
- Readers pin a generation. Eviction skips active disk generations and retains active memory allocations in its accounting.
- TTL age follows acquisition time, including the oldest materialized input for a view. View identities include dependency generations. Stricter view freshness limits propagate to cached ancestors, including through unmaterialized views.
- Identities include source configuration, semantic definition, schema, and an opaque authorization fingerprint. CSV, ClickHouse and GitHub configured bindings supply persistent scopes; custom connectors that omit a scope remain session-scoped.
- Memory admission is bounded; overflow uses disk. Disk generations are evicted by acquisition age. Disk budget accounting covers IPC payload files; small manifest, pointer and lock files add filesystem overhead.
- Refreshing a source causes dependent view materializations to acquire a new dependency identity on their next use.
- Cancellation of the caller doing a fill aborts that fill; waiting callers may subsequently start another fill.

Keep the configured cache directory private to the application/user. TTL defines freshness between explicit refreshes. Recreate a Project or refresh/rebind a ClickHouse provider to pick up a changed schema.

Example project additions (paths are relative to the project file):

```yaml
cache:
  directory: .cache/semantic-db
  max_memory_bytes: 67108864
  max_disk_bytes: 1073741824

sources:
  drilling_samples:
    connection: drilling
    table: drilling_samples
    columns: [well_id, sample_id, drilled_m, duration_min, load_kn]
    materialization:
      max_age_seconds: 300
      max_fill_bytes: 134217728

views:
  drilling_summary:
    sql_file: sql/drilling_summary.sql
    materialization:
      max_age_seconds: 60
      max_fill_bytes: 33554432
```

Cache status and invalidation work without source connections or credential resolution.

CLI controls:

```sh
semantic-db --config project.yaml --cache-status
semantic-db --config project.yaml --cache-refresh drilling_summary
semantic-db --config project.yaml --cache-invalidate KEY_FROM_STATUS
semantic-db --config project.yaml --bypass-cache --query 'SELECT * FROM drilling_summary'
semantic-db --config project.yaml --query-timeout-seconds 60 --query 'SELECT COUNT(*) FROM drilling_summary'
```

The REPL equivalents are `.cache-status`, `.cache-refresh NAME`, `.cache-invalidate KEY`, and `.cache-bypass on|off`. Natural-language execution also uses the engine execution path after generated-SQL validation.

## Federation and qualified SQL

Federation runs after normal logical simplification. A scoped renderer uses internal positional column names to avoid alias collisions. Unsupported operations remain local while supported children can federate. A local `SQRT(SUM(...))`, for example, retains the remote `SUM`. Remaining IN subqueries no longer disable federation for every independent input.

Fallback scans push projection, qualified predicates, and safe limits. Empty projections preserve row counts. `federation` and `filter_pushdown` are independent toggles.

| Family                                    | Remote qualification                                                                             |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Projection, filter, sort, limit, grouping | Supported expressions; explicit null ordering                                                    |
| Joins                                     | Equality inner/outer/semi/anti joins, nulls do not match                                         |
| Aggregates                                | Builtin COUNT/SUM/AVG/MIN/MAX; exact single-expression COUNT DISTINCT; FILTER                    |
| Empty aggregates                          | SUM/AVG/MIN/MAX use OrNull; COUNT retains zero                                                   |
| Arithmetic                                | Float64 arithmetic; integer arithmetic remains local                                             |
| Casts                                     | Identity and lossless integer/Float32 widening; other casts remain local                         |
| Conditionals                              | CASE, COALESCE, NULLIF, BETWEEN                                                                  |
| IN                                        | Non-nullable input and literal non-null list; other forms remain local or decorrelate            |
| Text                                      | Builtin lower/upper translated to UTF8 variants; LIKE without custom escapes/ILIKE               |
| Date parts                                | Literal year/month/day/hour/minute on UTC or unzoned timestamp columns                           |
| Windows                                   | Builtin row_number, rank, dense_rank; unsupported window functions remain local                  |
| Set operations                            | UNION ALL                                                                                        |
| Other functions                           | Local, including arbitrary UDFs, broad date truncation/timezone operations and unqualified casts |

Floating-point comparisons, ordering, grouping, join keys, MIN/MAX, distinct counts, IN/BETWEEN and NULLIF stay local because DataFusion and ClickHouse handle NaN differently. Float64 arithmetic and non-distinct SUM/AVG/COUNT remain qualified. Codec workspace is bounded independently of the whole query byte budget and included in decode memory admission.

Qualification checks builtin implementations rather than accepting a function just because its name matches. Literal backslashes and ambiguous unsupported expressions remain local. This is an explicit qualified subset; adding a SQL function requires typed equivalence tests.

Runtime filters snapshot complete integer/text IN lists at execution. Defaults are 1,000 keys and 64 KiB of generated predicate text. Oversized, negated, unsupported or unavailable filters are skipped, never truncated. The local join remains responsible for correctness.

## Metadata, table options and deployments

`ClickHouse::discover`, `table_names`, `metadata`, `invalidate_metadata`, `refresh_table` and `capabilities` expose discovery and explicit refresh. Table/column metadata has a bounded 60-second connection-scoped cache. Explicit binding requires access to the relevant `system.tables` and `system.columns` metadata. Cardinalities are estimates, never exact facts used to answer queries; FINAL scans omit the raw-row estimate.

Native AggregateFunction state columns are rejected unless excluded by `columns`; use an explicitly finalized ClickHouse view for aggregation states. `final_read: true` applies FINAL to the bound table in both federation and fallback and requires a compatible merge engine.

All deployment options are available through the configured loader:

- Basic authentication: `user` and optional `password_env`. Omitting the password reference means an empty password.
- Bearer authentication: `bearer_token_env`; mutually exclusive with password and mTLS identity references.
- TLS: `ca_pem_env` and `identity_pem_env` (certificate plus private key PEM).
- Explicit `proxy`, `roles`, and `failover_endpoints`.
- `connect_timeout_seconds`, `read_timeout_seconds`, `query_timeout_seconds`, `pool_idle_timeout_seconds`.
- `max_attempts` (default 1, maximum 8), `max_concurrent_requests`, `max_block_size`, `max_response_bytes`, `max_decoded_bytes`.
- `codec: lz4|zstd|none`, `dictionary_output`, `string_as_binary`.
- `runtime_filters`, `runtime_filter_max_keys`, `runtime_filter_max_bytes`.
- Typed `server` limits: memory, read rows/bytes, result rows/bytes, threads, and external group/sort spill thresholds.

HTTP redirects are disabled. Transient retries/failover occur before a successful response body is exposed, with bounded backoff and numeric Retry-After support. There is no retry after stream consumption starts. Failover endpoints must represent the same authorized data scope. Read/result/timeout/group/sort overflow modes are explicitly `throw`; there is no fallback that silently weakens those settings.

Custom CA/mTLS/proxy/role options are configured through reqwest/ClickHouse; production certificate chains and deployment-specific RBAC still need validation in the target deployment.

## Benchmarks and compatibility

Controlled fixture: 50,000 MergeTree rows, five measured repetitions after warmup. These measurements describe this machine and fixture.

| Workload             | Local fallback median | Federated median | Received bytes, fallback → federated |
| -------------------- | --------------------: | ---------------: | -----------------------------------: |
| Aggregate            |             22.235 ms |         5.462 ms |                      200,968 → 1,080 |
| Partial federation   |             22.292 ms |         6.612 ms |                      200,968 → 1,080 |
| Selective projection |             16.191 ms |         4.176 ms |                        200,912 → 544 |

Experimental PREWHERE: 1.761 ms versus 1.840 ms WHERE, about 4.3% improvement. Two client requests: 2.535 ms, about 37.8% slower. Neither meets the adoption gate of at least 15% median latency or 25% bytes improvement, with no comparison regressing more than 10%. No explicit PREWHERE/client-parallel scan mode was enabled. ClickHouse's own server optimizations remain available.

Public instance: `sql-clickhouse.clickhouse.com:443`, user `demo`, version `26.9.1.36875`. The opt-in run completed 32 sequential requests including metadata and warmups, within the 40-request cap. It uses Hacker News IDs below 1,000, a 10-second deadline, 10,000-row blocks/results, and explicit read/result byte limits. This smoke run is not a CI performance gate.

| Public workload    | Fallback median | Federated median | Received bytes |
| ------------------ | --------------: | ---------------: | -------------: |
| Projection         |      216.555 ms |       193.738 ms | 21,248 → 4,272 |
| Aggregate          |      180.506 ms |       162.784 ms |      816 → 440 |
| Partial federation |      185.174 ms |       166.451 ms |      816 → 456 |

Artifacts: [controlled results](clickhouse-controlled-benchmark.json), [public results](clickhouse-public-benchmark.json).

```sh
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test -p semantic-sources --features clickhouse --test clickhouse connector_integration -- --ignored
CLICKHOUSE_TEST_TAG=26.8.2.7 cargo test -p semantic-sources --features clickhouse --test clickhouse connector_integration -- --ignored
cargo test -p semantic-sources --features clickhouse --test clickhouse clickhouse_controlled_benchmark -- --ignored
SEMANTIC_PUBLIC_CLICKHOUSE=1 cargo run -p semantic-sources --features clickhouse --example clickhouse_public_benchmark
```

Docker Desktop installations without `/var/run/docker.sock` may need `DOCKER_HOST` set to the socket reported by `docker context inspect`.

CI pins these verified image digests:

- 25.8.3.66: `sha256:3b28daecdf0625bd7dc27d555ae8f39042882045e49b04668ceccdd282f67d9b`
- 26.8.2.7: `sha256:fa394da808cc53f76d0344429421d6c422a6ee85fe7450135c0e3cff4df9bcbb`

Both LTS versions passed the expanded SQL/type/FINAL integration suite. Workspace tests cover shared cache publication, failed fills, cancellation, concurrent fill coalescing, restart reuse, access isolation, pinned generations, dependent-view invalidation, and bypass. The public benchmark is always opt-in.

## Migration

Existing connection configuration remains accepted. New defaults add engine-wide execution budgets and independently enabled fallback filter pushdown. To get an unoptimized scan reference, disable both `federation` and `filter_pushdown` (and `runtime_filters` for join experiments).

Rust struct literals for project/view/source configuration need the new optional cache/materialization fields; serde configurations can omit them. Embedded custom connectors can supply `SourceConnection::authorization_scope` for persistent reuse. Applications binding providers directly can set a `SourceDescriptor`; changing access scope, schema or source semantics must change that descriptor.
