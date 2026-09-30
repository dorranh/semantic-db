# Full semantic compiler requirement audit

Working ledger for the [full implementation plan](compiler-full-implementation-plan.md).
The prospective live-model thresholds and sample protocol are in the
[evaluation protocol](compiler-full-evaluation-protocol.md).
Started 2026-09-29 at `32eef0bbbbdf5c9c6e91d40679875fcc41079048`.
This is a section-level inventory; detailed clauses and validating fixtures are
added with their owning work package. A partial status never means that every
requirement in the section is complete.

## Verified starting point

The sole verifier ran the selected offline P0 regression gate with
`--locked --offline`: 157 tests passed across 20 binaries. The PostgreSQL library
lane passed nine tests. No failures or ignored tests were reported. The verifier
recorded an unchanged tracked-source manifest digest
`e628fc61b063c40a9189020d5500858d5863b64fa6fa775451b2fc1d6b503ef3`
before and after. These results belong to the starting checkout, not later edits.
Logs: `/tmp/semantic-compiler-baseline-selected.log` and
`/tmp/semantic-compiler-baseline-postgres.log` on the local verifier host.

No live model, live database, workspace all-feature gate or production load
measurement was run at this starting point. `compiler-mvp-release-audit.md`
records the narrow profile's earlier detailed evidence.

## Section-to-package map

| Architecture section | Current foundation | Remaining implementation and evidence | Owner | Status |
| --- | --- | --- | --- | --- |
| §1 Decision and scope | Catalog, typed proposal/binding, relational lowering, SQL/direct paths exist | Check all four stage boundaries, accepted guarantees and exclusions across every added profile | P00/P21 | Partial |
| §2 Architecture and boundaries | Compiler orchestration and snapshot references exist | Complete typed artifact/analysis boundaries and source/runtime integrations | P04/P08/P15/P17 | Partial |
| §3 Lossless normalization | Source archives, maps, provenance and capability states exist | New executable importer families must preserve exact values, scope, negative meaning and unsupported extensions | P03/P17 | Partial |
| §4 Catalog IR | Immutable revisions, fact authority/conflicts, governed metrics and relationships exist | Entity/grain/semantic types, concepts/functions, advanced metrics, temporal/allocation and typed view facts | P03/P06/P10–P13 | Partial |
| §5 Catalog lifecycle | Atomic publication, reverse dependencies, search updates, optional deferred backend exist | Real configured deferred path, selected provider resolution, topological project-view work, retention/race evidence | P08/P16 | Partial |
| §6 Context selection | Full/retrieved/auto context, lexical search, hydration, entity-key fact closure and bounds exist | Other new fact families, alternatives, whole-budget accounting and sufficient-context evaluation | P07/P18 | Partial |
| §7 Intent IR | Exact original text, row/graph requirement spans and dispositions exist | Rich roles/alternatives/defaults, prepared parameters, new operation evidence and held-out interpretation tests | P02/P05/P18 | Partial |
| §8 Binding and Bound Query | Strict metric/ratio/window/lookup/composition profiles exist | Typed compatibility, concepts/views, advanced grain/relationships, conversion, allocation and parameter rechecks | P03/P05/P06/P10–P13 | Partial |
| §9 Relational IR | Versioned internal row plan, checked graph operators and a bounded graph conditional scalar profile exist | Broader typed scalar expression boundary, pass/analysis ownership and rewrite verification | P02/P04/P05/P12/P14 | Partial |
| §10 Backend planning and SQL | DataFusion and SQL AST paths; conservative PostgreSQL subset exist | New function/capability profiles, complete execution validity and live conformance for claims made | P15; each semantic slice | Partial |
| §11 Bounded orchestration | Work limits, admission, cancellation and outcomes exist | Richer outcome/repair evidence, resource accounting and concurrency/capture stress tests | P05/P09/P14/P19 | Partial |
| §12 Worked examples | Nested active/deep views, checked decimal net revenue by observed UTC month/current billing region, distinct billing/shipping customer roles, and fail-closed order/item fan-out have independent fixtures | Final example audit and source-enforced uniqueness evidence remain | P06/P11–P13 | Partial |
| §13 Interfaces | Embedding, CLI and HTTP compiler entry points exist | Current clients/proposals/prompts and importers updated with every enabled capability | P05/P17; each semantic slice | Partial |
| §14 Versioning/cache/execution | MVP execution profile, full-snapshot row cache, replay and authorization checks exist | Graph cache, dependency-aware reuse, parameter/binding/profile validity and explicit obsolete-artifact rejection | P09/P15/P16 | Partial |
| §15 Diagnostics/observation | Compiler records, digests, metrics, tracing and bounded capture exist | Rule/pass records, safe diagnostics, replay compatibility, queue/resource/telemetry accounting and overhead evidence | P14/P19 | Partial |
| §16 Evaluation | Independent MVP expected rows and small scale checks exist | Held-out interpretation/retrieval, distinguishing advanced fixtures, repeated load/scale and measured gates | P01/P18/P19 | Partial |
| §17 Delivery path | P0 implementation supersedes the old current-repository description | Execute plan with file leases and one verifier; re-audit full implementation | P00–P21 | Active |
| §18 Alternatives | Several conditional choices are named | Revisit recorded deferrals when measured evidence or a consumer appears | P20/P21 | Conditional decisions recorded below |
| §19 References | Research inputs, no code contract | No implementation gate; revisit only for selected optional choices | P20 | Reference |

## Package verification queue

