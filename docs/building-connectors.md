# Build a connector

A connector supplies data. Ossie supplies semantic names and definitions. Use a
DataFusion `TableProvider` for execution and a `ConnectorFactory` to make it
available to configured CLI and embedded users. Dataset users should only need
to add model entries and bindings after your connector is registered.

## Choose the smallest implementation

| Starting point | Implement |
| --- | --- |
| An existing compatible DataFusion provider | A factory and connection wrapper; return that provider. |
| A paginated API | A lazy page stream producing Arrow batches, initially wrapped in `StreamingTable`. |
| A correct scan that needs fewer remote requests | A custom `TableProvider` translating a proven subset of predicates; reuse the existing stream. |

`RelationBackend` is useful for applications supplying their own catalog and
Arrow schemas. It is not required for an Ossie connector: the importer obtains
physical schemas from the providers your factory returns.

Use DataFusion 55 and the matching Arrow types. The `semantic-db` facade re-exports
both. The provider API is version-sensitive; compile your adapter with the same
dependency versions as its host.

## Run the complete template

From the repository root:

```sh
cargo run -p example-custom-connector -- \
  --config examples/connectors/semantic-db.yaml --inspect
cargo run -p example-custom-connector -- \
  --config examples/connectors/semantic-db.yaml \
  --query 'SELECT id FROM items WHERE id >= 4 LIMIT 1'
cargo test -p example-custom-connector --locked
```

The query returns ID 4. The [template](../examples/connectors/src/main.rs)
is a small custom CLI binary backed by five simulated API items, delivered two
at a time. It uses the same configuration loader, Ossie importer, SQL/NL commands,
and REPL as the standard executable. Replace `PageStream::fetch_page` with your
API-specific HTTP operation. The demo intentionally has no authentication or
real network transport; use the GitHub implementation for those details.

Its tests verify lazy planning, fresh scans, stream drop, a residual filter that
needs later pages despite `LIMIT 1`, and result equivalence to an independently
constructed local table.

## Wire up configuration once

The [registry interfaces](../crates/semantic-sources/src/lib.rs) separate these jobs:

1. `ConnectorFactory::validate_connection` and `validate_source` parse and check
   options without network access, provider construction, or secret resolution.
   Reject unsupported/unknown options and invalid required scope early.
2. `ConnectorFactory::connect` resolves named secrets through the supplied resolver
   and creates a reusable connection/client. Do not read environment variables
   inside the connector; the host owns that policy.
3. `SourceConnection::table` resolves one collection/table to a provider. Obtain
   its schema from metadata or an explicit API schema. Defer query rows to execution.
4. Register the factory once with `Registry::register("my_api", factory)`. Config
   files can now select it through `connections.*.connector`.

The loader validates all configuration first, constructs only selected-model
sources, reuses connections and source providers, and performs the final Ossie
column/type checks. Relative file paths are supplied through `base_dir`.
Sanitize connector errors because the CLI prints them with connection/source paths.

For a custom CLI, depend on the `semantic-cli` library package and call:

```rust,ignore
let mut registry = semantic_sources::Registry::standard();
registry.register("my_api", MyConnector)?;
semantic_cli::run_with_registry(registry).await?;
```

For embedding, pass that registry to `Project::load`; see [embedding](embedding.md).
The stock binary only knows compiled/registered factories. There is no dynamic
Rust plugin ABI or runtime discovery. No engine modification is needed for an
ordinary provider; adding a builtin to the standard distribution is a separate
registration/dependency change.

## Implement pagination before optimization

Use `StreamingTable` with a `PartitionStream` for the baseline. Declare a stable
Arrow schema, and create fresh page/cursor state in `execute` for each scan.
Fetch a page only when the returned stream is polled; yield bounded batches.
Client pools can be shared, but pagination state must not leak across executions.

For GraphQL, define the row contract explicitly: collection, required arguments,
node-to-column mapping, types, nullability, and row grain. A child connection
usually needs its own relation and cursor per parent. An issue-label table has
one row per association and no row for an unlabeled issue. Introspection alone
cannot decide these meanings, required scope, or efficient pagination.

