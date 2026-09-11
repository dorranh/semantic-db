# GitHub GraphQL architecture experiment

Live GitHub issues and labels are exposed as DataFusion tables, imported through
Ossie, and joined to an authored local team CSV. This uses the existing provider
and source-binding interfaces without engine changes. It establishes a tested
correctness baseline; remote optimization is future work.

## Run

From the repository root, set `GITHUB_TOKEN` in the environment or ignored `.env`
file. Use read access to the selected repositories. Never put credentials in the
Ossie model. See GitHub's [authentication guide](https://docs.github.com/en/graphql/guides/forming-calls-with-graphql).

```sh
# Default: apache/ossie; count open issues by locally assigned team.
cargo run -p semantic-db --features ossie,github --example github

# Interactive SQL and natural-language queries over the same scoped tables.
cargo run -p semantic-db --features ossie,github --example github -- --repl

# Repeat --repo for multiple repositories.
cargo run -p semantic-db --features ossie,github --example github -- \
  --repo apache/ossie --repo apache/datafusion \
  --query "SELECT repository, number, title FROM issues WHERE state = 'OPEN' LIMIT 10"

# Many-to-many query; more requests than an issue scan.
cargo run -p semantic-db --features ossie,github --example github -- \
  --file examples/github/open_issues_by_label.sql

# Plan without GitHub row requests.
cargo run -p semantic-db --features ossie,github --example github -- \
  --query "SELECT COUNT(*) FROM issues" --dry-run

# Also uses the existing OPENAI_* configuration.
cargo run -p semantic-db --features ossie,github --example github -- \
  --ask "Count open issues by team and repository, including unmapped repositories"

# Semantic probe: stale is deliberately undefined; expect clarification.
cargo run -p semantic-db --features ossie,github --example github -- \
  --ask "Which teams have stale issues?" --dry-run
```

Environment variables take precedence over `.env`. SQL mode needs no model key.
The example prints scope and attempted request count, and collects results before
printing them. A later failure cannot appear as a successful partial table.
`--dry-run --ask` still calls the model. Plain `--dry-run` constructs a logical
plan; it does not prove permissions or estimate network cost.

`--teams PATH` replaces the local mapping. It requires `repository,team` headers
and one row per repository, unique ignoring case. The supplied teams are example
labels, not actual GitHub organization teams. Unmapped repositories remain
visible in the default query.

In the REPL, SQL can span lines and runs when a line ends with `;`:

```text
github> .tables
github> .schema issues
github> SELECT repository, number, title FROM issues WHERE state = 'OPEN' LIMIT 10;
github> .view open_issues=SELECT * FROM issues WHERE state = 'OPEN'
github> .ask Count open issues by team and repository
github> .plan Which teams have stale issues?
github> .quit
```

`.ask` executes grounded SQL; `.plan` only compiles it. Both call the configured
model and initialize it on first use, so SQL-only sessions need no OpenAI key.
Query/command errors leave the session open. Ctrl-C clears unfinished input;
Ctrl-D exits. History is session-local. Requests are reported per query, and
each query gets fresh scans with the configured per-scan budgets. Add `--repo`
to the launch command to change scope. `--repl` cannot be combined with batch
`--query`, `--file`, `--ask`, or `--dry-run` options.

## Model contract

| Relation | Grain | Key |
| --- | --- | --- |
| `issues` | One issue in configured scope; excludes pull requests | `issue_id` (global node ID) |
| `issue_labels` | One issue-label association; no row for unlabeled issues | `issue_id, label_id` |
| `repository_teams` | One authored assignment per repository | `repository`, compared ignoring case |

Issue numbers are only unique within a repository. Names are canonical GitHub
owner/name values. Timestamps use Arrow UTC milliseconds; author and closure
time can be null. Joining labels multiplies issue rows. Use distinct issue IDs
for totals, and do not sum per-label counts to obtain total issues.

The model uses Ossie's supported identity-field profile. Joins and grain are
descriptive; executable relationships and metrics remain unsupported. Declared
keys produce warnings and are not enforced. "Stale", "urgent", and "healthy"
need explicit definitions. Clarification is model-driven, not deterministic.

The optional `github` facade feature exposes `GitHub` and `GitHubConfig`:

```rust,ignore
let github = GitHub::new(GitHubConfig::new(token, repositories))?;
sources.bind("github.scoped.issues", github.issues()?)?;
sources.bind("github.scoped.issue_labels", github.issue_labels()?)?;
```

SQL-only library consumers can use `default-features = false` with features
`ossie,github`. This command-line example also requires the default compiler.

## Execution contract

- Construction and planning make no GitHub requests. Polling scans fetches
  pages and yields Arrow batches. Dropping a stream drops pending I/O; there
  are no background tasks or prefetch requests.
