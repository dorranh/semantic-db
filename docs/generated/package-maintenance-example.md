# Package maintenance: application integration example

This example joins application-owned Postgres tables, repository-scoped GitHub
issues, and ClickHouse PyPI download aggregates through Semantic DB. React/Vite
provides the UI; a Node backend reads and writes through Semantic DB with `pg`.
Prisma owns application migrations and setup seeding. The optional Ask panel compiles questions against the
same authored views.

## Review stages

1. **Postgres connector:** `crates/semantic-postgres`, registry/facade/CLI feature
   wiring, `tests/support/postgres.rs`, and the Ossie connector integration test.
2. **Engine and protocol:** `crates/semantic-engine/src/parameters.rs`, the
   materialization regression test, and `apps/semantic-server/src/protocol.rs`.
3. **Dashboard:** `examples/package-maintenance` databases, migrations, fixtures,
   model/views, backend, UI, and lifecycle scripts.
4. **Ask and scenarios:** server compilation HTTP routes and deterministic tests,
   the Ask panel, fixture controls, example CI, and this guide.

Each stage builds on the preceding foundation. Generated documentation is kept here;
the implemented reconciliation command is described at the end.

## Run locally

Requirements: the workspace Rust toolchain, Node >=22.12, npm, and Docker Compose.
With Just installed, run from the repository root:

```sh
just package-maintenance        # install/setup, then launch; preserves edits
just package-maintenance-stop   # stop app and databases; preserves volumes
just package-maintenance-check  # unit tests, TypeScript check, and UI build
just package-maintenance-test   # integration/browser tests against the running app
just package-maintenance-reset  # DELETE volumes and recreate fixtures
```

`just package-maintenance-setup` runs installation and database setup without
launching the app. For live sources use `SEMANTIC_LIVE=1 just package-maintenance`.

Alternatively, run from `examples/package-maintenance`:

```sh
npm ci
npm run setup
npm start
```

Open <http://127.0.0.1:5173>. Setup creates a local `.env` from `.env.example`, starts
the databases, generates Prisma Client, applies committed SQL migrations, and
seeds missing records. The example deliberately pins Prisma 6's SQL migration
workflow. Seed upserts never overwrite existing application edits. Ordinary start
only builds/launches the services; it does not run migrations or reset data.

```sh
npm run stop    # stops app processes and databases, preserves volumes
npm run setup   # restart databases / apply new migrations, preserve edits
npm start
npm run reset   # explicitly DELETE example database volumes and recreate fixtures
```

`Ctrl+C` stops the app services, leaving database containers running. Do not run
multiple instances on the same ports. No services publish outside loopback.

| Service | Address |
| --- | --- |
| Vite UI | 127.0.0.1:5173 |
| Node app API | 127.0.0.1:3001 |
| Semantic DB PostgreSQL protocol | 127.0.0.1:5544 |
| Semantic DB compilation/catalog HTTP | 127.0.0.1:5545 |
| Application Postgres | 127.0.0.1:5543 |
| ClickHouse HTTP | 127.0.0.1:8124 |
| GitHub fixture HTTP | 127.0.0.1:4010 |

All checked-in credentials are disposable local fixture credentials. Source and
model credentials are resolved in the server and are never sent to the browser.

## Data and semantic behavior

- `packages` owns the PyPI-to-repository mapping, team, and maintenance notes.
- `members` contains local assignees, not GitHub accounts.
- `issue_triage` stores optional human-owned assignment, priority and notes, keyed
  by the global GitHub issue ID. Missing triage defaults to unassigned/normal.
- `downloads_daily` sums physical ClickHouse contributions by package/day before
  any issue join. Its explicit fixture window is **September 1–14, 2026**.
- `issue_workspace` joins GitHub issues to optional triage and member records.
- `package_overview` joins independently aggregated issue and download totals.

`requests` has **20,167,017** fixture downloads, including an additional contribution
of 17 on September 1. Its two open issues do not multiply that total. The same
repository is deliberately mapped to `requests-toolbelt`; repository issue counts
repeat for both packages and must not be summed across packages. Missing download
observations remain NULL, not zero. Source failure is an error, not an empty result.

The UI preserves 64-bit counts as decimal strings; chart scaling uses BigInt
arithmetic. PostgreSQL dates/timestamps retain their textual precision at the
Node API boundary. All application-facing joins execute in Semantic DB.

Prisma owns migrations and setup seeding. Runtime writes use Semantic DB. Ossie is checked against existing source schemas.
Project loading never creates tables, applies migrations, or treats descriptive
Ossie keys as enforced constraints. Materialization is disabled in this example.
After an acknowledged save the app performs a new federated read. This demonstrates
fresh observations from the configured authoritative Postgres instance, not a
Semantic DB commit receipt or coordinated cross-source snapshot.

## Query and compiler interfaces