Continue based on the API's continuation contract, including empty pages that
still have a continuation cursor. Reject repeated/missing cursors, partial
GraphQL `data` plus `errors`, malformed values, inaccessible scope, and exhausted
budgets. Set request/time/response-size caps. Do not turn a cap or API error into
successful end-of-stream. Implement explicit retry policy if needed; avoid
unbounded retries and detached work after cancellation.

The [GitHub page stream](../crates/semantic-github/src/lib.rs) demonstrates nested
connections, null authors, UTC timestamps, bounded requests, redacted errors,
and cancellation by dropping the stream. It makes no row requests during
construction or planning. Metadata requests may be needed by other connectors;
document these separately from query execution.

## Add pushdown with an equivalence proof

Start with a predicate that avoids actual source work. The GitHub `IssueTable`
implementation handles direct `state = 'OPEN'` / `state = 'CLOSED'` equality and
passes a typed GraphQL variable to `issues(states: ...)`. It delegates projection
and safe limits to a `StreamingTable` over the existing page stream. It does not
need a new physical execution node just to support this predicate.

DataFusion calls `supports_filters_pushdown` per expression:

| Result | Contract |
| --- | --- |
| `Unsupported` | Leave the expression to DataFusion. |
| `Inexact` | May return extra rows, but must retain every qualifying row; DataFusion rechecks. |
| `Exact` | Return precisely the qualifying rows under SQL semantics; the local filter may be removed. |

Match the expression tree, supported operators, literal types, and columns.
Do not translate by formatting SQL strings. Check NULL semantics, case/collation,
timezones, casts, and compound predicates. A source's approximate search endpoint
is not an exact SQL predicate. Use GraphQL variables or bound database parameters.

Only honor limits supplied safely by the planner. Never interpret SQL `LIMIT 10`
as a cap of ten raw remote rows when residual predicates still need evaluation.
Projection may omit a column used by an exact pushed filter; retain fields you
need internally. An empty projection still needs correct row counts. Advertise
ordering only when every returned batch/partition actually satisfies it.

GitHub's `filter_pushdown: false` provides a baseline for comparison. Its
[tests](../crates/semantic-github/tests/query.rs) compare optimized, baseline, and
local results through Ossie, including a renamed state field, while asserting
that three baseline requests become two optimized requests on the fixture.
Repository pruning, remote field selection, aggregate/join pushdown, and shared
query budgets remain separate work.

See Apache DataFusion's [provider guide](https://datafusion.apache.org/library-user-guide/custom-table-providers.html)
for the extension model and GitHub's [repository schema](https://docs.github.com/en/graphql/reference/repos#repository)
for the `issues` API. Use this repository's pinned source for exact Rust signatures.

## Conformance and delivery

Use `semantic_sources::conformance::check_query_equivalence` with engines loaded
through the same Ossie model: one remote fixture provider, one independently
constructed local reference. This checks schemas and rows across differing batch
boundaries. Give multirow queries an explicit `ORDER BY` for deterministic order.
For pushdown, also compare with optimization disabled and measure remote work.

Cover the following with local fixtures:

- Multiple pages, empty sources/pages, nested cursors, repeated execution, and cancellation.
- NULL values, physical types, timestamps, aliases, projection, and `COUNT(*)`.
- Exact/inexact/unsupported filters, predicates on unselected fields, residual
  filters plus limits, sorting, joins, and duplicate-row multiplicity.
- Missing/invalid scope, permissions, transport/API/partial-data errors, malformed
  continuation state, and exhausted budgets—including failures after earlier batches.

Deliver the connector with a configuration example, Ossie model, SQL and expected
results, documented capabilities and limits, and an optional bounded live smoke
test separate from deterministic CI. A streaming consumer must invalidate an
answer if a later page fails; a live API scan does not imply snapshot isolation.