| Package | Implementation checkpoint | Verification | Result |
| --- | --- | --- | --- |
| P00 | Baseline and section inventory | Selected offline P0 and PostgreSQL library tests | Baseline passed, 166 tests total |
| P01 | Seeded independent graph-calculation, MVP acceptance and scripted held-out expected rows | Graph-calculation and MVP acceptance suites passed at earlier checkpoints; eight scripted held-out cases passed after batch 25 | Partial: wider distinguishing fixtures and live interpretation set pending |
| P02 | Exact post-composition ratio, downstream filter, unit bridge and graph evidence | Six dedicated graph-calculation tests and 51 existing typed compiler tests passed on verifier batch 3 | Functional first slice passed; broader profiles remain |
| P03 | Catalog unit/entity/grain facts, metric-to-slot bridge, typed metric/ratio and numeric field units, scoped grains, field comparison/reference/calendar/enum metadata and relation-scoped authored entity-key evidence | Batches 37–45 semantic type/unit/grain/reference/calendar/enum, batch 49 entity-key, batch 51 entity-grained metric and batch 53 field-unit import/publication/graph tests passed | Authored key evidence is preserved without claiming source enforcement; provider-issued constraint/verification path and wider entity metadata remain |
| P04 | Fixed row relational verifier with pass/analysis transfer, grouped type/nullability and right-predicate lineage, graph per-node contracts, checked graph Filter/Ratio/Conditional/Cast/NullTest/CompareSlots and unit-aware Set, row output Filter and weighted zero finalizer scalar IR | Batches 37–45 graph/verifier/scalar/finalizer, batch 50 conditional, batch 53 field-unit quotient/set, batch 54 checked cast, batch 55 checked null test, batch 56 slot comparison and batch 57 forged right-predicate instance rejection passed | Checked graph and a named row pass boundary passed; broader scalar and analysis ownership remain |
| P08 | Project view dependency queue and opt-in configured deferred read-only loader | Three deferred-loader and five project-view tests passed on verifier batch 3 | Functional first slice passed; runtime read/write contracts and app entry points remain |
| P05 | Structured diagnostic stage/action/recoverability | Pending | In progress |
| P09 | Graph compilation cache | Pending | In progress |
| P05 | Diagnostic taxonomy and bounded page offset/fetch | Compiler typed regression included in batch 13 (51 existing typed cases); targeted page test in batch 8 after expectation correction | Implemented first slice; final broad gate pending |
| P06 | Executable concepts with bounded typed arguments and declared competing alternatives, authored view temporal coverage, nested-view execution and exact direct-projection output lineage | Batch 5 concept catalog/compiler, batch 8 applicability/view, batch 42 nested-view rows, batch 55 parameterized concepts, batch 56 alternatives and batch 57 canonical SQL/source-revision lineage publication/binding/SQL-direct fixtures passed | Direct one-source lineage is proved only for its exact canonical profile; arbitrary view SQL remains opaque, and equivalence/implication is pending |
| P07 | Whole context facts/dependency manifest audit, including entity identity, declared governance metadata, pinned rendered relation/field/search/lineage contracts and per-call model byte envelope | Batches 8/11, 51–52 and 55–56 context/retrieval fixtures plus batch 57 pinned view-lineage omission, Auto envelope fallback and repair-conversation limits passed | Host byte accounting is bounded; provider token-window accounting, held-out sufficiency evaluation and broader retrieval alternatives pending |
| P08 | Configured deferred read-only lifecycle in CLI/server with bounded cold-key admission | Batch 7 CLI/server/sources/publication and batch 52 engine/sources cold overload, coalescing, cancellation, failure and eviction tests passed | Implemented first slice; production connectors and broader metadata lifecycle pending |
| P09 | Bounded graph cache/coalescing | Two graph-cache tests passed in batch 5 | Implemented first slice; scale gate pending |
| P10 | Exact sum/count mean, weighted mean, exact Int64 distinct, latest-time snapshot, and strict grouped cumulative ROWS frame | Batches 11/24/28/29 state suites and batch 31 cumulative SQL/direct tests passed | Bounded executable profiles passed; wider state/window algebra remains unsupported |
| P11 | Authored relationship path and two-hop guarded lookup | Batch 17 first-hop, batch 25 second-hop temporal, and batch 36 both-hop/nullable-contract SQL/direct cases passed | Bounded two-hop as-of UTC/Date32 profile passed with nullable authored-time guards; wider path algebra pending |
| P12 | Exact rational conversion with typed source/target units, UTC month, dated currency rate, bounded fill, authored business/fiscal mapping and shared typed Values operand | Batches 18, 22, 28, 29 and 38 conversion/calendar cases, batch 45 §12.2 expected decimal revenue by month/current region, and batch 54 typed-unit conversion/source-field compatibility passed | Date32 and UTC-instant IANA local-date mapping executable; wider calendar profiles remain conditional |
| P13 | Authored allocation, checked helper, engine functions and typed SQL/direct fan-out allocation | Batches 18/21, batch 33 composite keys and batch 34 two-target suites passed | One to four exact source keys and one or two typed target dimensions passed; wider allocation algebra remains bounded |
| P14 | Versioned stage decisions, replay divergence, retained-IR integrity, bounded observation queue, checked-artifact Debug privacy and lifecycle metrics | Batches 5/18/33 replay/observation, batches 46–47 Debug leak, and batch 49 typed route/in-flight/cancellation metrics regressions passed | Checked artifact/capture Debug is redacted; explicit serialization remains sensitive, and broader observation gates remain |
| P16 | Bounded dependency-aware analysis cache and per-Compiler context cache, with cross-publication selection and conservative bound-row reuse | Batches 17/38 cache cases, batch 46 active-key admission, and batch 47 current-snapshot binding reuse/competitor/page-key regressions passed | Only eligible single-source bindings reuse; broader dependency hooks and measured savings remain |
| P17 | Strict Ossie `SEMANTIC_DB` executable profile, guarded relationship/reference/enum/calendar/entity/comparison/unit/conversion/direct-view-lineage importers, public schema-only loader and prepared HTTP/CLI entry points | Batches 17/33/34/36, batch 46 reference, batch 47 enum-mapping, batch 48 calendar, batch 50 entity-key, batch 51 entity-grained metric, batch 52 binary-exact comparison, batch 53 field-unit, batch 54 typed conversion and batch 58 canonical nested view-lineage import→publish→SQL/direct/rejection fixtures passed | Standard foreign-key cardinality stays unknown until enforceable evidence; wider importer families and public capability parity pending |
| P15 | Local SQL artifact target/profile/parameters/output schema and current-snapshot execution checks | Batch 20 compiler execution-validity suite passed | Local execution validity slice passed; remote backend and live conformance gates pending |
| P18 | Independent scripted held-out rows, calendar/policy/grain and refusal/clarification cases | Batch 31 scripted runner matched 15/15 mode outcomes with 17/17 required-fact recall | Scripted orchestration only; live-model accuracy remains unmeasured (credentials absent) |
| P19 | Opt-in deterministic compiler, catalog scale, dependency-shape, large-prose, concurrency, cancellation and observation-overhead harnesses in `tests/performance` | Batches 27/30/31/32/33, batch 34 million-field, batch 36 post-cache wide, batch 37 dependency shapes, batch 38 large prose, batch 40 gated cancellation and batches 48–49 cache/lifecycle metrics tests | Host-scale evidence only; wide relation warm p50 improved from 91.6 to 1.9 ms, while wider admission/live-model measurements remain pending |
| P20 | Conditional extension decisions recorded below | No optional extension selected without a consumer or measured benefit | Decisions recorded; revisit at P21 |

Add rows for later packages when started. Every verified result must identify the
source contents, exact command, feature set, expected independent outcome and
unrun environment-dependent checks. The complete clause audit is a P21 gate.

Batch 3 source manifest: 397 tracked/untracked inputs, SHA-256
`5a2d57397fb2a04fea02a2c13e545ae4919b5b3ba7f608fc77bdf480395615ba`.
Commands used `--locked --offline`; full logs are
`/tmp/semantic-compiler-batch3-{graph,deferred,views,typed}.log` on the local
verifier host. `cargo fmt --all -- --check` found formatting changes in the new
agent-owned files, so formatting is not yet green for this batch. No broader
workspace/Clippy/live gate is claimed.

## Focused verification through batch 13

All commands below used `--locked --offline`; the sole verifier recorded unchanged
source manifests before and after each batch. Batch 11 used 427 tracked/untracked
inputs, SHA-256
`3417959f7fa7d22d0c1d4d7ee654e54b81592c0a8234f76d6444614fe07fcb5f`;
74 focused tests passed across catalog, engine, compiler, and Ossie, and
`cargo fmt --all -- --check` passed. Commands were `cargo test -p
semantic-catalog --test conversion --test relationship_path`, `cargo test -p
semantic-engine --test conversion --test checked_mean`, `cargo test -p
semantic-compiler --test metric_state_mean --test metric_state_guard --test
typed_path --test typed --test context_manifest`, and `cargo test -p
semantic-ossie --test executable_profile`. Logs are
`/tmp/semantic-compiler-batch11-{catalog,engine,compiler,ossie,fmt}.log`.