`apps/semantic-server` uses `datafusion-postgres` at pinned revision
`ffb14a52b3d7c7489559812e1ec38096f225ce93` (DataFusion 55 / Arrow 59) with custom
handlers. The published frontend release targets an older DataFusion version.
Both simple queries and extended Parse/Bind/Execute use Semantic DB's engine;
neither exposes the private DataFusion session or invokes the upstream SQL hooks.

```ts
const semantic = new Pool({ connectionString: process.env.SEMANTIC_DATABASE_URL });
const result = await semantic.query(
  'SELECT * FROM package_overview WHERE package_name = $1', ['requests']
);
```

The new Rust APIs are `Engine::describe_read(sql, parameter_type_hints)` and
`Engine::execute_parameters(sql, scalar_values, query_options)`. Description is
read-only and never fills caches. Execution uses the existing materialization and
query-budget path. Parameter indices are ordered numerically; values are bound,
never inserted into SQL text. Use explicit SQL casts when inference is ambiguous.

The initial frontend supports `pg`, prepared statements, typed NULLs, text/binary
results, basic `psql` queries, cancellation, and recovery after errors. Explicit
parameterized mutations use the engine write dispatcher. DDL, raw transaction
control, multiple statements and session changes are rejected. Startup accepts
UTF-8 encoding and connection identity fields; nonempty `options` and other session
settings are rejected. PostgreSQL
wire compatibility does not imply PostgreSQL SQL, catalog, ORM, or transaction
compatibility. `REQUIRE IDEMPOTENT MERGE` reaches the engine through these handlers.

Responses are collected before sending rows, with a **64 MiB / 100,000 row** server
result cap and the engine's execution budgets. This deliberately prevents partial
results from becoming application successes. Closing an in-progress client without
sending CancelRequest may leave work running until the query deadline; resources
are released when execution completes or is cancelled. Incremental wire streaming
remains future work; richer guarantee reports and sessions are library APIs.

The HTTP side exposes `GET /health`, `GET /catalog`, and `POST /compile` with
`{"question":"..."}`. It has no SQL execution route. Compilation returns the existing
`Compilation` outcome: grounded SQL/evidence, clarification, or unsupported.
The app executes grounded SQL through `pg`. For Ask, set `OPENAI_API_KEY`,
`OPENAI_MODEL`, and optionally `OPENAI_BASE_URL` in the example `.env`, then restart.
The compiler receives semantic metadata and the question, not source row samples.
Without model configuration the entire workspace still runs and Ask is disabled.

## Postgres connector contract

Enable the `postgres` feature on `semantic-db`/`semantic-sources`, or use the stock
CLI/server where it is enabled. Configuration:

```yaml
connections:
  app:
    connector: postgres
    connection_string_env: APP_DATABASE_URL
    pool_size: 8
    batch_size: 1024
    write_enabled: true
sources:
  app.packages:
    connection: app
    schema: public
    table: packages
```

The connector uses `tokio-postgres` and `deadpool-postgres`. It currently requires
`sslmode=disable`; TLS is not implemented. Use the local example deployment.
Native sessions default to read-only, with finite statement and idle-transaction
timeouts. Every scan owns a transaction and cursor, fetching bounded batches.
It rolls back after completion and discards the connection on cancellation/error.
Independent scans do not share a snapshot.

Supported native types: text/varchar, Boolean, smallint/integer/bigint, real/double,
date, timestamp, and timestamptz. Timestamp precision is microseconds; timestamptz
normalizes to UTC. Metadata nullability is conservatively nullable. Unsupported
types (including uuid, numeric, enums, arrays, JSON) fail inspection explicitly.
Bind a backend view with supported representations when appropriate. Schema changes
require project reload; incompatible scan schemas fail instead of coercing silently.
Projection and planner-safe limits are pushed; predicates and joins stay local.

Row batches charge decoded Arrow size against query byte budgets. The native client
does not expose exact wire byte counts; this is not a wire-payload memory bound.
A single large Postgres value may allocate before the decoded budget rejects it.

## Live sources

