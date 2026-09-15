# Reads, writes, transactions, and reconciliation: strategy

Status: agreed direction; proposed behavior, not a description of implemented features.

This document defines the connector-neutral product contract. The companion
[technical design](writes-and-reconciliation-design.md) describes implementation
boundaries, initial restrictions, and delivery stages.

## Purpose

An application should be able to bring its own storage, create and register its
application tables, and join them with the other relations in a semantic database.
It should be able to write application data through parameterized SQL and request
guarantees that the system either establishes or explicitly rejects.

Storage provisioning and production schema migrations remain application-owned.
Semantic DB provides table registration, a capability-gated table creation path,
federated queries, writes, and repeatable reconciliation. It does not become a
distributed transaction coordinator or an application migration framework.

## Commit boundaries, not connector names

**Semantic DB exposes and enforces connector-provided write guarantees. It does
not coordinate distributed transactions across independent commit boundaries.**

A commit boundary is the set of resources that can participate in one atomic
commit. Depending on the connector and its configuration, that may encompass a
database, a table, or one file/object. Sharing a connector type, endpoint, or
credential is not evidence of sharing a transaction.

Each connector reports supported operations and identifies the concrete boundary
for the requested targets. Semantic DB validates the whole operation against that
boundary before changing destination data. It never silently converts an atomic
request into several independent commits.

| Capability | Meaning |
| --- | --- |
| Writable | The connector can perform the requested mutation. |
| Atomic operation | The supported operation publishes all its destination changes together or none of them. |
| Multi-statement transaction | Several supported statements can share one commit/rollback lifecycle. |
| Read isolation | Participating reads have the specific isolation guarantee established by the connector. |
| Checked idempotent write | Repeating the same input has no additional logical data effect under the documented conditions. |

These capabilities are separate. Atomic publication does not imply multi-statement
transactions, uniqueness enforcement, durable recovery under every failure, or a
coordinated read snapshot. The connector must state its guarantees and limitations.
Read-only connectors remain valid members of a semantic database.

## Read guarantees

**Semantic DB provides federated reads by default and enforces stronger read
guarantees where participating connectors can establish them. Freshness, snapshot
consistency, and write visibility are separate requirements.**

| Read use case | Contract |
| --- | --- |
| Ordinary federated query | Sources are observed independently. One SQL statement does not imply one consistent snapshot, even across repeated scans of one source. |
| Snapshot query | Every participating scan uses one connector-established consistent snapshot, or the request is rejected. |
| Read session | A supported snapshot stays pinned across several queries until the session closes or expires. |
| Read own writes | Reads participating in a write transaction see its uncommitted changes, under its established isolation. |
| Read after commit | An explicit visibility requirement establishes that the read includes a known commit, or the request fails. |
| Cached query | Uses eligible cached generations under the selected policy and reports their age; cache age is not a bound on upstream data staleness. |
| Streaming query | Delivered rows are provisional until the stream completes successfully; failure or early abandonment is not a complete result. |

A read domain identifies resources for which a connector can establish one shared
snapshot. It is independent of a commit boundary: snapshot reads do not require
write permission or write support. Domain identity and compatible session support
must come from the connector, not matching connection names.

Explicit read sessions are strict by default. External observations require opt-in
and remain outside the pinned snapshot. Pinning separate snapshots independently
does not make a coordinated snapshot across sources. Coordinated snapshots across
independent read domains are outside the initial scope.

A pinned snapshot stabilizes source data, not arbitrary query output: clock/random
functions can still change, and row ordering requires explicit ORDER BY. Session
lifetime and source retention limit snapshot availability. An unavailable snapshot
causes failure rather than silent substitution with a newer snapshot. Historical
version selection/time-travel syntax is deferred; initial sessions pin a current
snapshot that the connector establishes.

Ordinary reads do not implicitly wait for a previous write. For explicit post-commit
visibility, the connector routes to a suitable reader, waits within the query budget,
or rejects the requirement. Bypassing a cache alone does not eliminate replica lag.
An existing older snapshot cannot silently advance to satisfy a newer commit.
Including a commit does not prevent subsequent writes from changing its values.

Callers can use configured caching, bypass caches, or impose a stricter maximum
cache age. Bypass means read the source directly, not that the source itself is
current or snapshot-consistent. Result reports identify observed/pinned sources,
used cache generations, fulfilled guarantees, and terminal completion status.