Batch 13 used 432 tracked/untracked inputs, SHA-256
`0d5aaeca70b68d4a7d4f75be45f87052e4d7a9eec6a5dd0811a90fc12bac7e5b`.
The compiler suite (`typed_conversion`, `typed`, `typed_path`,
`dependency_cache`) passed 60 tests; Ossie executable-profile passed four;
catalog conversion passed two; formatting passed. Catalog allocation passed
three of four tests, with one independent overflow fixture still being
corrected. Logs are `/tmp/semantic-compiler-batch13-{compiler,catalog,catalog-conversion,ossie,fmt}.log`.
No workspace-wide, all-feature, live PostgreSQL, live model, or measured load
gate is claimed for these snapshots.

Batch 15 used 434 tracked/untracked inputs, SHA-256
`e77adedabe0e7fcb170964ae9d2a93cdc8e8e45e02ccacfe49829d97712cd6b2`.
The UTC-month engine library case and four catalog allocation cases passed;
Ossie executable profile passed four; compiler dependency-cache passed three,
the existing typed suite 52, conversion two, and three non-temporal path cases.
The combined compiler command was blocked by a test fixture accessing a private
field (since corrected); one temporal-path case reached backend validation and
is under investigation. Formatting passed. Logs are
`/tmp/semantic-compiler-batch15-{engine-calendar,catalog-allocation,compiler,compiler-independent,ossie,fmt}.log`.

Batch 17 used 435 tracked/untracked inputs, SHA-256
`1fe65bf79ad2c9d93e6ba55f9126ed1739e144e9a5537c50dc5ded5406a5dfc8`.
All focused targets passed: compiler 63 tests including temporal two-hop SQL/direct
execution and context-cache reuse, sources public Ossie load two, engine
conversion two and calendar one, catalog allocation/conversion six. Formatting
passed. Logs are `/tmp/semantic-compiler-batch17-{compiler,sources-ossie,engine,engine-conversion,catalog,fmt}.log`.

Batch 18 used 440 tracked/untracked inputs, SHA-256
`5b5945cadca2fb3c4d8307961ed0f939a6ae8d59bc3b7fad8031e6bce8986a31`.
All 75 focused tests passed: engine allocation/conversion five, compiler
observation/context/calendar/conversion/path/typed 64, catalog
allocation/conversion six. Formatting passed. Logs are
`/tmp/semantic-compiler-batch18-{engine,compiler,catalog,fmt}.log`.
The later focused `typed_calendar_group` check passed two independent boundary
cases at source manifest
`988bdd86cd63c589da64b987b2f5eff0d221236a7ae71616e923bb2f7044e219`.
Its concurrent in-progress allocation files still needed formatting, which was
subsequently corrected before batch 18.

Batch 20 used 447 tracked/untracked inputs, SHA-256
`65f0bf83ff206f28fba637ddf03846ba33945f9515e09acae2efdd1ae56bbaf2`.
Compiler `typed_allocation`, `context_manifest`, `execution_validity`, and
`held_out_interpretation` passed ten focused tests; formatting passed. Batch 21
used 449 inputs, SHA-256
`39f5ca897a6311a2791d0eb3524a54e5e7f78b47a639d02c2e83cdf0fc8090c5`.
Catalog `currency_rate` and `allocation_publication` passed four tests;
compiler `context_manifest`, `typed_allocation`, and
`held_out_interpretation` passed nine; formatting passed. Both manifests were
unchanged before/after the verifier's runs. Logs are
`/tmp/semantic-compiler-batch20-{compiler,fmt}.log` and
`/tmp/semantic-compiler-batch21-{catalog,compiler,fmt}.log`. Allocation
SQL/direct execution includes a policy-visible duplicate source-key guard;
these focused cases passed in batch 21.

Batch 22 retry used 456 inputs, SHA-256
`42c75ed88cfc6a703e8f6ca6543670ead7cfb8afa9c6f4be46eb3524bc6c6fec`.
Currency conversion and calendar spine focused tests passed, as did a locked
check of the deterministic compiler benchmark and formatting. Cargo.lock was
updated by the sole verifier for the performance package dependencies before
this frozen run. Batch 24's catalog business-calendar and metric-state tests
passed nine; checked weighted-average engine plus library passed eleven.
Batch 25 used 464 inputs, SHA-256
`3d2fa3c9803e065ae5e14b56a094099ffa7071ea3c25786d787fd0e0faf90ede`.
The second-hop temporal path passed five focused SQL/direct cases, observed
calendar grouping passed two, and the engine weighted tests passed eleven.
The first scripted held-out runner produced seven of eight matching outcomes,
which exposed a retrieved-context hydration false refusal. The runner was
subsequently fixed to allow a bounded second model call and to fail its scripted
quality gate; a later frozen retry passed all eight cases with no outcome,
diagnostic or row mismatches. At source manifest
`c3ddf16b60cabe92a0c4bd1dd95e8b58962bc928c0157ed91ed805dc696bc2e0`,
weighted compiler SQL/direct passed two, the scripted runner passed eight, and
the catalog scale harness checked. Logs are
`/tmp/semantic-compiler-batch25-{catalog,compiler-stable,engine,held-out}.log`,
`/tmp/semantic-compiler-weighted-retry.log`, and
`/tmp/semantic-compiler-heldout-retry.{json,log}`. Calendar fill remains under
repair after a backend alias error at this historical checkpoint.

Batch 27 used 467 inputs, SHA-256
`b7a499121edd7da938cff0a74fe6c80ad07b361ba4a558dad8aba0fae7673357`.
Compiler calendar-fill and dependency-cache tests passed six; checked-distinct
engine and library tests passed twelve; formatting passed. Catalog scale at
100 relations and 100 fields per relation gave eight coherent readers, 282.66 ms
publication, 1902.99 ms index construction and 2.57 ms one-relation change, with
one relation affected. This is one host run, not a percentile or a production
capacity claim. The first compiler-scale 100×100 run exposed an overstrict
harness assertion about exactly three context fields; that assertion was removed
so later runs report extra context rather than hiding it. Logs are
`/tmp/semantic-compiler-batch27-{compiler,engine,fmt,catalog-scale,compiler-scale}.log`.

Batch 28 used 473 inputs, SHA-256
`030e928871d9fa84697b3bde80df7e9aa826228531640a4c4ff675e4cc459b71`.
The verifier updated the lockfile for the already cached `chrono-tz` catalog
dependency before the frozen run. Catalog business-calendar, currency-rate and
metric-state publication passed eight tests; compiler business-calendar,
calendar-fill, currency-rate, exact-distinct state and fail-closed state guard
passed nine; engine checked-distinct and library passed twelve. Formatting
passed, and the source manifest was unchanged before/after each frozen command.
Logs are `/tmp/semantic-compiler-batch28-{catalog,compiler,engine,fmt}.log`.
No broad workspace, Clippy, live database, live model or full load gate is
claimed for these snapshots.

