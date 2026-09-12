# ClickHouse connector assessment

Assessed 2026-09-12 against commit `fb46393`, DataFusion 55.0.0,
datafusion-federation 0.5.6, and clickhouse-rs 0.15.2. Execution probes used the
repository's ClickHouse 25.8.3.66 fixture. Recommendations below are based on
source inspection and small diagnostic queries, not production benchmarks.

The highest-value work is preserving remote execution when only part of a query
is unsupported, making fallback scans selective, and bounding decoded memory.
The connector already provides a useful read-only foundation: streaming Arrow,
same-connection join and aggregate federation, SQL-compatible empty aggregates
and outer joins, connection isolation, timeouts, byte limits, and secret handling.

## Measured behavior

Each diagnostic below executed one remote request. Bytes are the connector's
HTTP-payload counter, before Arrow IPC decompression; they are not decoded array
memory or ClickHouse storage bytes read.

| Query over the six-row `samples` fixture | Federation | Received rows | Received bytes |
| --- | --- | ---: | ---: |
| Grouped `SUM(depth_m)`, ordered by `well_id` | Enabled | 2 | 504 |
| Same query with `SQRT(SUM(depth_m))` | Enabled; falls back | 6 | 1,568 |
| `SELECT well_id FROM samples` | Enabled | 6 | 544 |
| `SELECT well_id FROM samples` | Disabled | 6 | 1,568 |
| `WHERE depth_m >= 15 AND SQRT(duration_min) > 1` | Enabled; falls back | 6 | 1,568 |

The last query returns three rows locally. Its ordinary comparison is eligible
for remote execution, but the unsupported scalar expression prevents this
candidate from being federated. EXPLAIN confirmed a local filter and scan.

A second probe bound an ordinary view containing 10,000 copies of a 1,024-byte
string. With `max_response_bytes = 262144`, execution succeeded with 82,560
payload bytes and 10,280,124 bytes reported by Arrow's array-memory accounting.
This establishes that the response budget is not a decoded-memory budget; it
does not measure process peak memory. The server's Arrow codec was `lz4_frame`.

## Prioritized work

Priority 1 means the next implementation tranche. Priority 2 means valuable
follow-up work. Priority 3 depends on actual deployment or workload needs.

| Priority | Missing capability | Expected value | Relative effort |
| --- | --- | --- | --- |
| 1 | Projection and supported predicates in fallback scans | Avoid transferring unused columns and rejected rows | Medium |
| 1 | Preserve supported subplans below unsupported operators | Keep large reductions such as GROUP BY remote | High |
| 1 | Decoded-memory accounting and shared query budgets | Bound resource use across decoding and multiple scans | Medium–high |
| 1 | Run connector integration tests in CI; expand compatibility coverage | Detect SQL, transport, and server-version regressions | Medium initially; ongoing |
| 2 | Broader, type-aware SQL pushdown | Support common semantic metrics and time-based analysis efficiently | Medium per feature family |
| 2 | Query metrics, fallback reasons, structured errors | Explain slow queries and diagnose failures | Medium |
| 2 | Runtime join filters and useful statistics | Reduce data transfer for mixed-source joins | High |
| 2 | Explicit type and schema-evolution contracts | Make broader ClickHouse schemas dependable | Medium–high |
| 3 | Engine-aware read options, discovery, deployment configuration | Reduce setup friction for more installations | Varies |
| 3 | Codec, dictionary, batch-size, and client-parallelism tuning | Improve measured transport bottlenecks | Benchmark first |

### 1. Make fallback scans selective

[ScanTable::scan](../../crates/semantic-clickhouse/src/lib.rs) builds its SQL from
every field in the bound schema. It applies the supplied projection and limit
through DataFusion's local `StreamingTable`; it rejects pushed filters and does
not implement `supports_filters_pushdown`.

Implement remote projection from the supplied indices and return the matching
schema. Preserve named-column selection, which already protects against remote
column additions and reordering. Handle empty projections explicitly so
count-only local execution still receives the right row count.

Add a shared expression qualification/translation layer for supported scan
predicates. Push supported conjuncts while retaining residual expressions in
DataFusion. Only advertise exact predicates when ClickHouse and DataFusion
semantics agree for the operand types. The GitHub connector already demonstrates
the repository's `supports_filters_pushdown` pattern.