## Reads and writes in one application

A transaction can read its own writes only when its reads actually participate in
the backend transaction. Ordinary pooled reads or cached copies are insufficient.
An isolation request must be honored or rejected; query splitting must not weaken
it silently.

External reads can supply input to atomic destination writes. Those reads do not
join the destination transaction. External data may change during a scan or between
the scan and commit. Rolling back the destination does not roll back a source.

The default for explicit transactions is participating reads only. External reads
require an explicit option, and the resulting guarantee report identifies them.
Standalone reconciliation explicitly uses external observations as input; it does
not claim a shared source/destination snapshot.

## Reconciliation and eventual convergence

The application expresses desired destination values derived from source data.
A reconciliation run observes the sources, computes desired rows, and applies
them through one supported atomic destination operation. Repeated runs can bring
application state into agreement with its sources even when an individual run
observes data that becomes stale before commit.

Convergence is conditional on all of the following:

- Stable record identities and a rule that updates the fields being synchronized.
- Successful runs continuing to observe the relevant source changes.
- Defined handling of deletions and records leaving the source query's filter.
- No treatment of failed or partial source reads as authoritative absence.
- Serialization or version checks preventing older observations from overwriting newer ones.
- Ownership rules preventing competing writers from continually undoing reconciliation.

After sources stabilize, a subsequent complete run can bring the synchronized
fields into agreement. Continuously changing or unavailable sources imply no fixed
convergence deadline. Scheduling, retries across application restarts, and maximum
staleness are application responsibilities in the initial scope.

Application-owned fields are preserved unless explicitly included in the write.
A join alone does not synchronize stored data; reconciliation includes a mutation.

## Checked idempotency

The proposed SQL modifier is:

    REQUIRE IDEMPOTENT
    MERGE INTO ...

It is a requirement the engine checks, not an assertion the author makes.
The initial supported operation is a restricted deterministic keyed merge.

For fixed source input S and bound parameters P, let F update destination state D.
The logical guarantee is:

    F(F(D, S, P), S, P) = F(D, S, P)

This assumes no intervening writes and that the verified schema, connector, and
side-effect conditions continue to hold. A fresh reconciliation run can observe
different input and intentionally produce a different destination state.

The guarantee covers logical persisted application data, including relevant
backend effects that the connector can account for. It does not promise identical
row counts in responses, physical files, snapshot IDs, transaction IDs, or storage
maintenance records. Unknown application-visible triggers or other hidden effects
prevent strict acceptance.

Idempotency does not establish convergence: an insert that ignores existing keys
can be idempotent while never propagating updates. It also does not establish
exactly-once execution, ordering between runs, or cross-source consistency.

Semantic DB accepts supported patterns and rejects patterns it cannot establish
as safe. Rejection means "not established by this implementation," not necessarily
"mathematically non-idempotent." Some requirements, such as input key uniqueness,
are validated at runtime before destination changes become visible.

## Failure and developer experience

- Unsupported operations, boundaries, or requested guarantees fail explicitly.
- Atomic writes are required by default. Ordinary writes may explicitly opt into
  a documented non-atomic mode; checked idempotent writes cannot.
- A failed source scan cannot publish a partial reconciliation result.
- A lost commit acknowledgement can leave an **unknown outcome**. The system must
  not report rollback merely because the caller observed a timeout.
- Explanation output distinguishes static checks, runtime checks, the actual
  read/commit boundaries, external observations, and unsupported capabilities.
- Fixed-input retries reuse staged input. Refetching sources is a new run.
- Read retries cannot silently duplicate or mix already delivered rows. A caller
  retry starts a new execution; its observations may change unless a session pins them.

## Initial product boundary

Deliver ordinary parameterized reads and writes, connector-scoped snapshot read
sessions and write transactions, and checked keyed reconciliation. Define cache,
post-commit visibility, and streaming completion contracts alongside them. Keep the
query and natural-language compilation paths read-only. Add capabilities incrementally
as connectors can implement and verify them.

Distributed commit, coordinated cross-domain snapshots, global serializability,
historical version selection, automatic scheduling, automatic
deletion reconciliation, arbitrary SQL proofs, and durable exactly-once job
execution are outside the initial scope. None is implied by the SQL modifier.