Batch 29 used 476 inputs, SHA-256
`225364dfdcc63e2485ce10f6bb69f9ad7b6aa5675c571783a0776984b0ac9a91`.
The sole verifier first updated only `Cargo.lock` for the cached engine
`chrono-tz` dependency with `cargo check --offline -p semantic-engine`. Frozen
SnapshotBalance engine and compiler tests passed two each; UTC-instant business
calendar compiler tests passed three, existing Date32 calendar tests passed two,
and formatting passed. Engine library failed one hard-coded function-count
assertion after registering the sixteenth local function. The expanded fake
remote test failed an overstrict expectation that a local-only weighted
aggregate must also issue a remote read; it can use the adaptor's local fallback.
Neither failure indicated a mismatch in expected result rows. Logs are
`/tmp/semantic-compiler-batch29-{unlocked-engine-check,engine-snapshot,engine-functions,engine-lib,compiler-snapshot,compiler-calendar-utc,compiler-calendar,fmt}.log`.

Batch 30 used 476 inputs, SHA-256
`218724b1a9a17683525ceb9b1d87f64d0c0160f5068b5947425862e3b1da99ab`.
After correcting those assertions, engine library passed 13, compiler-function
fake-remote tests passed four, and formatting passed. The deterministic
`compiler_scale` host probe at 100 relations × 100 fields used seven measured
runs after two warmups: cold compile 8.824 ms, warm p50 2.701 ms and sample
p95/p99 2.727 ms. Each warm compile looked up three requested fields and
rendered four context fields; the extra field is explicit in the JSON report.
Index construction took 2129.824 ms for 10,100 charged objects. RSS was not
available under this sandbox. JSON and log are
`/tmp/semantic-compiler-batch30-compiler-scale.{json,log}`; test logs are
`/tmp/semantic-compiler-batch30-{engine-lib,engine-functions,fmt}.log`.
These are local single-host observations with a deterministic proposal, not
live-model or production latency evidence.

Batch 31 used 478 inputs, SHA-256
`7373fa133a7dd568088d6a967c309e2a28229e692b74b7a9c95629c0b03b115f`.
The compiler's existing typed suite passed 52, new strict grouped cumulative
ROWS tests passed two, and library tests passed seven including the relational
version/comparison-profile mutation. The engine fake-remote suite passed four,
including local SnapshotBalance and local-date behavior. Formatting passed.
The expanded scripted held-out runner matched 15/15 mode outcomes with zero
incorrect accepts or false refusals and recalled 17/17 required facts; it still
does not measure a model's interpretation. The compiler host probe at 1,000
relations × 100 fields used seven measured runs after two warmups: index
construction 23,766.254 ms for 101,000 charged objects, cold compile 8.641 ms,
warm p50 2.697 ms and sample p95/p99 2.744 ms. Every warm run looked up three
fields, rendered four context fields and made one deterministic provider call.
Logs and machine-readable results are under
`/tmp/semantic-compiler-batch31-{compiler-typed,compiler-lib,engine-functions,held-out,fmt,compiler-scale}`.
The frozen manifest was unchanged before/after. Live model credentials were
absent in the local environment; no live quality result is claimed.

Batch 32 used 482 inputs, SHA-256
`398b7b4db8f45e9c44480a805141700a7a2a773d112a8fa853444346be843f54`.
The sole verifier updated `Cargo.lock` only for the performance crate's new
`semantic-plan` dependency with an unlocked offline check; its resulting SHA-256
was `bcc90368c3fadbb4ccec6cf2544f7f5103cccc8315e7932338fd4796c50d1b5a`.
Compiler verifier library tests passed four, nine compiler integration targets
passed 79, catalog concepts passed one, Ossie executable profile passed four,
and formatting passed. The eight-client deterministic concurrency probe passed:
seven hits and one miss on the cold same-key wave, 63 hits and one cumulative
miss after seven warm rounds, and five evictions under distinct-key pressure
while retaining four entries and 7,344 bytes. Cold p50 was 11.165 ms and warm
p50 was 1.612 ms. The full locked offline workspace check across all targets
and features passed in 1m46s. The frozen source manifest matched before/after.
Logs are `/tmp/semantic-compiler-batch32-{unlocked-check,compiler-verify,compiler-targets,catalog-concepts,ossie-profile,fmt,workspace-check}.log`;
the concurrency result is `/tmp/semantic-compiler-batch32-concurrency.json`.

Batch 33 used 485 inputs, SHA-256
`d889242a1d5d4486b9420e8f9d241f717ca19f50f56cff09c54096d38f7d5964`.
Composite-source-key allocation passed three new SQL/direct cases and four
existing cases; compiler typed regression passed 52 and library passed ten.
Prepared HTTP passed two integration and one admission unit case; existing
server typed tests passed three. Stage replay passed three, including retained
IR and transition-digest integrity. The deterministic observation probe ran
100 measured repetitions after ten warmups per mode with the same semantic
artifact digest: disabled and normal p50/p95 were each 0.625/0.685 ms; debug
with retained IR p50/p95 was 1.161/1.246 ms. Its bounded queue reported 110
drops while compilation continued. Formatting, diff check and full locked
offline workspace check across all targets/features passed. The frozen source
manifest was identical before/after; no lockfile change was needed. Logs are
`/tmp/semantic-compiler-batch33-{compiler-targets,compiler-lib,server-tests,server-http-unit,observation-check,observation,fmt,diff-check,workspace-check}.log`;
JSON is `/tmp/semantic-compiler-batch33-observation.json`.

Batch 34 used 487 inputs, SHA-256
`e432dc7abf757e033adf7b97e1d641bba2e8314d0e816daac3d158a75f2606c6`.
The sole verifier updated only `Cargo.lock` for the CLI's new prepared-query
dependencies (resulting SHA-256
`e325968a92290655aa67807af928b25326f631b8242f1bce00277b4e5b642e07`).
Seven compiler integration targets passed 69, compiler library passed ten,
CLI command/prepared targets passed 5/2, and formatting/diff checks passed.
The locked offline workspace check across all targets/features passed. The
two-target allocation fixture validated exact SQL/direct output and guarded
missing, policy-filtered and duplicate full target tuples. A 10,000-field
single relation compiled with warm p50 91.662 ms after 5.048 s index build;
a 10,000-relation × 100-field million-field catalog compiled with warm p50
2.833 ms after 53.793 s engine construction and 264.141 s index build.
Both selected three fields and rendered four context fields. RSS was unavailable
under this sandbox. The frozen source manifest matched before/after. Logs and
JSON are under `/tmp/semantic-compiler-batch34-{wide-scale,million-scale}`;
gate logs are `/tmp/semantic-compiler-batch34-{unlocked-cli-check,compiler-targets,compiler-lib,cli-tests,fmt,diff-check,workspace-check}.log`.

