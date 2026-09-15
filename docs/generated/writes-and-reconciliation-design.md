# Reads, writes, transactions, and reconciliation: technical design

Status: proposed implementation design, 2026-09-15. APIs and SQL extensions below
are new work. The [strategy document](writes-and-reconciliation-strategy.md)
defines the product guarantees without committing to individual connectors.

## 1. Current implementation and ownership

The engine owns a private DataFusion session and an in-memory descriptive catalog.
Its SQL planning path disables DDL, DML, and session statements. Providers resolve
once at registration. SourceConnection exposes read providers but the loader does
not retain a write session. Ossie imports wrap physical providers in projections;
declared primary/unique keys are explicitly unenforced. Query execution and
materialization are currently read-oriented. DataFusion 55 is the pinned baseline.

Extend the existing crates rather than creating a distributed coordinator:

| Component | New responsibility |
| --- | --- |
| semantic-engine | Statement dispatch, read/write bindings, normalized mutation plans, requirement checks, staging, read sessions, transaction handles, reports/errors. |
| semantic-sources | Retain optional read-session and write connections independently; attach them to concrete registered resources. |
| semantic-runtime | Execution budgets, cancellation/completion support, temporary staged-input lifecycle shared with connectors. |
| semantic-catalog | Continue describing relations; do not promote semantic key declarations to enforced constraints. |
| Connector crates | Read/commit domain identities, snapshot providers, commit visibility, native mutation execution, conflict validation, transaction-bound reads. |
| semantic-db | Re-export the public read, write, session, and transaction interfaces. |
| semantic-cli | Explicit mutation entry points, read requirements/reports, explanations, errors, and completion. |

Keep connector contracts in semantic-engine for the first implementation; existing
connectors need not implement them. Reuse DataFusion expression/types infrastructure,
but do not let its default mutation execution bypass the engine's checks.

## 2. Public entry points and compatibility

Add the following conceptual API surface; signatures use existing Arrow/DataFusion
types for schemas and scalar parameters:

    Engine::prepare_write(sql) -> PreparedWrite
    PreparedWrite::explain() -> WriteExplanation
    PreparedWrite::execute(parameters, WriteOptions) -> WriteResult
    Engine::begin_transaction(connection, TransactionOptions) -> Transaction
    Transaction::execute_write(sql, parameters) -> WriteResult
    Transaction::query(sql, parameters, ReadOptions) -> ReadResult
    Transaction::commit() -> CommitReceipt
    Transaction::rollback()

    Engine::execute_read(sql, parameters, ReadOptions) -> ReadExecution
    Engine::explain_read(sql, ReadOptions) -> ReadExplanation
    Engine::begin_read_session(relations, ReadSessionOptions) -> ReadSession
    ReadSession::query(sql, parameters, ReadOptions) -> ReadResult
    ReadSession::close()

ReadResult contains collected RecordBatches and a completed ReadReport. ReadExecution
owns a guarded stream, a live report/completion handle, and any query-scoped snapshot
resources; its collect method returns ReadResult only after successful completion.
Add these as new APIs; existing query/execute/prepare signatures remain compatible.
Legacy read paths retain observed-read defaults and existing configured cache policy.

Methods that plan, inspect, read, or write asynchronously are async. PreparedWrite
captures the engine binding generation; execution rejects stale schema/binding
assumptions and revalidates backend facts under the connector's concurrency rules.
Use positional $1, $2 parameters bound as values, never interpolated SQL. Identifiers
are resolved from registered targets and quoted by the connector.

WriteOptions contains atomicity (Required by default; explicit BestEffort for
ordinary writes), existing query budgets, and a staging disk budget. Start with a
64 MiB staging memory threshold and a 1 GiB temporary disk limit, both configurable.
Transactions and REQUIRE IDEMPOTENT reject BestEffort. Default automatic write
retries to zero; applications can start a new run after a conflict. Do not invent
an automatic retry guarantee from the idempotency modifier.

TransactionOptions requires a supported isolation level and defaults external
reads to Reject. AllowObserved is an explicit alternative. No silent isolation
downgrade. One handle permits one in-flight operation; commit/rollback consume it.
Transactions return collected reads initially, bounded by budgets, so streams cannot
escape the transaction or keep a connection busy during commit.