Forward a limit only when the planner's contract and residual-filter placement
make it safe. In particular, limiting input before a local residual filter can
omit qualifying rows. Local limits can already stop polling early, so the
current behavior should not be described as always consuming the entire table.

Acceptance: narrow fallback SQL references only required columns; mixed
supported/unsupported predicates return identical results while transferring
fewer rows; residual-filter LIMIT and zero-column cases remain correct.

### 2. Keep supported portions of rejected candidates remote

[restrict_federation](../../crates/semantic-clickhouse/src/policy.rs) unwraps an
entire candidate when `supported()` rejects any node or expression. It does not
extract smaller supported subplans. This is why `SQRT(SUM(depth_m))` loses remote
aggregation even though SQRT could run over the two aggregate results locally.

Split plans at unsupported operations and retain maximal supported remote
children. Respect aggregate, join, sort, and limit boundaries; splitting arbitrary
expression trees is not sufficient. Reuse the capability checks from scan
pushdown so the two paths cannot silently diverge.

The [engine federation guard](../../crates/semantic-engine/src/federation.rs)
also skips federation for a plan containing a remaining `IN` subquery because
the pinned federation version errors on that form. This is shared optimizer work
as well as connector work. Upgrading the dependency requires rechecking the
nested-alias workaround and subquery behavior.

Acceptance: the SQRT-over-SUM probe still transfers two rows; unrelated supported
branches remain remote; local residual evaluation preserves NULLs and join
multiplicity.

### 3. Bound decoded memory and whole-query work

[transport::execute](../../crates/semantic-clickhouse/src/transport.rs) checks
the byte budget before `StreamDecoder` decompresses Arrow messages.
`Decoder::push` also accumulates every batch decoded from a chunk into a vector
before yielding. Neither connector path reserves decoded buffers through the
DataFusion memory pool. The existing TaskContext parameters are unused.

Introduce separate accounting for transport payloads, decoded batches, and
in-flight memory. Yield batches incrementally and investigate decoder allocation
limits: checking array size only after decoding cannot prevent the allocation
that crossed a limit. Include conversion buffers and dictionary expansion in
memory tests. Integrate reservations with DataFusion where feasible.