Batch 35 used 487 inputs, SHA-256
`36e0ac37d8893fbfdd93c6128a20f057ceffb6dff338cae6bc8683bd5f60c7d3`;
the manifest matched before/after. Compiler observation/diagnostic focused tests
passed 3/2; the sandbox-compatible compiler suite passed 159, catalog+engine
passed 102, and sources passed 30. The wide 10,000-field stage probe confirmed
backend planning dominated warm latency: one sample recorded 95.881 ms backend
of 97.572 ms total, versus 61 µs context, 24 µs bind and 9 µs lower.
Two Ossie test issues remained at this checkpoint: a new expected-row type
annotation was ambiguous (E0283), and an older test expected
`unsupported_feature` where the strict executable metric importer now returns
`invalid_extension`. Full workspace all-target checks were consequently blocked
by the Ossie compile error; the no-default-feature workspace library check,
formatting and diff checks passed. CLI, server and OpenAI tests that bind a
loopback port encountered sandbox `PermissionDenied`; this is an environment
restriction, not a demonstrated semantic failure. The verifier will rerun the
affected gates after repair and attempt an auto-reviewed unsandboxed loopback
run. Logs are `/tmp/semantic-compiler-batch35-{ossie-focused,ossie-import,compiler-nosocket,catalog-engine,sources,cli,server,fmt,diff-check}.log`;
wide JSON is `/tmp/semantic-compiler-batch35-wide-scale.json`.

Batch 36 used 489 inputs, SHA-256
`9ee9cf4769eb0b51d483d938f5b17ebebef4b50373a870ee16fb422ff122deb0`;
the manifest matched before/after. Ossie executable/import tests passed 7/11,
engine generated-plan-cache/query tests passed seven, compiler path/temporal
contract tests passed seven, and compiler library passed ten. Broad engine+Ossie
passed 76; broad compiler without the three sandbox-blocked loopback tests
passed 161. Auto-review approved unsandboxed loopback runs: OpenAI target passed
four, CLI LLM/server-files passed four with one intentionally ignored live-API
case, and server compiler/protocol passed four. Complete CLI+server broad suites
passed 61 with one intentionally ignored live-API case. Both locked offline
workspace all-targets checks (all-features and no-default-features), formatting
and diff checks passed. The wide 10,000-field probe after per-engine generated
logical-plan reuse had warm p50/p95 1.923/2.098 ms, versus batch 35's
91.637/97.576 ms; one warm backend stage fell from 95.881 to 0.247 ms.
Cold backend planning remained 97.834 ms. Logs and wide JSON are under
`/tmp/semantic-compiler-batch36-*`; loopback escalations were approved and
passed, with no auto-review rejection.

Batch 37 used 491 tracked and non-ignored untracked inputs, SHA-256
`ab9cf099fedaebeb4ce46cbd3dae6f6852cd5b7b81edb5231bc2fab80894decf`;
the source manifest matched before and after. Focused relational-verifier tests
passed 5/5, graph calculation and typed diagnostics 9/9, and catalog semantic
types 3/3. The complete compiler/catalog/plan/engine offline library and test
sweep passed 270/270 across 67 binaries, including an approved unsandboxed
loopback OpenAI lane. Both locked offline workspace all-targets checks
(all-features and no-default-features), formatting, and diff checks passed.
This snapshot adds a versioned fixed analysis transfer contract and mutation
checks, requested exact ratio-unit validation, and deterministic catalog
dependency probes. The probes changed a shared root in chain, diamond and hub
graphs while preserving an unrelated relation. Affected relations were 9/10,
25/26 and 33/34 respectively; each incremental index charged three objects.
The probes establish dependency correctness on these shapes, not production
latency. Logs and JSON are under `/tmp/semantic-compiler-batch37-*`.

Batch 38's initial frozen build stopped on a mutable-binding compile error in
the new Values SQL emitter; no downstream gate was claimed for that attempt.
After repair, the retry used 494 inputs, SHA-256
`ac4e46790d9a7537d941801e751a521ad0d672022a65099a714e2c230e989939`;
the manifest matched before and after. Compiler library Values and context
tests passed 13/13, focused calendar/context integration tests 7/7, and the
complete compiler/catalog/plan/engine offline library/test sweep passed
275/275 across 68 binaries, including an approved unsandboxed loopback lane.
Both locked offline workspace all-targets checks (all-features and
no-default-features), formatting and diff checks passed. A 16-relation,
16-field, 4,096-byte-minimum-description probe compiled with three fields
looked up and a 5,755-byte context. Its seven warm samples after two warmups
had p50 1.934 ms and sample p95 2.045 ms. This is a local bounded-prose
probe, not a model or service measurement. Logs and JSON are under
`/tmp/semantic-compiler-batch38-retry-*`.

Batch 39's initial frozen build stopped on a local variable shadowing a test
query builder; no broad gate was claimed for that attempt. The repaired retry
used 495 inputs, SHA-256
`2d0f6c3d43caf4f893cb19fef5d2cf8625a7601c90eca66e81c2da37c343f5bb`,
unchanged before and after. Focused comparison/diagnostic cases passed 3/3,
and the grouped-output type/nullability mutation case passed 1/1. The broad
compiler/catalog/plan/engine/Ossie library and test sweep passed 298/298
across 72 binaries, including the approved loopback lane. Both locked offline
workspace all-targets checks (all-features and no-default-features), formatting
and diff checks passed. Logs are `/tmp/semantic-compiler-batch39-retry-*`.

Batch 40 used 496 inputs, SHA-256
`921a415c6e2405ddb54e225188f286d7bcedb8d0b2fa4d728c59bd37f82551dc`;
the manifest matched before and after. Focused graph output-contract mutation
passed 1/1, compiler acceptance/typed/prepared/state/held-out suites 82/82,
catalog 14/14, Ossie 7/7 and sources Ossie 2/2. The complete six-crate
compiler/catalog/plan/engine/Ossie/sources library and test sweep passed
330/330 across 80 binaries, including approved local loopback tests. Both
locked offline workspace all-targets checks (all-features and
no-default-features), formatting and diff checks passed. This snapshot
replaces metric/ratio unit strings with tagged semantic units, checks requested
unit kind exactly, and rejects old string units in strict Ossie extensions.
An opt-in eight-client gated-provider probe cancelled four while four later
compiled; it measured 0.085 ms for cancellation completion and survivor p50
16.198 ms on this host. This is not admission, live model, or service latency
evidence. Logs and JSON are under `/tmp/semantic-compiler-batch40-*`.

Batch 41 used 497 inputs, SHA-256
`590232383c73490abd9ef4323e61d94e65ab363e49eedbe9e600931dea9f6be2`;
the manifest matched before and after. Graph output-contract focused tests passed
2/2, scoped-grain and related compiler integration targets 76/76, and catalog
semantic-type tests 3/3. The scripted held-out example again matched 15/15
mode outcomes with no incorrect accept or false refusal and recalled 17/17
required facts; it does not score live interpretation. The broad six-crate
compiler/catalog/plan/engine/Ossie/sources sweep passed 332/332 across 81
binaries, including approved local loopback tests. Both locked offline
workspace all-targets checks (all-features and no-default-features), formatting
and diff checks passed. The compiler now checks every graph node's direct
schema against its bound slots, and a requested grain identifies the relation
for each key. Logs and held-out JSON are under `/tmp/semantic-compiler-batch41-*`.