The qualifier certifies one statement, including when used inside a transaction.
It does not certify that replaying a sequence of individually accepted statements
is idempotent; later statements can change earlier statements' inputs. Do not expose
a transaction-level idempotency assertion in this version.

Preserve Engine::query, plan_sql, execute, and generated-SQL validation as read-only.
The stronger read contract is available through execute_read and the session APIs;
low-level DataFrame access via plan_sql does not certify those guarantees. No new
read SQL modifier in v1: use ReadOptions, and CLI --read-consistency observed|snapshot
plus --read-cache configured|bypass|MAX_AGE_SECONDS for --query. --explain-read plans
without fetching rows or acquiring snapshots, and --read-report prints the terminal
report to stderr. Snapshot sessions and commit-receipt requirements are library APIs.
Add CLI --write and --explain-write; retain --query semantics. In the REPL, route
explicitly authored DML/REQUIRE statements through the write dispatcher. Initial
multi-statement transaction control is a library API; reject raw BEGIN/COMMIT and
multi-statement SQL with a targeted message. Do not add natural-language writes.

## 3. Read/write bindings and connector contracts

A registered base relation may have an optional WriteBinding alongside its read
provider. It contains a shared WriteConnection, an opaque TargetId, and an explicit
logical-to-physical column mapping. Neither source strings nor descriptive keys
are executable routing information.

Independently, a base relation may have a ReadBinding containing an optional
ReadConnection and opaque ResourceId. Add SourceConnection::read_connection and
Engine::attach_read_binding separately from write support, with default unsupported
snapshot/visibility capabilities. A plain TableProvider remains valid for observed
reads. Retain physical resource identity through aliases/projections so all scans of
the same resource are bound consistently. Unknown identity/mapping rejects stronger
requirements. Views obtain read bindings through expanded dependency lineage.

ReadConnection provides inspect_read(resource), validate_read(resources, requirements),
bind_observed_read(resources, options, context), and open_read_session(resources, options).
The observed binding establishes requested visibility/routing before returning read
providers without implying a shared snapshot; session binding additionally pins it.
Reports identify a ReadDomainId, supported
snapshot scope, visibility capabilities, and session lifetime limits. A ReadSession
provides bound read providers, snapshot evidence, and close/cleanup. These interfaces
must work with read-only credentials and connectors without any WriteConnection.
Write transactions implement the same provider-binding/report machinery against
their native transaction session, adding read-your-writes; opening a separate read
session must never substitute for that transaction's reads.

Add SourceConnection::write_connection with a default read-only result, and an
optional source-to-target binding method. Preserve existing registration and backend
traits; add Engine::register_writable_table and attach_write_binding for explicit
opt-in. The project loader retains connection objects and attaches write bindings
after read registration. For Ossie imports, permit only verified direct column
renames/projections with reversible mappings. Authored views, calculated columns,
and unknown mappings are read-only targets initially. Key columns must be exposed.

Use object-safe traits returning BoxFuture, following the source loader's existing
style. The contract has these operations:

| Interface operation | Required result/behavior |
| --- | --- |
| inspect_target(target) | Current schema revision, supported operations, key evidence, comparison semantics, side-effect eligibility. |
| validate_operation(plan, requirements) | Operation-specific eligibility, runtime checks, concrete CommitDomainId and atomicity/durability/isolation description. |
| apply(plan, staged_input, context) | Execute one operation; return a known commit, known failure, or unknown outcome. |
| begin(targets/options) | Optional multi-statement session; verifies one domain and requested isolation. |
| transaction read_provider(target) | A provider bound to that session, including uncommitted writes. |
| transaction apply/commit/rollback | Stateful lifecycle with errors that preserve outcome knowledge. |

CommitDomainId is opaque, connector-issued, and scoped to the configured access
context. Never infer it from connector name, matching URLs, or credentials. Sharing
a domain is necessary but does not replace validation of the entire operation and
its live execution session. New transaction targets must be validated/enlisted before
their statement; unsupported access poisons the transaction so it cannot be committed.

