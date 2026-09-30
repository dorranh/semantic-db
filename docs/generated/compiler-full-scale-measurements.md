# Full compiler scale measurements

These opt-in host probes use deterministic proposals and synthetic metadata.
They measure catalog/index work and compiler orchestration without a live model,
network, database, or result-row execution. Each row is a measurement of the
specified source snapshot and host, not a production latency guarantee.

## 2026-09-29: 100 relations × 100 fields

The sole verifier ran `cargo run --locked --offline -p semantic-performance
--bin compiler_scale -- --relations 100 --fields 100 --repetitions 7 --warmup 2`
on macOS arm64 with Rust 1.94.0. Frozen source manifest: 476 tracked and
non-ignored untracked files, SHA-256
`218724b1a9a17683525ceb9b1d87f64d0c0160f5068b5947425862e3b1da99ab`.
The source manifest was unchanged before and after the run. Full machine-readable
output is `/tmp/semantic-compiler-batch30-compiler-scale.json` on this host.

| Measure | Observation |
| --- | ---: |
| Total fields | 10,000 |
| Engine/catalog construction | 537.691 ms |
| Cold search index | 2,129.824 ms; 10,100 objects charged |
| First compilation | 8.824 ms |
| Warm compilation, seven samples | p50 2.701 ms; sample p95/p99 2.727 ms |
| Work per warm compilation | One relation, three fields looked up, six nodes visited |
| Rendered context | One relation, four fields, 815 bytes |
| Proposal calls | One deterministic provider call per compilation |
| Plan size | 154 SQL bytes |

The three requested fields are all found; a fourth field is included by the
initial relation-context policy and is reported as `extra_context_fields: 1`.
The seven warm samples establish neither a stable tail-latency percentile nor a
service-level objective. `ps` could not provide process RSS under this sandbox,
so allocation and peak memory remain unmeasured. Index construction is reported
separately from compilation. Broader relation/field shapes, concurrency,
invalidation, cancellation, observation overhead, live-model cost and row
execution remain separate P19 gates.

## 2026-09-29: 1,000 relations × 100 fields

The verifier repeated the same command with `--relations 1000 --fields 100`
against frozen 478-file source manifest
`7373fa133a7dd568088d6a967c309e2a28229e692b74b7a9c95629c0b03b115f`.
The manifest was unchanged before and after. JSON is
`/tmp/semantic-compiler-batch31-compiler-scale.json` on this host.

| Measure | Observation |
| --- | ---: |
| Total fields | 100,000 |
| Cold search index | 23,766.254 ms; 101,000 objects charged |
| First compilation | 8.641 ms |
| Warm compilation, seven samples | p50 2.697 ms; sample p95/p99 2.744 ms |
| Work per warm compilation | One relation, three fields looked up, six nodes visited |
| Rendered context | One relation, four fields, 815 bytes |

The selective request retained the same three-field lookup work when the catalog
grew tenfold. Initial index construction grew substantially and must be measured
and amortized separately. Seven samples remain too few for a release tail-latency
claim; RSS, concurrent admission and model cost remain unmeasured.

## 2026-09-29: eight-client same-key concurrency and cache pressure

The verifier ran the deterministic `compiler_concurrency` binary against frozen
482-file source manifest
`398b7b4db8f45e9c44480a805141700a7a2a773d112a8fa853444346be843f54`.
Its source manifest matched before and after. Machine-readable output is
`/tmp/semantic-compiler-batch32-concurrency.json` on this host.

| Measure | Observation |
| --- | ---: |
| Cold same-key wave | Eight clients; seven cache hits, one miss; p50 11.165 ms |
| Seven warm rounds | 56 additional compiles; 63 cumulative hits, one cumulative miss; p50 1.612 ms, sample p95 2.701 ms |
| Distinct-key pressure | Nine cumulative misses, five evictions, four entries and 7,344 retained bytes |

This checks same-key coalescing and bounded cache retention under a small local
load. It does not measure live model latency, result-row execution, admission
failure behavior, cancellation, service tail latency, or production memory.

## 2026-09-29: observation-mode overhead

The verifier ran `compiler_observation` for 100 measured repetitions after ten
warmups per mode on frozen 485-file source manifest
`d889242a1d5d4486b9420e8f9d241f717ca19f50f56cff09c54096d38f7d5964`.
The manifest matched before/after. JSON is
`/tmp/semantic-compiler-batch33-observation.json` on this host.

| Mode | p50 | Sample p95 | Semantic artifact |
| --- | ---: | ---: | --- |
| Disabled observation | 0.625 ms | 0.685 ms | Identical |
| Normal bounded queue | 0.625 ms | 0.685 ms | Identical |
| Debug with retained stage IR | 1.161 ms | 1.246 ms | Identical |

The unconsumed bounded queue reported 110 dropped events without blocking
compilation. These are single-host microprobe samples of a simple row projection;
they do not establish service latency or model, database, or production telemetry
overhead.

## 2026-09-29: wide relation and million-field catalog

The verifier ran two deterministic `compiler_scale` shapes against frozen 487-file
source manifest
`e432dc7abf757e033adf7b97e1d641bba2e8314d0e816daac3d158a75f2606c6`.
The source manifest matched before/after. Full results are
`/tmp/semantic-compiler-batch34-{wide-scale,million-scale}.json` on this host.