Batch 42 used 500 inputs, SHA-256
`c7e3d051954b33af9cef33368c740a4659b3cb6eab6d02b765544d1f8e8853fb`;
the manifest matched before and after. Focused scalar graph, nested view,
scoped grain, typed/MVP/calculation/prepared/state compiler targets passed
84/84; catalog publication/state 12/12; Ossie 7/7; sources Ossie 2/2.
The scripted held-out example again matched 15/15 outcomes with 17/17
required facts recalled; it is not live-model scoring. The broad six-crate
library/test sweep passed 336/336 across 83 binaries, including approved
local loopback tests. Both locked offline workspace all-targets checks
(all-features and no-default-features), formatting and diff checks passed.
This snapshot makes metric source grains relation-scoped and checks them at
publication/import/binding; it also gives graph filters one checked Boolean
expression path for SQL and direct planning. A nested authored-view fixture
returned the expected two well IDs through both plans. Logs and held-out JSON
are under `/tmp/semantic-compiler-batch42-*`.

Batch 43 used 503 inputs, SHA-256
`fe37b64296ffb5d3000591c7a36402107105032a9ad2532db47f4aa323d086ad`;
the manifest matched before and after. Focused compiler targets passed 72/72,
compiler library 16/16, catalog reference/path/publication 15/15, and Ossie
executable profile 7/7. The broad six-crate library/test sweep passed 341/341
across 86 binaries, including approved local loopback. Both locked offline
workspace all-targets checks (all-features and no-default-features), formatting
and diff checks passed. This snapshot checks authored relationship-key reference
systems at publication and binding, and lowers graph ratios through one
versioned checked scalar call for SQL and direct execution. Logs are under
`/tmp/semantic-compiler-batch43-*`.

Batch 44 used 506 inputs, SHA-256
`a78a09c336788711a13aa142f2b216fbf8376f4e79947054bc13e990b5ded74e`;
the manifest matched before and after. Focused compiler integration targets
passed 61/61 plus lower verifier 7/7; catalog calendar/semantic/publication
passed 13/13; Ossie executable profile passed 7/7. The broad six-crate
library/test sweep passed 346/346 across 89 binaries, including approved local
loopback. Both locked offline workspace all-targets checks (all-features and
no-default-features), formatting and diff checks passed. This snapshot validates
authored calendar references and checks Gregorian UTC month grouping against
independent SQL/direct rows; fiscal meaning rejects that profile. Row output
filters now use checked Boolean scalar IR with exact type and stage verification.
Logs are under `/tmp/semantic-compiler-batch44-*`.

Batch 45's first frozen snapshot had 510 inputs, SHA-256
`abea0bd1f0eeb236b07dda4ddb3c4c4565d14eebfc448c427066a59eef36e8d7`.
Catalog enum/calendar/publication passed 11/11 and formatting/diff passed, but
compiler construction stopped on two new aggregation fields omitted in lookup
lowering; no broader gate was claimed for that snapshot. The repaired retry used
510 inputs, SHA-256
`eb7e794c9a432404e3d0d83024aab414e51aa7a348735d3b6455274320263281`,
unchanged before and after. Focused compiler passed 63/63 and lower verifier
8/8; Ossie executable profile passed 7/7. Broad six-crate library/test coverage
passed 352/352 across 93 binaries, including approved local loopback. Both
locked offline workspace all-targets checks (all-features and
no-default-features), formatting and diff checks passed. The §12.2 fixture
asserts four independent month/region Decimal128 totals against SQL and direct
execution, omits empty February and filtered April, retains an unmatched
customer under null region, and rejects an unauthored historical-region role.
Same-typed enum codes now require their authored domain mappings, while the
weighted zero-result finalizer is one checked scalar contract for SQL and direct
plans. Logs are under `/tmp/semantic-compiler-batch45-retry-*`; the first
failed attempt is `/tmp/semantic-compiler-batch45-compiler-focused.log`.

Batch 46 used 510 inputs, SHA-256
`5e1ece431be7fa3b9c4822c78ff589fec4df5ad9d4e491f877f0b79e2ff5e21a`;
the manifest matched before and after. Focused Ossie executable/import passed
19/19, sources Ossie 2/2, compiler cache/context/observation/reference/diagnostic
15/15 plus library 18/18, and catalog reference 2/2. The affected broad
compiler/Ossie/sources/catalog library/test sweep passed 300/300 across 78
binaries, including approved local loopback. Both locked offline workspace
all-targets checks (all-features and no-default-features), formatting and diff
checks passed. This snapshot imports exact field reference systems through a
strict Ossie extension and rejects mismatched keys before lookup execution.
The analysis cache now limits distinct active build keys while coalescing same-key
work, and direct row/graph artifact `Debug` output omits sensitive request and
SQL content. Logs are under `/tmp/semantic-compiler-batch46-*`.

Batch 47's repaired snapshot used 511 inputs, SHA-256
`4c12dc1cf61c88c0cf55e1429d785ded037960a50389d3eb17288f129c51d222`;
the manifest matched before and after. Focused compiler tests passed 20/20 and
Ossie executable-profile tests passed 21/21. The broad compiler/Ossie
library/test sweep passed 221/221 across 52 binaries. Both locked offline
workspace all-targets checks (all-features and no-default-features), formatting
and diff checks passed. This snapshot imports authored enum mappings through a
strict Ossie extension, reuses dependency-checked bound row analysis across
engine replacement while re-lowering on the current engine, and includes page
limits in row/graph artifact cache identity. Additional bound, relational and
replay `Debug` implementations omit sensitive request content. The initial
Ossie test build identified a missing direct test dependency, which was added
before the repaired snapshot. Logs are under
`/tmp/semantic-compiler-batch47-retry-*`.

Batch 48's repaired snapshot used 513 inputs, SHA-256
`4e18451aecc19ddfc94a6ff882e4051810862dd388800d6a05caff9ca65cee01`;
the manifest matched before and after. Focused compiler tests passed 9/9 and
Ossie executable-profile tests passed 13/13. The broad compiler/Ossie
library/test sweep passed 228/228 across 54 binaries. Both locked offline
workspace all-targets checks (all-features and no-default-features), formatting
and diff checks passed. The fixture proves billing and shipping roles retain
distinct customer instances and exact SQL/direct rows, while an unavailable
shipping role cannot reuse billing. A strict Ossie field extension now imports
Gregorian UTC calendar meaning and rejects malformed, duplicated and unsupported
calendar declarations, with a fiscal/Zurich mismatch rejected at binding.
Analysis-cache lifecycle tests cover active-key admission, cancelled leaders,
failed builds, bounded retention and eviction. The initial test compilation found
an ambiguous expected-row type in the new role fixture, fixed before the
repaired snapshot. Logs are under `/tmp/semantic-compiler-batch48-retry-*`.