Key evidence distinguishes native enforcement, validation protected by a commit
precondition, and descriptive-only declarations. Equality, null handling, and type
conversion must agree between staging and destination matching. Use the connector's
native staging validation when comparison semantics differ; reject unsupported
collations/conversions rather than trusting Arrow uniqueness alone.

Ordinary INSERT, UPDATE, DELETE, and restricted MERGE have separate capability
checks. First support one mutation target per statement and registered relations
only. Cross-source inputs are evaluated by the engine and staged. Target predicates
and assignments execute inside the destination operation; do not precompute a
read/modify/write from unprotected target reads. Unsupported expression lowering
fails before application-table changes. RETURNING and arbitrary multi-action MERGE
are deferred; writes return execution reports instead of lazy mutation DataFrames.

## 4. SQL and checked merge normalization

Recognize this extension before delegating the remaining SQL to the pinned parser:

    REQUIRE IDEMPOTENT
    MERGE INTO issue_assignments AS target
    USING (
        SELECT id AS issue_id, state AS github_status
        FROM github_issues
    ) AS source
    ON target.issue_id = source.issue_id
    WHEN MATCHED THEN
        UPDATE SET github_status = source.github_status
    WHEN NOT MATCHED THEN
        INSERT (issue_id, github_status)
        VALUES (source.issue_id, source.github_status);

Tokenize the leading modifier case-insensitively, respecting comments and quoted
text; do not remove it with a regex. Allow one statement and an optional trailing
semicolon. Support EXPLAIN REQUIRE IDEMPOTENT MERGE ... as non-executing explanation.
Require bound parameter values for parameter-dependent checks at execution.

Normalize accepted MERGE syntax to KeyedMergePlan: one target, a source SELECT plan,
target/source key pairs, update assignments, and optional insert assignments.
The connector receives this representation, not an unvalidated SQL string. It may
lower it to native SQL or file operations. Both clauses are optional individually,
but at least one is required; absence of an insert clause means update-only.

### Initial acceptance rules

- One base target; ON is only a conjunction of equalities mapping a complete,
  verified non-null key to source columns. Composite keys are supported.
- At most one unconditional WHEN MATCHED UPDATE and one unconditional WHEN NOT
  MATCHED INSERT. No delete, key updates, extra match predicates, or by-source clauses.
- Updates and inserts use deterministic source expressions, constants, and bound
  parameters. Inserted key values must be exactly the matched source key values.
- Unassigned existing columns are preserved. Omitted insert columns must have
  connector-verified safe behavior; required columns without values reject the plan.
- The expanded source plan cannot read the destination physical target, including
  aliases, views, and alternate bindings. This prevents feedback through the source
  query. Other application tables can be joined; their observations belong to S.
- Start source support with scans, projections, filters, and deterministic inner/
  left equijoins. Reject aggregates, windows, LIMIT/OFFSET, sampling, and other source
  operators until their repeatability rules are separately implemented and tested.
- Audit an allowlist of scalar operations/types for deterministic behavior. Reject
  stable-per-query time functions, randomness, unverified UDFs, sequences, and opaque
  remote expressions. Volatility metadata alone does not prove replay stability.
- Reject unknown application-visible side effects. The connector must account for
  relevant triggers, defaults, generated expressions, and secondary mutations;
  initial implementations should reject unsupported cases rather than analyze code.

Validate the pre-optimization plan, expanded lineage, and connector execution
semantics. Pushdown must preserve the checked expressions and comparison behavior;
disable pushdown for a source fragment where that cannot be established. Duplicate
or null source keys reject the run; never select an arbitrary winning row. Empty
input is a no-op, never an instruction to delete destination rows.

Explain output lists normalized operation, mapped key, actual boundary, requested
and supported guarantees, source observations, static checks, runtime checks, and
rejection reasons. A successful EXPLAIN does not claim runtime checks already passed.

## 5. Execution, retries, and failure states

Execute in this order:

1. Resolve targets, validate the static plan and connector eligibility.
2. Execute source reads to completion into immutable staged Arrow batches, spilling
   to temporary Arrow IPC when needed. Bypass materializations in write inputs in v1.
3. Validate staged keys and values, including connector comparison semantics.
4. Begin the destination operation or use the explicit transaction session; check
   target revision, key evidence, side effects, and concurrency preconditions.