| Shape | Engine build | Index build | Cold compile | Warm p50 | Warm sample p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 relation × 10,000 fields | 0.515 s | 5.048 s | 97.011 ms | 91.662 ms | 92.946 ms |
| 10,000 relations × 100 fields | 53.793 s | 264.141 s | 9.079 ms | 2.833 ms | 2.937 ms |

Both selected one relation and looked up three fields, rendered four context
fields, and visited six plan nodes. The wide-relation latency shows that some
compile work still scales with schema width outside the lookup/context counters;
it is an active optimization target. The million-field catalog met the shape
gate, but its index construction is substantial and must be amortized. RSS
remained unavailable under the sandbox. These local samples are not service
latency or production memory claims.

The next frozen source snapshot (batch 35, 487 files, SHA-256
`36e0ac37d8893fbfdd93c6128a20f057ceffb6dff338cae6bc8683bd5f60c7d3`)
repeated the wide shape for seven measured runs after two warmups. Warm p50 was
91.637 ms and sample p95 97.576 ms. Per-stage timing identified backend
planning as the cost: cold backend 91.121 ms of 95.608 ms total; one warm
sample backend 95.881 ms of 97.572 ms, while context/bind/lower were
61/24/9 µs. Machine-readable output is
`/tmp/semantic-compiler-batch35-wide-scale.json`. A per-engine generated-plan
reuse optimization is the next measured experiment; these timings do not yet
establish its benefit or correctness.

The batch 36 frozen source (489 files, SHA-256
`9ee9cf4769eb0b51d483d938f5b17ebebef4b50373a870ee16fb422ff122deb0`)
added an eight-entry per-engine generated logical-plan cache keyed by exact SQL
and binding generation. Query-only and registered-relation checks still run on
every call; parameterized SQL bypasses this cache. Engine tests confirmed that
a cached plan reads newly added provider rows and that authorization checks
still reject invalid SQL. The repeated 1×10,000 probe used seven measured runs
after two warmups:

| Measure | Before cache (batch 35) | After cache (batch 36) |
| --- | ---: | ---: |
| Warm p50 | 91.637 ms | 1.923 ms |
| Warm sample p95 | 97.576 ms | 2.098 ms |
| One warm backend stage | 95.881 ms | 0.247 ms |
| Cold backend stage | 91.121 ms | 97.834 ms |

The cache removes repeated plan construction in this local fixture; it does not
reduce initial planning or index construction. Its retention budget counts SQL
key bytes and caps entries, not exact heap usage of plans/provider schemas.
Full output is `/tmp/semantic-compiler-batch36-wide-scale.json`.

## 2026-09-30: catalog dependency publication shapes

The verifier ran three deterministic `catalog_dependencies` shapes against
frozen 491-file source manifest
`ab9cf099fedaebeb4ce46cbd3dae6f6852cd5b7b81edb5231bc2fab80894decf`.
The manifest matched before and after, and each root search returned one hit.

| Shape | Affected / total relations | Cold index | Incremental index | Charged objects after change |
| --- | ---: | ---: | ---: | ---: |
| Chain, depth 8 | 9 / 10 | 6.895 ms | 0.580 ms | 3 |
| Diamond, depth 8 | 25 / 26 | 14.091 ms | 0.706 ms | 3 |
| Hub, 32 spokes | 33 / 34 | 20.354 ms | 2.939 ms | 3 |

Every dependent relation was affected, while the unrelated control was
preserved. These one-host measurements exercise catalog publication and index
work only. Results are in
`/tmp/semantic-compiler-batch37-dependencies-{chain,diamond,hub}.json`.

## 2026-09-30: bounded large-prose host probe

The verifier ran `compiler_scale` with 16 relations × 16 fields and a minimum
4,096-byte description per relation, for seven measured runs after two warmups.
The frozen 494-file source manifest SHA-256 was
`ac4e46790d9a7537d941801e751a521ad0d672022a65099a714e2c230e989939`
and matched after the run. Cold compilation was 8.924 ms; warm p50 was
1.934 ms and sample p95 was 2.045 ms. The query looked up three fields;
rendered context held 16 fields in 5,755 bytes, and model input accounting
reported 29,616 bytes. This checks bounded retrieval and host compilation
with synthetic prose, not live model interpretation or production memory.
Machine-readable output is
`/tmp/semantic-compiler-batch38-retry-description-scale.json`.

## 2026-09-30: concurrent provider-wait cancellation

The verifier ran the deterministic `compiler_cancellation` probe with eight
clients on frozen 496-file source manifest
`921a415c6e2405ddb54e225188f286d7bcedb8d0b2fa4d728c59bd37f82551dc`.
The manifest matched after the run. All eight calls reached a gated local
provider: four cancelled while waiting, and four compiled after release with
the same semantic artifact digest. Cancellation completion was 0.085 ms;
survivor p50/p95 latency was 16.198/16.540 ms. The provider is deterministic
and performs no model or network call. This verifies interruption under this
small concurrent load, not admission limits, production tail latency or row
execution. Machine-readable output is
`/tmp/semantic-compiler-batch40-cancellation.json`.