Batch 49 used 517 inputs, SHA-256
`122c2ef94a7603ca0ffac6216d6055e8d79882193154894fc0aa1ece1730d0c1`;
the manifest matched before and after. Focused catalog tests passed 3/3,
compiler tests 9/9 and the existing aggregate-metrics test 1/1. The broad
catalog/compiler library/test sweep passed 263/263 across 73 binaries. Both
locked offline workspace all-targets checks (all-features and
no-default-features), formatting and diff checks passed. A relation-scoped
entity-key contract retains authored identity and provenance but rejects
unauthenticated source/runtime uniqueness claims; publication checks its key
scope and declarations. Typed compiler metrics now track live/peak in-flight
requests, terminal cancellation/deadline, and abandoned tasks with one guard per
public route. The §12.3 fixture rejects an unapproved item-category dimension
and prevents duplicated order amounts at execution even when the dimension is
whitelisted. Logs are under `/tmp/semantic-compiler-batch49-*`.

Batch 50's repaired snapshot used 518 inputs, SHA-256
`a422e5b50421db67124b1c4a5f994f57206ddc6e9a0ae1ab4e5986f4d2e8454c`;
the manifest matched before and after. Focused Ossie tests passed 16/16 and
compiler tests passed 19/19. The broad compiler/Ossie library/test sweep passed
239/239 across 57 binaries. Both locked offline workspace all-targets checks
(all-features and no-default-features), formatting and diff checks passed. The
strict Ossie entity extension imports an authored scoped key with provenance but
does not upgrade unknown join cardinality. A new graph conditional node uses a
checked output predicate and same-typed literal branches; SQL and direct paths
produce independent true/false/unknown expected rows. Conservative SQL nullability
is preserved in its portable slot contract. Missing lookup-role diagnostics now
identify the request requirement and missing relationship. The first focused
snapshot exposed a SQL/direct nullability mismatch; a diagnostic frozen run
isolated it and the repaired retry passed. Logs are under
`/tmp/semantic-compiler-batch50-retry-*`.

Batch 51's repaired snapshot used 520 inputs, SHA-256
`eacf8b607ef8b4216120b1e2d7ea7d108bd983fd7ad41857ed0ae4f5b1766ab0`;
the manifest matched before and after. The publication target passed 9/9 and
the broad catalog/Ossie/compiler library/test sweep passed 304/304 across 79
binaries. The original focused catalog, Ossie and compiler gates passed 1/1,
18/18 and 13/13 before the test-only correction. Both locked offline workspace
all-targets checks (all-features and no-default-features), formatting and diff
checks passed. Entity-grained metrics now require the exact authored entity ID
and key tuple at import and publication; the imported COUNT metric produces the
independent SQL/direct result 2. Entity identity is rendered in compiler context,
its keys are dependency-closed, and omission fails the context manifest audit.
Row-binding diagnostics attach the correct requirement ID across temporal,
output-slot, window and main resolution while global input failures retain no
invented requirement. The first broad run exposed an old generic diagnostic-code
expectation in a publication test; the repaired retry passed. Logs are under
`/tmp/semantic-compiler-batch51-retry-*`.

Batch 52 used 521 inputs, SHA-256
`884ba2ce69271a877da2f3526d06702cedff11ef43d743e1f09829339e6ea0b1`;
the manifest matched before and after. Focused engine tests passed 7/7,
sources 6/6, Ossie 20/20 and compiler 8/8. The broad four-crate library/test
sweep passed 337/337 across 81 binaries. Both locked offline workspace
all-targets checks (all-features and no-default-features), formatting and diff
checks passed. Deferred provider resolution now bounds distinct active cold keys
while preserving same-table coalescing and releasing admission on cancellation,
failure and success. Ossie imports exact binary comparison meaning on String
fields, rejects unsupported collation profiles and executes a fixed SQL/direct
row fixture. The context audit now verifies declared keys and governing metadata
groups in the rendered payload. Logs are under
`/tmp/semantic-compiler-batch52-*`.

Batch 53 used 523 inputs, SHA-256
`4e6cbdb49312d4e8f05c2df1ee022e5e845b17ea4c4dceccdf56338900077cce`;
the manifest matched before and after. Focused catalog tests passed 2/2,
Ossie 22/22 and compiler 68/68. The broad catalog/Ossie/compiler library/test
sweep passed 315/315 across 81 binaries. Both locked offline workspace
all-targets checks (all-features and no-default-features), formatting and diff
checks passed. Numeric field units now publish and import under strict physical
and unit validation. Row projection/group/lookup outputs carry authored unit
facts into graph calculations, where same-unit ratios yield dimensionless
meaning, different units produce a checked quotient, and incompatible known
units cannot be aligned by a graph Set. SQL/direct expected rows and rejection
fixtures cover these paths; absent unit evidence remains unknown. Logs are under
`/tmp/semantic-compiler-batch53-*`.

Batch 54 used 524 inputs, SHA-256
`52b9da28ea4650be17f03778b0b65efe5af476f5a76607ab1e770c059d922ab9`;
the manifest matched before and after. Focused catalog tests passed 3/3,
Ossie 24/24 and compiler 21/21, with compiler examples checking successfully.
The broad catalog/Ossie/compiler library/test sweep passed 321/321 across 82
binaries. Both locked offline workspace all-targets checks (all-features and
no-default-features), formatting and diff checks passed. Checked graph Cast now
supports the bounded Int64-to-Decimal128(38,0) conversion with independent
SQL/direct expected rows at null, negative and maximum integer boundaries;
invalid scope, type, wire and evidence are rejected. Conversion source and
target units are typed, publication and binding check authored source-field
units, and strict Ossie conversion import preserves exact rational and rounding
metadata with SQL/direct and rejection fixtures. Logs are under
`/tmp/semantic-compiler-batch54-*`.

Batch 55's repaired snapshot used 525 inputs, SHA-256
`f7058db6ae29bc00f4075b6ed3b759012a8d6d53ce678d9e6d72ebfc53796037`;
the manifest matched before and after. Catalog concept tests passed 2/2 on
the original snapshot, and the repaired focused compiler run passed 37/37
across eight targets; compiler examples checked successfully. The broad
catalog/compiler library/test sweep passed 294/294 across 80 binaries. Both
locked offline workspace all-targets checks (all-features and
no-default-features), formatting and diff checks passed. Authored concepts now
bind bounded, exact typed arguments to their declared placeholders; missing,
extra and wrong-typed arguments fail before execution. Graph NullTest produces
nonnullable Boolean values with checked Int64 scope and independent SQL/direct
null and nonnull rows. The context audit rejects duplicate or unmanifested
relations, altered field contracts, fabricated relation facts and missing pinned
retrieval evidence. The first focused compiler snapshot exposed a test-helper
name shadowing error; the test-only repair and retry passed. Logs are under
`/tmp/semantic-compiler-batch55-retry-*`.