The live configuration uses [ClickPy's public demo instance](https://github.com/ClickHouse/clickpy#configuration):
`https://sql-clickhouse.clickhouse.com:443`, database `pypi`, user `demo`, with no
password. Supply a real `GITHUB_TOKEN` in `.env` with access to the listed repositories.
The fixture's `CLICKHOUSE_PASSWORD` is not used by the public live configuration.

The public account caps `max_block_size` at 10,000, so the live configuration sets
`max_block_size: 10000`. The connector's default of 65,536 exceeds that cap and
causes HTTP 500 / ClickHouse code 452 (`SETTING_CONSTRAINT_VIOLATION`), including
during metadata loading. This block size controls response batching, not the total
number of rows that a query can read.

The public account also enables legacy integer encoding for Arrow dates. The live
configuration sets `date_as_uint16: false` to request Arrow Date32 values, matching
the semantic model. Leave this option unset on older ClickHouse versions that do
not support `output_format_arrow_date_as_uint16` (introduced in 26.2).

For a private instance, change the endpoint and user and add `password_env` for its
password. Its database must expose ClickPy's `pypi_downloads_per_day` schema
(`date Date`, `project String`, `count Int64`). No giant raw downloads import is needed.

Review `views/downloads_daily.sql` and the model's window descriptions together if
changing the date range or package scope. The committed example intentionally uses
a historical, reproducible window; it does not call this current activity. Start with:

```sh
SEMANTIC_LIVE=1 npm start
```

Live mode disables the GitHub fixture process and never falls back to fixture data.
The configured repository scope is fixed at startup. The fixture-only quiet repository
is not queried in live mode. Application mappings outside scope have no observed issues;
this does not establish that those repositories have no issues upstream.

The issue panel includes both open and closed issues. It first resolves the selected
package, then binds its repository into the Semantic DB detail query. The GitHub
provider follows cursors within one scan, fetching every matching page. Repository
equality (including `lower(repository)`) prunes pagination for other repositories.
One initial page per configured repository establishes its canonical name, preserving
correctness when a configured name is an alias after a rename.

Live mode allows 128 GitHub requests per scan and a 120-second query deadline;
the Node client's query timeout is 130 seconds. Fixture mode keeps its 24-request,
30-second engine and 35-second client limits. These are total scan/query budgets,
not per-page budgets, and are never reset while following a cursor. A later-page
failure, exhausted budget, or cancellation still fails the whole collected result.
The panel's sorting requires complete matching input before it can return rows.
Live mode caches the GitHub `issues` relation for 300 seconds from successful
publication in `.run/cache/live`, with 64 MiB memory and per-fill limits and a
256 MiB disk budget. The first query after a miss or expiry fetches all issue states
across all configured repositories before publishing the cache; subsequent queries
reuse that complete generation. Refresh is demand-driven, with no stale fallback
after a failed fill. Cache age does not bound the age of upstream observations.
Package and triage tables remain uncached, so edits are joined to the cached issues
on every read. ClickHouse downloads and fixture mode remain uncached. Restart the
live server after changing cache settings.

The same live engine deadline applies to `npm run reconcile`, which still observes
all configured repositories and both issue states, bypassing the issue cache.

The server also exposes `--query-timeout-seconds` for launches outside the example
lifecycle script (default 30). Repository scope, request limits and memory limits
remain enforced; there are no automatic retries.

ClickHouse queries have explicit package/date predicates and finite server/response
budgets. The local integration checks inspect the native query log to ensure these
predicates reach ClickHouse. Verify the same query plans and limits on your deployment;
this example does not promise a live-source latency or completeness SLA.

## Checks and fixture controls

```sh
# Repository root
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo test -p semantic-postgres connector_integration -- --ignored
cargo test -p semantic-sources --features postgres connector_integration_postgres -- --ignored

# Example directory, after npm start
npm test
npm run build
npm run test:integration
npm run test:e2e  # requires agent-browser and its Chromium installation
```

The browser check writes screenshots to ignored `.run/desktop.png` and `.run/mobile.png`
and restores the application values it changed. Run destructive/reset checks only on
this disposable example stack. Ordinary tests do not require GitHub/model credentials.
Compiler integration tests use a local mock model; protocol tests inject delayed and
late-failing streams. Public live sources remain opt-in, outside deterministic CI.

Fixture controls accept POST JSON at `http://127.0.0.1:4010/control`:

```json
{"closed_issue":"I_requests_2"}
{"delay_ms":2000}
{"fail_after_first_page":true}
{"reset":true}
```

Failure after page one affects only scans that actually request another page. The
requests detail query includes closed issues and exercises this failure; a filtered
query that fits on one page can still complete legitimately.

## Writes and reconciliation

Runtime saves in `server/mutations.ts` use parameterized Semantic DB UPDATE and MERGE.
Prisma is not loaded by the application server. Existing validation and UI behavior
are preserved. Lost commit acknowledgements receive an explicit unknown-outcome
message; the application does not automatically retry mutations.

After applying the additive migration with `npm run setup` and starting the app:

```sh
npm run reconcile
```

The command executes `writes/reconcile_issues.sql` through Semantic DB and prints the
outcome and saved `synced_issues` rows. Repeating unchanged observations produces no
additional logical changes. Later runs propagate changed GitHub state. The separate
`issue_triage` table preserves human assignments, priorities and notes. Missing source
rows are retained; a failed or partial source scan is never authoritative absence.
The dashboard continues showing live-source observations.

Runs hold `.run/reconcile.lock` before scanning sources. If a crashed process leaves
this file, verify its recorded PID is no longer running before removing it. No
background scheduler or cross-deployment coordination is supplied.

Integration tests cover replay, changed fixture state, preserved triage, and late-page
failure without destination publication. Native conformance tests cover snapshot
sessions, transaction read-own-writes, rollback, composite keys, schema changes,
expiry and receipt visibility. See the
[implementation guide](writes-and-reconciliation-implementation.md) for the public
interfaces and conservative Postgres capability restrictions.