5. Apply all destination changes and publish through the validated commit boundary.
6. Return the actual outcome; remove staging after it is no longer needed.

Step 4 may require private staging writes. "Before destination changes" means before
application-table changes can become visible, not that no temporary bytes are written.
Any source error, incomplete stream, conversion failure, or budget exhaustion aborts
the run. Connector validation must remain protected through commit; preflight
inspection alone cannot exclude concurrent DDL or newly inserted duplicate keys.

Use explicit outcome classes:

| Outcome | Meaning |
| --- | --- |
| Rejected | Unsupported plan/guarantee or failed validation; no visible destination changes. |
| Aborted | Atomic destination operation is known not to have committed. |
| Committed | Commit acknowledged, with optional opaque receipt. |
| OutcomeUnknown | Commit may have happened; do not report success or rollback. |
| PartialFailure | Explicit BestEffort ordinary write failed with possible partial effects. |
| AppliedInTransaction | Statement succeeded; no commit has yet occurred. |

A cancellation before commit should roll back. After commit dispatch, cancellation
or lost connectivity may yield OutcomeUnknown. Transaction errors poison the handle;
only rollback/cleanup remains available. Dropping a handle schedules best-effort
cleanup and prevents pool reuse until the connector proves the session clean; drop
is not a synchronous rollback acknowledgement.

Fixed-input retry means reuse of the exact staged rows and parameter values. A
connector may retry internal commit conflicts only when it can revalidate the full
operation and outcome; it must not blindly rebase writes. No generic retry after
OutcomeUnknown, and no durable operation-deduplication ledger in v1. Starting a fresh
query is a new observation, not recovery of an exactly-once job.

WriteResult includes the outcome, boundary, guarantee report, operation ID, optional
commit receipt, and optional affected-row statistics. Do not promise equal counts
across connectors or repeated runs. Diagnostics exclude bound values and secrets.

## 6. Read requirements, sessions, and materialization

### Ordinary reads and requested guarantees

ReadOptions wraps existing QueryOptions budgets and adds:

- consistency: Observed (default) or Snapshot.
- cache: Configured (default), Bypass, or MaxAge(duration).
- after_commits: zero or more connector-issued commit receipts (default empty).

Use ReadOptions.cache as the authoritative policy on the new entry point; reject
contradictory legacy cache flags instead of applying hidden precedence. Existing
entry points retain their existing QueryOptions behavior.

Observed reads do not establish a common snapshot. Providers may naturally supply
stronger behavior, but the engine only advertises what it establishes. Repeated
scans, pagination, view expansion, and federation can observe source changes within
one SQL statement. Preserve ordinary cross-source joins and existing read-only use.

Snapshot requires all expanded base dependencies to participate in one verified
ReadDomainId and live session. Reject unknown/independent domains before executing
the query. For a standalone query, acquire a query-scoped snapshot before any row
scan and retain it until completion or cleanup. Resolve all aliases to session-bound
providers; no scan may use a shared provider that escapes the session. DataFusion
pushdown and local residual execution must preserve this binding. Even a fully
pushed-down query is subject to requirement validation. Queries with no base sources
need no snapshot acquisition. EXPLAIN reports acquisition and runtime checks as
pending; it does not open a session.

### Read-only sessions

ReadSessionOptions requests Snapshot and sets external_reads to Reject by default;
AllowObserved is explicit. Begin takes a nonempty set of registered relations, expands
their dependencies, and verifies one read domain. Opening establishes a snapshot
(forcing acquisition when the backend begins lazily), rather than just reserving a
connection. Options may include after_commits receipts to satisfy before snapshot
acquisition. Freeze the engine catalog/bindings for the session; newly referenced
resources must join the same snapshot or be rejected before their query scans begin.
External sources in opt-in sessions remain ordinary observations and are listed
outside the snapshot guarantee.

Session queries cannot weaken the pinned-domain binding, even with Observed options;
requesting Snapshot for the whole query still rejects external observations. Support
one in-flight query per session and collected ReadResult initially. Require a finite
session lifetime (default 300 seconds, configurable within connector limits), checked
at query start and during execution. Expiry closes the session and fails active work.
Query budgets apply separately and cannot extend session lifetime. Explicit close or
drop releases resources; errors/expiry never silently reopen at a newer snapshot.