- Repositories are scanned sequentially. Each connection has independent cursor
  state. Repeated/missing continuation cursors fail. Duplicate configured scope
  entries and aliases resolving to the same canonical repository fail.
- GraphQL queries fetch fixed issue fields. SQL projection, filters, ordering,
  aggregation, and joins run locally. No remote filter/aggregate/join pushdown
  or dynamic repository pruning is implemented.
- DataFusion can stop a scan for a safe limit. Residual filters may require more
  pages; `LIMIT 10` is not a promise to fetch only 10 remote rows. Counts and
  global sorts generally require the full scope.
- Labels independently enumerate issues and paginate each issue's labels by
  node ID. This exposes N+1 requests, including requests for unlabeled issues.
  Issue enumeration is not shared with concurrent issue scans.
- Defaults: 100 items/page, 100 requests **per scan**, 30 seconds/request, and
  4 MiB/response. Use `--page-size` and `--max-requests` for the first two.
  Exhaustion fails rather than truncating. Joined scans have separate budgets.
- HTTP errors, GraphQL errors (including partial HTTP 200 responses), invalid
  data, and inaccessible repositories fail. No automatic retries or rate-limit
  sleeps. Errors omit tokens and response bodies. HTTPS is required except for
  loopback test servers, and redirects are disabled.
- Pagination is not a transactional snapshot. Changes during/between scans can
  affect joins. Streaming consumers may see batches before an error; they must
  invalidate incomplete answers if a later page fails.

GitHub documents [pagination](https://docs.github.com/en/graphql/guides/using-pagination-in-the-graphql-api),
[rate/resource limits](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api),
and the [issue schema](https://docs.github.com/en/graphql/reference/issues).
Our request budget is not a GitHub query-point budget.

## Validation

```sh
# Deterministic local HTTP fixtures; no external API or token needed.
cargo test -p semantic-github --locked

# Optional one-request live smoke check; reads process environment only.
cargo test -p semantic-github --locked -- --ignored --nocapture
```

Remote results are compared with independent local SQL fixtures through the same
Ossie import path. Checks cover multi-page issues/labels, null authors, UTC
conversion, ownership joins, count multiplicity, filters plus limits, sorting,
empty-column counts, lazy planning, stream drop, budgets, HTTP/GraphQL errors,
malformed data, byte limits, timeouts, and cursor failures.

Observed live checks on September 11, 2026 (not stable expected row counts):

| Probe on `apache/ossie` | Result | HTTP requests |
| --- | --- | --- |
| One issue, page size 1, `LIMIT 1` | Issue 52, OPEN | 1 |
| Open issues joined to local team mapping, default page size | 47 open issues, Semantic models | 1 |
| Same aggregation with page size 2 and request cap 20 | Explicit budget error; no successful partial count | 20 |
| Label/team aggregation with page size 2 and cap 30 | Explicit budget error | 30 |
| Label/team aggregation with default settings | Completed, zero groups | 63 |

The zero-group result still required many requests. The deterministic fixture
provides nonempty label joins: 2 requests for an issue scan, 6 for the label
scan, and 8 for their join. Its compiler test verifies metadata delivery,
grounded SQL execution, and clarification without GitHub I/O using a fake model.
The separately approved live model probe for "Which teams have stale issues?"
returned `needs_clarification` in one attempt, asking for the age cutoff,
reference time, and time field. It made no GitHub row requests. This observed
response is not a deterministic guarantee of future model behavior.

## Architecture findings

1. **Provider integration works without engine changes.** Ossie's field
   projection preserves execution through the provider. Result equivalence
   through the importer is now tested, beyond successful HTTP requests.
2. **Scope lacks a typed catalog contract.** Required API arguments live in
   connector configuration. The compiler cannot validate requested repositories
   against actual scope or estimate cost. Add inspectable scope/capabilities.
3. **Join knowledge is descriptive.** Relationships, uniqueness, and metric grain
   are not enforced. Duplicate team mappings can inflate counts. Validated
   cardinality and a metric correct across label joins are useful next steps.
4. **Federation does not imply efficient remote execution.** Local predicates
   and join keys do not prune scans. Add exact repository/state pushdown and
   remote field selection with equivalence tests before batched dependent joins.
5. **Execution policy needs query scope.** Provider-local caps cannot coordinate
   multiple scans. Shared deadlines, cancellation, request/point budgets, and
   tracing need a contract above connectors. Request totals currently belong
   to the client lifetime, not individual EXPLAIN operators.
6. **Consistency and provenance need explicit semantics.** Live tables have no
   shared snapshot. Caching will need credential/scope-aware keys, freshness
   policies, and provenance that distinguishes partial from complete results.

These motivate specific contract extensions before a universal GraphQL-to-Ossie
schema translator.
