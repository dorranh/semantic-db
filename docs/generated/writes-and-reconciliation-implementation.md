# Postgres writes and reconciliation

This implementation covers the first Postgres delivery stages of the
[strategy](writes-and-reconciliation-strategy.md) and
[design](writes-and-reconciliation-design.md). Iceberg remains a separate milestone.

## Entry points

`semantic-db` re-exports `PreparedWrite`, `WriteOptions`, `WriteResult`,
`ReadOptions`, `ReadExecution`, `ReadResult`, `ReadSession`, `Transaction`, the
connector traits, binding types, and table-creation types.

```rust,ignore
let prepared = engine.prepare_write(
    "UPDATE packages SET team = $2::text WHERE name = $1::text"
).await?;
let explanation = prepared.explain().await?;
let result = prepared.execute(parameters, WriteOptions::default()).await?;
```

Use `describe_write` with optional Arrow parameter-type hints for protocol
preparation. Values are bound, never interpolated. INSERT, UPDATE, DELETE and the
restricted keyed MERGE execute through normalized connector plans. Read APIs,
compiler output validation, and `plan_sql` remain read-only. Raw transaction SQL,
multiple statements, RETURNING, arbitrary MERGE actions and natural-language writes
are unsupported.

`REQUIRE IDEMPOTENT MERGE` checks the supported deterministic source operators,
assignments, enforced non-null key, target lineage and backend eligibility. It
rejects unsafe or unverified expressions/effects. A duplicate/null source key is
validated using destination types and equality before application-table changes.
Unchanged assignments skip updates; unassigned existing columns are preserved.
Source input must complete before application changes. Temporary Arrow staging
spills after 64 MiB and has a configurable 1 GiB disk limit.

Default writes require atomicity and have no automatic retries. Postgres executes
ordinary writes atomically even when BestEffort is allowed; checked statements and
explicit transactions reject BestEffort. Outcomes distinguish acknowledged commits,
uncommitted transaction statements, known aborts and unknown acknowledgements. An
unknown outcome is never a rollback acknowledgement or permission to retry blindly.

## Registration and Postgres

Existing Postgres configurations stay read-only. To opt in:

```yaml
connections:
  app:
    connector: postgres
    connection_string_env: APP_DATABASE_URL
    write_enabled: true
app_tables:
  synced_issues:
    connection: app
    schema: public
    table: synced_issues
```

Ossie is optional for app-only projects. Existing imports keep their semantic metadata;
reversible projections receive physical read/write bindings. Authored views are
read-only targets. Connection and resource evidence comes from the connector,
not descriptive catalog keys. Checked sources need connector-issued physical identity;
custom embedded providers can attach it with `attach_resource_identity`. Unknown
identity is rejected. Sources in the same storage namespace but a different configured
domain are conservatively rejected when disjointness cannot be established. Reload
after schema changes.

`Engine::create_table` supports explicit bootstrap definitions. Configuration loading
only attaches existing tables and does not execute migrations. Creation does not
replace an existing table. Failure after physical creation reports the recoverable
target instead of pretending DDL and registration were one atomic operation.

The Postgres connector retains its existing builtin Arrow/native type support and
`sslmode=disable` restriction. It uses temporary staging and native SQL in a pinned
transaction. Conservative destination locking protects live schema/key validation
through commit and serializes writes to each target. Checked merges use Serializable
isolation. Unknown defaults, generated/identity columns, unsupported collations,
triggers, rules, inheritance, expression/partial indexes, custom index comparison code
and unverified constraints are rejected in checked mode. No trigger
or application-function code analysis is attempted.

## Read sessions and transactions

Use `execute_read(sql, parameters, ReadOptions)` for observed or snapshot reads and
terminal reports. Snapshot queries require one verified read domain for every base
source. Read-only snapshot sessions do not require write credentials. Session and
transaction providers replace scans in an execution-local catalog, including aliases
and dependent views; they never replace the shared engine's providers.