Pin current snapshots only in v1. Historical snapshot selection is deferred. Snapshot
guarantees cover base data, not volatile functions or unordered result presentation.
If retention/schema changes make the pin unreadable, fail rather than refresh it.

### Transaction reads

Build transaction-local read contexts and rebuild dependent views with transaction
providers; never swap providers on the shared Engine. Query splitting and federation
must preserve the requested isolation. Reject unsupported guarantees, including a
statement-wide snapshot if separate backend scans cannot share one. Serialize use
of the native session or stage scan results as required by its driver.

Participating reads bypass caches and see the transaction's own writes. External
reads require AllowObserved and carry no stronger guarantee than their source.
The default rejection applies through expanded view lineage, not only SQL table names.

### Post-commit visibility

A known successful commit may return connector-issued visibility evidence in its
CommitReceipt. Absence of usable evidence means after_commits is unsupported, not
implicitly satisfied. Receipts include an opaque domain/access-bound identity and
scope; never expose credentials or treat receipts as authorization.

Before scanning, the read connector validates that each receipt applies to a read
dependency and establishes a view that includes it: route to a suitable authoritative
reader, or wait for verified replica visibility within the existing query deadline.
Reject incompatible receipts/domains or unsupported verification. A timeout returns
a read failure, not a stale successful result. Read visibility includes the commit's
effects subject to subsequent changes; it does not promise its values are unchanged.

Establish visibility before acquiring a new requested snapshot. For an already pinned
session/transaction, verify its snapshot includes the receipts or reject the request;
do not advance it. Multiple receipts must all be satisfied but imply no shared snapshot
across independent domains. Ordinary reads have no implicit session-wide last-write
tracking. Callers pass receipts when they require a post-commit guarantee.

### Cache policy

Configured retains existing per-relation freshness policies. MaxAge tightens those
policies transitively through dependent views; it never enables otherwise disabled
caching or relaxes a stricter configured limit. Bypass disables materializations.
Age is measured from successful cache generation publication, not an assertion about
when upstream data changed. No fallback to an expired generation after refresh failure
when the effective age bound cannot be met. Bypass does not eliminate upstream lag.

Snapshot reads, all session/transaction reads (including opt-in external reads),
after_commits reads, and write-input scans bypass caches in v1. This stronger rule
overrides a permissive configured/requested cache policy and appears in the report.
Exact snapshot-tagged cache reuse can be added later.

For v1, bypass materialization for any relation/view transitively depending on a
writable target, including read-only aliases of its physical identity. Invalidate
existing generations when attaching the write binding and prevent their reuse on
future loads. This prevents cached writable data from violating explicit visibility
requirements, including when writes occur outside semantic-db; source routing still
has to establish visibility. Retain cache support for unrelated read-only relations.
An epoch/CDC-based writable-cache design can follow separately.

### Result reports and streaming completion

ReadReport records requested/established guarantees, participating resource/domain
identities, available opaque snapshot evidence, external observations, actual cache
generations/publication times/ages, satisfied receipts, and completion state. Reuse
existing remote budgets/metrics; never invent a source timestamp or snapshot token
when a provider cannot supply one. Explain reports planned guarantees; execution
reports actual acquisitions and cache selections.

The guarded stream owns a shared completion handle with Pending, Complete, Failed,
Cancelled, or Abandoned states. Only successful exhaustion of the full execution
marks Complete. A late error marks Failed even after rows have been yielded; dropping
an unfinished stream marks Abandoned and releases query-scoped sessions. Polling the
completion handle alone does not drive execution. Keep reports available after stream
drop; collect returns an error rather than a successful partial ReadResult.

No transparent whole-query retry in v1. A fresh execution after failure must discard
previous partial output and can observe different data; a still-valid pinned session
preserves only its participating snapshot. Connector-local transport retries are
allowed only if they preserve scan/snapshot identity and cannot re-emit rows. Streaming
consumers must treat rows as provisional until Complete; external side effects they
perform from partial output are not rolled back by semantic-db.

