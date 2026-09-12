# GitHub issues, labels, and local teams

This dataset exposes live repository-scoped GitHub issues and issue-label
associations through Ossie, and joins them to an authored local team CSV.
Use the standard CLI; edit scope and execution limits in
[semantic-db.yaml](semantic-db.yaml).

## Run

From the repository root, set `GITHUB_TOKEN` in the environment or working
directory's ignored `.env`. Use read access to the configured repositories.
The default scope is `apache/ossie`; change `connections.github.repositories`
to use another set. Credentials stay out of the Ossie model and project file.

```sh
# Offline: no token, provider construction, or API requests.
cargo run -p semantic-cli -- --config examples/github/semantic-db.yaml --inspect

# Count open issues by local team; unmapped repositories remain visible.
cargo run -p semantic-cli -- --config examples/github/semantic-db.yaml \
  --file examples/github/open_issues_by_team.sql

# Interactive SQL and natural language.
cargo run -p semantic-cli -- --config examples/github/semantic-db.yaml

# Many-to-many label aggregation; can require substantially more requests.
cargo run -p semantic-cli -- --config examples/github/semantic-db.yaml \
  --file examples/github/open_issues_by_label.sql

# Plan SQL without GitHub row requests.
cargo run -p semantic-cli -- --config examples/github/semantic-db.yaml \
  --query "SELECT number FROM issues WHERE state = 'OPEN' LIMIT 10" --dry-run

# Uses OPENAI_* configuration as well as GITHUB_TOKEN.
cargo run -p semantic-cli -- --config examples/github/semantic-db.yaml \
  --ask "Count open issues by team and repository, including unmapped repositories"
```

The REPL supports `.tables`, `.schema issues`, `.view`, `.ask`, `.plan`, and `.quit`.
End SQL with a semicolon. `--validate --connect` checks provider/model schema
compatibility but makes no GitHub requests and does not verify row permissions.
SQL-only use needs no model key. `--ask ... --dry-run` still calls the model.

Source paths inside the project resolve relative to its directory. To replace
the local mapping, change `sources.local.repository_teams.path`. Keep one row
per repository and validate uniqueness ignoring case before relying on counts;
the Ossie key is descriptive and the shared CSV loader does not enforce it.
The supplied team names are examples, not actual GitHub organization teams.

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


## Execution and limits

See the [connector reference](../../docs/connectors.md#github) for every option
and the execution contract. Exact `issues.state` equality to `'OPEN'` or
`'CLOSED'` can reduce remote pages. Other filters, ordering, aggregation, and joins
execute locally. GraphQL field selection is fixed. `issue_labels` independently
enumerates issues and pages their labels; joins do not share scans or push issue
filters into label scans. This can cause N+1 requests, even for unlabeled issues.

Defaults are 100 items/page, 100 requests per scan, 30 seconds/request, and 4 MiB
per response. Exhaustion and partial GraphQL responses fail rather than returning
successful partial results. There is no shared query budget, retry policy, or
transactional snapshot. `LIMIT` is not a promise about remote request count.

"Stale", "urgent", and "healthy" deliberately have no executable definition.
The model should request clarification; this is not a deterministic guarantee.
Use explicit criteria in a query or a curated SQL view.

## Embed and verify

The compact [Rust example](../../crates/semantic-db/examples/github.rs) loads
this same project and executes its default query. It reads process environment
variables only; export `GITHUB_TOKEN` before running it:

```sh
cargo run -p semantic-db --features sources,github --example github
cargo test -p semantic-github --locked
```

The offline tests compare API fixture rows with independent local SQL tables
through Ossie. They cover pagination, nulls, timestamps, joins, projection,
residual filters and limits, cancellation, budgets, partial errors, and exact
pushdown. The state-filter fixture uses two requests with pushdown versus three
without. Live row counts change and are not correctness fixtures.

The former standalone GitHub example's `--repo`, `--teams`, `--page-size`,
`--max-requests`, and `--repl` workflow is replaced by project configuration and
the standard CLI. See [CLI reference](../../docs/cli.md) for batch/REPL modes and
[experiment findings](../../docs/github-experiment.md) for remaining architecture work.