Add a shared query deadline/budget and per-connection concurrency limit. Current
timeouts and byte caps apply independently to each remote execution. Expose a
small allowlist of server resource settings, such as memory/read/result limits
and spill thresholds, while preserving the connector's semantic settings.
Pin relevant overflow modes to `throw` so profile changes cannot turn resource
limits into successful partial results. These ClickHouse limits have different
units and distributed scope from client budgets.
[ClickHouse query restrictions](https://clickhouse.com/docs/concepts/features/configuration/settings/query-complexity)
document those distinctions and partial-result modes.

Acceptance: highly compressible data and concurrent scans are accounted for;
memory exhaustion fails clearly; no configured server limit silently truncates
results. Do not claim a strict peak-memory ceiling without covering allocations
inside the Arrow decoder.

### 4. Make integration and equivalence checks routine

The two Docker tests pass locally, but both are ignored during ordinary cargo
test. The checked-in [CI workflow](../../.github/workflows/ci.yml) runs ordinary
workspace tests and does not invoke `just test-connectors` or its equivalent.
Add a dedicated Docker-backed CI job.

The current fixture provides useful independent references for simple filters,
aggregation, inner/left joins, NULL ordering, aggregate-state views, fresh reads,
connection isolation, limits, and failures. Expand it with:

- Integer boundaries, decimal precision/scale and aggregation, non-finite
  floats, date/time zones and precision, binary strings, dictionaries, and
  nested types.
- Right/full joins, duplicate and NULL keys, residual join conditions, offsets,
  implicit casts, semi/anti joins as support expands, and mixed local/remote plans.
- Midstream server errors, schema-only/empty results with changed schemas,
  consumer drop during streaming, and server-side cancellation verification.
- Compression/memory stress, wide selective scans, larger grouped queries, and
  several concurrent executions.
- A declared minimum server version plus a current supported version. Only
  25.8.3.66 is exercised by the existing fixture.

Keep correctness and performance assertions separate. For pushdown regressions,
assert plan placement and transferred rows/columns alongside result equivalence.
Use larger controlled benchmarks for latency claims; six rows are insufficient.

### 5. Expand SQL pushdown by semantic family

The [policy](../../crates/semantic-clickhouse/src/policy.rs) permits comparisons,
Boolean expressions, NULL checks, and five builtin aggregates. Missing explicit
capabilities include arithmetic, casts, scalar functions, CASE, LIKE, list IN,
DISTINCT aggregates, aggregate FILTER, windows, unions, and semi/anti joins.
Some syntax can be rewritten by DataFusion into supported forms, so surface SQL
alone does not prove fallback; inspect the optimized plan.

Prioritize metric arithmetic, compatible casts, conditional aggregates, exact
distinct counts, and date/time bucketing. A ratio such as
`SUM(depth_m) / SUM(duration_min)` is especially relevant to semantic models.
Partial federation would let the ratio remain local while its sums execute
remotely, even before arithmetic itself is qualified.

Use operand/result types as part of qualification. Today builtin identity is
checked for aggregates, but there is no comprehensive type whitelist for
comparisons and aggregate inputs. Preserve empty/all-NULL behavior, integer
overflow policy, decimal scale, division by zero, Unicode behavior, and timezone
semantics. Approximate distinct and quantile functions should be explicit
capabilities with an accuracy contract, not replacements for exact SQL.

### 6. Add query-level diagnostics and operational controls

[QueryMetrics](../../crates/semantic-clickhouse/src/lib.rs) exposes cumulative
request, row, and byte counts per connection. It does not implement the
federation executor's optional DataFusion metrics hook. Errors in the transport
are reduced to sanitized generic messages.

Add per-execution identifiers, elapsed/time-to-first-batch measurements, decoded
memory, failure categories, and fallback reason codes. Connect metrics to
EXPLAIN ANALYZE; expose remote SQL only through an intentional diagnostic path
because it can contain literals. Preserve safe ClickHouse error codes and a
query ID without forwarding arbitrary error bodies. The existing connection
`log_comment` is useful but cannot distinguish simultaneous queries.

ClickHouse supports query IDs and progress/summary information over HTTP.
[HTTP interface](https://github.com/ClickHouse/clickhouse-docs/blob/main/docs/integrations/interfaces/http.md)
documents these facilities and the possibility of errors after response headers.
Measure server rows/bytes read separately from result bytes to see whether
storage pruning is effective.

Disconnect cancellation already exists. Improve its observability and live
tests before adding an explicit cancellation API. Automatic retries are absent;
if needed, limit them to qualified transient failures before any batch is
delivered and retain a total deadline. Reads may see different data after a
retry, so even pre-delivery retries require a freshness policy.

### 7. Optimize mixed-source joins

[Executor::execute](../../crates/semantic-clickhouse/src/lib.rs) deliberately
ignores runtime physical filters. Small local or other-source inputs therefore
cannot dynamically restrict a large ClickHouse scan. Same-connection joins
already fuse; independent connections deliberately do not.

Implement bounded runtime key/range filters for qualified join types, with NULL
and duplicate handling, cardinality thresholds, and correctly typed values.
Start with small key sets; consider external-table transport only if large
sets justify the complexity. Do not merge independently resolved credentials
or connections solely because endpoint strings match.

Implement conservative statistics through the federation executor's existing
statistics hook and/or the fallback provider. Metadata-derived row counts are
estimates, especially for views and deduplicating engines; they must not be
advertised as exact filtered or aggregate cardinalities.

### 8. Define type, schema, and engine contracts

The connector discovers Arrow schemas with `SELECT * ... LIMIT 0`; it does not
retain ClickHouse-native type or table-engine metadata. Normalization casts
columns positionally and checks count and nullability. This does not constitute
a complete schema-drift or lossless-conversion policy.

Document supported mappings and add explicit binary/text behavior. Forcing
ClickHouse String to Arrow text is unsuitable for arbitrary binary data. Qualify
Date/Date32/DateTime64, Decimal, UUID/IP, Enum, LowCardinality, Array/Tuple/Map,
and newer JSON/Dynamic/Variant types individually. Several can already flow
through Arrow; the gap is a tested connector contract, not blanket absence.

Current ClickHouse documentation includes
`output_format_arrow_unsupported_types_as_binary`, defaulting to enabled, which
can represent AggregateFunction states as binary. Our older fixture does not
expose that setting and currently rejects raw states. Because the connector
relies on server serialization failure rather than native type inspection, its
documented raw-state rejection needs a version-aware enforcement strategy.
This is a compatibility risk, not a failure reproduced on a newer server.
[Arrow format settings and mappings](https://clickhouse.com/docs/reference/formats/Arrow/Arrow)
describe the relevant controls.

Discovery is explicitly disabled. Add optional table/view enumeration,
schema refresh, native type/engine metadata, and bounded metadata caching.
The shared loader already reuses named connections and source bindings;
it binds distinct required sources serially. Bounded concurrent metadata
binding can improve startup for large catalogs without adding row-query I/O.

Table configuration currently accepts only a literal name. For ReplacingMergeTree
datasets, provide an explicit current-state view or a qualified opt-in FINAL
read mode when users need deduplicated rows. Raw reads are not automatically
incorrect; their semantics differ from current-state reads. ClickHouse explains
query-time engine behavior in its
[engine overview](https://clickhouse.com/blog/updates-in-clickhouse-1-purpose-built-engines).
Keep finalized aggregate views as the default boundary for AggregateFunction
states; do not average already-averaged batches or blindly finalize unmerged states.

### 9. Broaden deployment configuration when required

[ConnectionOptions](../../crates/semantic-sources/src/clickhouse.rs) supports
endpoint/database/user/password reference plus timeout, byte budget, and the
federation switch. There is no connector configuration for custom CA roots,
mTLS, token authentication, selected roles, proxy routing, failover endpoints,
or separate connection/read deadlines. Some capabilities already exist in the
underlying client but are not exposed here.

Add these in response to deployment requirements, retaining explicit secret
resolution and validated options. Public HTTPS with user/password already covers
many deployments; absence of these options does not imply ClickHouse Cloud is
unsupported.

## Optimizations to benchmark or leave to ClickHouse

- **Pooling:** clickhouse-rs already owns a reusable Hyper connection pool. The
  missing capability is configuration/measurement, not connection reuse.
- **Compression:** this build disables the client's optional outer compression,
  but enables Arrow LZ4/Zstandard decoding. Arrow compression is already active
  in the tested server. Benchmark codecs and dictionary output before adding
  another compression layer. Dictionary output is explicitly disabled today.
- **PREWHERE:** eligible WHERE predicates can be moved automatically by
  ClickHouse. First ensure filters reach the server. Manual PREWHERE needs
  workload evidence and care around FINAL and outer joins.
  [PREWHERE documentation](https://clickhouse.com/docs/sql-reference/statements/select/prewhere)
  explains those semantics.
- **Client scan parallelism:** one remote stream does not mean one server worker.
  ClickHouse can parallelize internally. Only split scans after measuring a
  transport bottleneck and defining disjoint partitions, stable read semantics,
  and global sort/limit behavior. Avoid OFFSET-based pagination.
- **Indexes, storage projections, materialized views, and query caches:** these
  are primarily server/model design choices. Exposing plans and storage-read
  metrics will make them easier to tune. Existing finalized views already work.
  Result caching needs an explicit freshness contract because current reads
  intentionally observe new inserts.
- **Writes and CDC:** the connector is intentionally read-only. Inserts, schema
  management, and transactions would be a separate product scope.

## Suggested implementation sequence

1. Add the Docker CI job and preserve these measured cases as regressions.
2. Implement fallback projection and qualified predicate pushdown, including
   empty-projection and residual-filter LIMIT cases.
3. Add decoded-memory accounting, shared budgets, and useful per-query metrics.
4. Split rejected federation candidates to preserve supported remote children.
5. Expand arithmetic/date/conditional/distinct support through equivalence tests.
6. Use workload measurements to choose runtime join filters, statistics,
   dictionary output, deployment options, and engine-aware reads.

Validation performed: seven semantic-clickhouse unit tests, nine semantic-sources
unit/integration tests that do not require Docker, and both existing ClickHouse
Docker integration tests passed. Two temporary diagnostic tests also passed and
were removed after analysis. The initial sandboxed timeout test could not bind
a local socket; rerunning with socket access passed. No connector implementation
changes were made. CI was inspected, not executed remotely.