## 7. Bootstrap and application tables

Add capability-gated Engine::create_table(connection, TableDefinition), taking
exclusive engine access. TableDefinition provides logical registration name, opaque
connector target options, Arrow schema, and explicit key constraints. The connector
creates the physical table, inspects it, and returns verified read/write bindings.
Validate catalog name conflicts and definitions before backend DDL.

Creation rejects an existing physical table; attach existing tables explicitly via
registration. No automatic drop/alter or schema inference into destructive migrations.
If physical creation succeeds but registration fails, return the created target's
identity so the application can recover; do not claim cross-catalog atomicity or
delete the table automatically. Creation runs at application initialization, before
sharing the engine across requests. Reload/rebuild remains the schema refresh path.

Add optional project app_tables entries for attaching existing application tables:
logical name, configured connection, and connector-owned target options. Read schemas
and enforced key facts come from live connector inspection. These entries join the
catalog before project views are planned; validate names against imported datasets
and views offline. Loading does not create tables or run migrations. Make the Ossie
model optional for an app-only project; preserve the existing model path when present.
Bootstrap via the Rust creation API or application migrations, then attach via config.

## 8. First connector implementations

### Postgres: first complete implementation

Add an opt-in connector and application-owned connection configuration using the
existing secret resolver. Use a pooled async native client with pinned transaction
connections. Implement reads as Arrow providers and parameterized destination SQL.
Start transaction isolation support with Repeatable Read and Serializable; serialize
scans on the pinned session, collect within budgets, and report serialization failures.

Provide read-only snapshot sessions independently, using a pinned read-only transaction
and forcing initial snapshot acquisition during open. Query-scoped snapshot reads use
the same mechanism with stream-owned cleanup. For post-commit visibility, start with
receipts valid for the same configured authoritative database and fresh reads routed
there after acknowledgement. Replica waiting and verification against already open
snapshots are unsupported initially; reject those requirements rather than infer them.

For keyed merge, stage source rows in a temporary table inside the transaction,
validate using destination equality semantics, and use native keyed upserts (or a
keyed update for update-only). Require a suitable enforced, non-null unique key.
Avoid updates when assigned values are unchanged. Native constraint handling protects
concurrent key creation; unknown triggers/default effects reject checked mode.
Transactions can cover several supported tables in one database session. Generated
SQL implements the normalized plan; arbitrary user SQL is not passed through.

### Iceberg: subsequent implementation, independent milestone

Read sessions pin a table snapshot and resolve every scan from it, even when the
connector has no write support. Advertise one-table snapshot scope initially. Multiple
table snapshots are independent domains; do not label them a shared snapshot. A commit
receipt identifies the committed table snapshot; post-commit reads require verified
visibility of that snapshot or a descendant on the same branch. Missing/expired
lineage or incompatible branches reject the requirement. Never silently repin if
snapshot files disappear; retention coordination must support the session lifetime
or reads fail explicitly.

Implement the same normalized keyed merge with snapshot reads, affected-file rewrites,
and an atomic table snapshot commit. Preserve rows/columns outside the requested
update. Identifier fields are descriptive, so validate key uniqueness in the relevant
destination rows, including duplicates across files.

Start with a conservative expected-snapshot commit precondition: any intervening
destination snapshot change conflicts. Recompute from the new destination snapshot
using the same staged source input, or fail; do not auto-rebase an append. Skip the
commit when logical contents are unchanged. Return unknown outcomes honestly and
clean up unreferenced files only after establishing they are not committed/referenced.

Advertise single-table atomic merge first, not arbitrary multi-statement or multi-table
transactions. Revalidate on every run because other writers can introduce duplicates.
Use copy-on-write first; optimized conflict detection and delete-file strategies are
later work. Confirm the selected Rust library/catalog can enforce the required commit
precondition and rewrite operations before enabling the capability. A file writer
alone is insufficient; Spark's SQL support is not proof of native Rust support.

## 9. Delivery and acceptance tests

1. **Core contracts and fake connector:** bindings, parser, normalized plans,
   parameter binding, explanations, read requirements/completion reports, staging,
   outcome states, and fail-closed validation.