`begin_read_session` takes registered relation names and `ReadSessionOptions`.
`begin_transaction` takes an `Arc<dyn WriteConnection>` and `TransactionOptions`.
Transactions support Repeatable Read and Serializable. Queries collect results;
mutable handles permit one operation at a time. Transaction writes return
`AppliedInTransaction`; `commit` returns a `WriteResult` with the actual commit outcome
and optional receipt. Errors or abandoned operations poison transaction handles.

External observations require `AllowObserved` inside sessions/transactions. They are
reported outside the pinned snapshot. A whole-query Snapshot requirement rejects
external observations even with that opt-in. Session lifetime defaults to 300 seconds;
Postgres accepts positive lifetimes up to 3600 seconds. Expiry never acquires a newer
snapshot. Native session scans serialize access and collect within query budgets
before yielding, avoiding interleaved-cursor deadlocks in joins.

Commit receipts are opaque evidence scoped to the live configured connector instance.
Pass them in `ReadOptions.after_commits`. Receipt reads pin an authoritative reader;
replica verification, receipt transfer across connector restarts, and visibility checks
against already-open snapshots are unsupported. A receipt is not authorization.

## Cache and completion reports

Configured caching remains available for unrelated read-only data. Bypass and MaxAge
are explicit policies. Snapshot/session/transaction reads, receipt reads and write
inputs bypass caches. Writable resources, known physical aliases and dependent views
cannot select cached generations after binding attachment or subsequent project loads.
Relations reached through a different configured connection in the same storage namespace
also bypass caching when physical disjointness cannot be established.

Version-2 cache manifests include publication time. Older generations without that
information are rebuilt. Reports identify selected generations and their publication
ages; those ages do not measure upstream data staleness. Failed refreshes do not fall
back to expired generations.

`ReadExecution.report` remains available after stream drop. Successful exhaustion
marks Complete; errors, cancellation and abandonment have distinct terminal states.
Already yielded rows remain provisional until Complete. `collect` cannot return a
successful partial result. There is no transparent whole-query retry.

## Frontends and example

The CLI supports `--write SQL`, `--explain-write SQL`, `--explain-read`,
`--read-consistency observed|snapshot`, `--read-cache configured|bypass|SECONDS`, and
`--read-report` for `--query`. Authored REPL writes use the same dispatcher. PostgreSQL
simple and prepared query paths return mutation command tags; Parse/Describe and
EXPLAIN never apply writes. Snapshot sessions and receipts are library interfaces.

The package maintenance server uses its Semantic DB `pg` connection for every runtime
save. Prisma remains responsible for migrations and setup seeding. Run
`npm run reconcile` from the example directory after setup/start to synchronize GitHub
issues into `synced_issues`. The dashboard continues reading live GitHub observations.

The command holds `.run/reconcile.lock` before observing sources. An abandoned command
may leave a lock: verify that the recorded PID has stopped before removing it. This
is local example serialization, not coordination across deployments. Missing source
rows are retained; deletion reconciliation and scheduling remain application-owned.

## Verification

The normal workspace suite includes connector fault tests for stream completion,
late failures, staging budgets, cancellation cleanup, source participation and
cache alias exclusion. Protocol tests cover prepared writes without Parse-time
mutation and both simple/prepared cancellation. Protocol cancellation reports
`08007 / OutcomeUnknown` for a dispatched write because cancellation can interrupt
commit acknowledgement; read cancellation retains its existing behavior.

Run the native Postgres conformance tests with Docker available:

```sh
cargo test -p semantic-postgres -p semantic-sources --all-features \
  connector_integration_postgres -- --ignored
```

These exercise replay, composite keys, snapshots, transactions, creation/loading,
schema/index validation, and a TCP proxy that drops the acknowledgement after an
actual commit. The example integration suite checks repeated reconciliation,
changed fixture observations, preserved triage, and no saved publication after a
late source-page failure. CI runs native conformance and the example's reconciliation,
integration and browser checks.