Batch 56 used 526 inputs, SHA-256
`af47ebef0fc917fff3b468282300c2b6dc163d65d91392e840e92f943b4f241f`;
the manifest matched before and after. Focused catalog concept tests passed
3/3, Ossie executable-profile 24/24 and compiler tests 51/51 across eleven
targets; compiler examples checked successfully. The broad catalog/Ossie/
compiler library/test sweep passed 337/337 across 84 binaries. Both locked
offline workspace all-targets checks (all-features and no-default-features),
formatting and diff checks passed. Authored concept alternatives now identify
bounded competing definitions: names or aliases clarify when more than one
applies, while an explicit durable ID pins the selected definition. Checked
graph CompareSlots yields Boolean true/false/unknown with exact Int64 scope and
unit compatibility in SQL and direct execution. A per-model-call host byte
envelope reserves output space and prevents provider invocation when initial
or expanded messages exceed the limit; it does not claim exact provider token
accounting. Logs are under `/tmp/semantic-compiler-batch56-*`.

Batch 57's third frozen retry used 529 inputs, SHA-256
`d00e9fe75c0c50470914176ea7dd6d97e99fdac3b82f993bd2e1b68ca4aacd56`;
the manifest matched before and after. Focused retrieval passed 8/8, other
compiler integration targets 17/17, compiler-lib right-predicate regression
1/1 and examples check green. Catalog view-lineage passed 1/1 before the
test-only retrieval fixture repairs. The broad catalog/compiler/Ossie
library/test sweep passed 343/343 across 86 binaries. Both locked offline
workspace all-targets checks (all-features and no-default-features),
formatting and diff checks passed. A view can now publish exact, complete
one-source projection lineage only when its canonical SQL, source revision,
dependency, output map, type and nullability agree; a two-level view fixture
produces independent SQL/direct expected rows. The context payload includes
and audits the pinned lineage. The row pass rejects Related/Lookup predicates
whose original field instance would be silently rebound to the right alias.
Auto context selection respects the complete per-call host byte envelope,
falling back to retrieval when full context cannot fit; a growing repair
conversation is checked again before the second provider call. The first two
focused snapshots exposed test-only helper shadowing and a stale context
assertion; the third retry passed. Logs are under
`/tmp/semantic-compiler-batch57-retry3-*`.

Batch 58 used 529 inputs, SHA-256
`c27189733d2ddcb04075b8b50471792933a793cfefa156b0b9475f4069d87746`;
the manifest matched before and after. Focused Ossie executable-profile tests
passed 26/26, engine library 13/13 and compiler view/context tests 13/13;
compiler examples checked successfully. The broad catalog/engine/Ossie/
compiler library/test sweep passed 403/403 across 101 binaries. Both locked
offline workspace all-targets checks (all-features and no-default-features),
formatting and diff checks passed. Strict Ossie model-level view-lineage import
now registers canonical direct-projection views after loaded sources, including
ordered nested views, through a synchronous engine entry point that validates
SQL dependencies, declared dependencies, names and schemas. It pins actual
source revisions and extension provenance; an optional expected revision
rejects stale input. Import→publication→typed SQL/direct expected rows and
malformed/missing/duplicate/stale fixtures passed. Logs are under
`/tmp/semantic-compiler-batch58-*`.

## Mandatory closure still open after batch 58

These are implementation and evidence gates for a full-architecture claim, not
regressions in the verified bounded profiles. A later package must either close
each mandatory clause with a named fixture and frozen verification or record a
specific architecture-authorized exclusion. P20's optional extensions remain
separate.

| Owner | Outstanding clause-level work | Decisive gate |
| --- | --- | --- |
| P03/P17 | Provider-authenticated key or functional-dependency evidence and its trust boundary; current authored keys cannot assert source enforcement | Publication/import rejects forged source evidence and a checked join uses only an attested constraint |
| P04/P14 | Fixed row/graph pass registry with versioned per-node type, nullability, key/grain, lineage and policy analyses, explicit transfer/invalidation and pass decisions | Corrupt scope, stale outer-join nullability, lost policy or requirement, and invalidated-analysis reuse fail at a named pass boundary |
| P05 | Full semantic role, quantifier/Boolean scope, alternative and default provenance in intent and repair | Independent negation/aggregate-scope and competing-interpretation fixtures retain evidence through binding or clarify |
| P06/P07 | Wider view lineage/applicability beyond exact direct projection, explicit equivalence where justified, and complete interpretation-context alternative closure | Nested-view restriction, partial temporal coverage and omitted competitor fixtures fail closed across the advertised profile |
| P07 | Manifest retrieval configuration, full prompt/token/work accounting and held-out sufficient-context quality | Pinned complete accounting plus field-only, cross-domain, budget-exhausted and dependency-diamond retrieval fixtures |
| P08/P16 | Production deferred connector schema-drift, snapshot retention and wider positive/negative dependency invalidation | Same-query revision/race fixtures and measured reuse without stale acceptance |
| P14/P15 | Runtime-obligation discharge, execution correlation and live target conformance for any advertised remote backend | Retained decision/replay trace links compile to execution; independent live PostgreSQL parameter/row/error results match the supported profile |
| P17 | Public-client and imported-source parity for every enabled advanced semantic family | Exact import→publish→bind→SQL/direct expected rows and rejection through CLI/HTTP for each advertised family |
| P18/P19 | Live held-out interpretation/retrieval gates and repeated scale/concurrency/overhead gates | Predeclared score thresholds and source-identified repeated measurements; scripted provider outcomes remain a separate gate |
| P21 | Final clause reconciliation, supported-profile/version audit and release disposition | Every enabled capability has expected-result, rejection, public-entry, revision and runtime evidence on the final frozen tree |

## Explicit scope decisions

- This is a greenfield compiler. Compiler APIs/IRs/proposal schemas may change
  directly; update current callers and reject obsolete artifacts. No old-client
  compatibility or migration project is required.
- Existing SQL compatibility mode remains separately labeled. Typed semantic
  failures never fall back to it as a successful compilation.
- Ordinary Rust `tracing` and compiler-owned records satisfy the observation
  path. OpenTelemetry remains excluded under the MVP handoff instruction.
- General SQL equivalence, natural-language writes, autonomous row profiling and
  a new physical optimizer are outside the architecture. Conditional extensions
  receive their P20 decision records before any full-design claim.

## P20 conditional decisions at this checkpoint

These are current implementation decisions, not claims that a future consumer
can never require the profiles. A selected extension needs a distinguishing
fixture, capability contract, revision dependency and execution evidence.

| Extension | Disposition | Evidence needed to select it |
| --- | --- | --- |
| Embeddings or reranking | Deferred | Held-out quality and end-to-end cost/latency improvement over lexical retrieval under the same incorrect-accept threshold |
| Additional SQL dialect or wider ClickHouse profile | Deferred | Named backend consumer and independent live dialect/result conformance |
| Substrait or external relational wire format | Deferred | Independent plan consumer and versioned subset/extension contract |
| Caller `AuthoredAssumptions` mode | Deferred; strict remains default | Named use case, caller-selected assumption classes and measured failure behavior |
| Cross-rename identity migration | Deferred | Persistence or catalog migration consumer with an authored conflict-resolved mapping |
| Geographic operations, additional calendar profiles, correlation, recursion or grouping sets | Deferred per profile | Concrete accepted use case, bounded typed semantics and distinguishing expected rows |
| Live value discovery | Deferred | Explicit product authorization, freshness/provenance policy and read budget |

The mandatory business/fiscal calendar mapping in P12 remains separate from
the optional *additional* calendar profiles above.