2. **Application writes:** Postgres read/write connector, create/attach workflow,
   parameterized reads/INSERT/UPDATE/DELETE, independent snapshot read sessions,
   transaction lifecycle, commit visibility, and cache policy/reporting.
3. **Checked reconciliation:** restricted MERGE, CLI entry points, Postgres conformance,
   and a runnable external-issues/application-assignments example using mock source data.
4. **Second commit model:** Iceberg conformance for pinned table reads/commit visibility,
   forced snapshot conflicts, and crashes/acknowledgement loss. Enable capabilities
   only after their guarantees pass the same suite.

Required tests at the relevant milestone:

- Existing read-only loading/query/compiler behavior remains unchanged.
- Observed joins across changing sources work without claiming shared snapshots;
  unsupported Snapshot requests reject before any source row scans.
- Read-only connectors can pin snapshots without write privileges. Self-joins, views,
  aliases, pushdown, repeated queries, and local residuals use the same pin. Independent
  domains reject strict reads; opt-in external reads are reported separately.
- Read-session expiry, close, stream abandonment, source retention loss, and schema
  invalidation release resources and never substitute a fresh snapshot silently.
- Post-commit reads include supplied commits or fail. Cover lagging readers, waiting
  deadlines, missing evidence, wrong domains, and receipts newer than a pinned session.
- Configured/Bypass/MaxAge apply transitively; stronger reads bypass caches; reports
  name actual generations and ages. Expired-cache refresh failure cannot return stale
  success. Cache bypass alone must not be reported as verified commit visibility.
- Streams transition from Pending to the correct terminal state after success, late
  error, cancellation, or drop. No partial collect success, hidden whole-query retry,
  duplicated batches, or snapshot-resource leaks. EXPLAIN acquires no snapshot or rows.
- Parser handles comments, quoting, case, parameters, trailing semicolons, and rejects
  multiple statements or unsupported modifier combinations without writes.
- Deterministic keyed merges insert/update once; replaying identical staged input
  preserves logical rows; changed source input updates only declared fields.
- Update-only, insert-only, composite keys, empty input, null/duplicate keys, type
  conversions, and destination comparison semantics have explicit coverage.
- Target feedback through aliases/views, unsafe expressions, unsupported defaults/
  triggers, key-changing updates, and unsupported capabilities reject safely.
- Late source failures and staging budget exhaustion leave destination data unchanged.
- Concurrent key creation, schema change, and snapshot conflict either preserve the
  guarantee or abort. Inject failures before/after commit acknowledgement and verify
  outcome classification; no blind retries or unsafe cleanup.
- Multiple writes in a transaction commit/rollback together; reads see own writes;
  independent requests cannot see uncommitted data or share transaction state.
- External reads require opt-in inside transactions; isolation cannot be weakened
  through query splitting; writable aliases/dependent views cannot serve stale caches.
- Ordinary non-atomic writes require explicit opt-in and report partial failures;
  checked writes and explicit transactions reject that mode.
- Failed table registration after physical creation reports recoverable partial
  bootstrap state; project loading never performs DDL.

Use connector conformance tests with real backends plus fault-injecting fake sessions;
SQL text snapshots alone cannot verify these guarantees. Run relevant crate tests and
the workspace regression suite when implementing. No runtime code changes accompany
this design document.

## References

- [DataFusion TableProvider](https://docs.rs/datafusion/55.0.0/datafusion/catalog/trait.TableProvider.html): native mutation extension points; not a transaction coordinator.
- [Postgres INSERT](https://www.postgresql.org/docs/18/sql-insert.html): conflict handling and backend side effects.
- [Postgres isolation](https://www.postgresql.org/docs/18/transaction-iso.html): transaction snapshots and serialization failures.
- [Iceberg specification](https://iceberg.apache.org/spec/): identifier fields do not enforce uniqueness.
- [Iceberg reliability](https://iceberg.apache.org/docs/1.10.2/reliability/): atomic publication and validation of commit assumptions.
- [Iceberg writes](https://iceberg.apache.org/docs/1.10.2/spark-writes/): merge through affected-file rewriting in the Spark implementation.
- [Iceberg implementation status](https://iceberg.apache.org/status/): verify library-specific capabilities before implementation.
