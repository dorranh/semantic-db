# Connectors and project configuration

The standard CLI includes CSV and experimental GitHub. Rust applications enable
the facade's `sources` feature, plus `github` if needed. The same project loader
is used in both paths. ClickHouse, Snowflake, and a universal GraphQL connector
are not implemented.

## Project format

```yaml
ossie: ./model.ossie.yaml
model: sales                  # Optional if the document has exactly one model
connections:
  local:
    connector: csv
sources:
  sales.orders:               # Exact Ossie dataset source key
    connection: local
    path: ./orders.csv
```

YAML and JSON are accepted; duplicate keys and unknown project fields fail.
Each connector validates its own connection/source options, including rejecting
unknown options for builtins. This unpublished configuration format may evolve.

All relative model and CSV paths use the project directory. `--file` and `.env`
use the working directory. Source keys are opaque and never executed as SQL or
interpreted as a URL. Connections may be reused by several sources; sources may
be reused by several datasets. Only sources used by the selected model are
constructed, although all configured entries are checked offline for validity.
Unknown connectors fail with the list registered in the current binary.

The library accepts an explicit secret resolver and does not read environment
variables or `.env`. The CLI resolves environment variables first, then `.env` in
the working directory, without changing the process environment. Offline
inspection/validation does not read `.env` or resolve credentials. Never place
secret values in models or connection options; use secret references.

## Available connectors

| Connector | Sources and scope | Execution |
| --- | --- | --- |
| `csv` | Headered CSV path per binding | DataFusion CSV provider; schema inference reads a sample. |
| `github` | Issues and issue-label associations in an explicit repository set | Lazy GraphQL pages; exact issue-state equality pushdown; other predicates and joins/aggregates stay local. |

A connector's filter support is expression-specific. There is no global
“supports pushdown” switch that makes arbitrary SQL remote. Query limits do not
promise the same number of API rows or requests.

## CSV

Connection options: none. Source options: required `path`, a nonempty string.
Headers and schema inference use DataFusion defaults. To customize delimiters,
provide schemas, or use another format, bind an existing provider in Rust or
register a connector with explicit supported options.

## GitHub

```yaml
ossie: github.ossie.yaml
connections:
  github:
    connector: github
    token_env: GITHUB_TOKEN
    repositories: [apache/ossie]
    page_size: 100
    max_requests_per_scan: 100
sources:
  github.scoped.issues:
    connection: github
    collection: issues
  github.scoped.issue_labels:
    connection: github
    collection: issue_labels
```

The complete [GitHub project](../examples/github/semantic-db.yaml) also binds a
local team CSV. Use that file with the supplied model, which requires all three
bindings. `repositories` is connection-wide scope shared by its issue/label
providers; define another connection for a different scope.

| Connection option | Required/default | Meaning |
| --- | --- | --- |
| `token_env` | Required | Name passed to the secret resolver; token needs read access to the scoped repositories. |
| `repositories` | Required, nonempty | Unique `owner/name` values; duplicate names ignoring case fail. |
| `endpoint` | `https://api.github.com/graphql` | HTTPS endpoint, or loopback HTTP for tests. No embedded credentials, query, fragment, or redirects. |
| `page_size` | `100` | Items per API page, 1–100. |
| `max_requests_per_scan` | `100` | Hard request cap, including nested label pages; exhaustion fails. |
| `request_timeout_seconds` | `30` | Positive timeout per request. |
| `max_response_bytes` | `4194304` | Positive per-response byte cap. |
| `filter_pushdown` | `true` | Set false to compare with local residual filtering. |

Source option: `collection`, either `issues` or `issue_labels`.
Construction and planning make no GitHub requests. Rows are read when streams
are polled. Dropping a stream drops pending I/O. Each execution has new pagination
state; clients are shared but issue enumeration is not cached/shared across scans.

For `issues`, direct equality on `state` to `'OPEN'` or `'CLOSED'` (including
reversed operands) is pushed exactly to GitHub's `states` argument. Other
expressions fall back to DataFusion. Aliases in Ossie preserve this optimization.
The API still returns a fixed field selection. Label scans do not inherit issue
filters from a join and still incur nested pagination/N+1 requests.

Partial GraphQL data, inaccessible repositories, malformed pages, repeated or
missing cursors, budget exhaustion, and transport errors fail. No automatic
retries or rate-limit sleeps. Caps are per scan, not shared across a query.
Streams may yield batches before a later failure; consumers must invalidate
incomplete results. Live pagination has no shared transactional snapshot.

## Custom connectors

Register a `ConnectorFactory` with `Registry::register`, then use its name in
`connections.*.connector`. Factories validate options offline and return reusable
`SourceConnection`s; these return `Arc<dyn TableProvider>`. Start with the
[connector guide](building-connectors.md) and its runnable template.

Custom connectors are compiled Rust dependencies. The standard binary has no
runtime shared-library plugin loader. Build a small host using
`semantic_cli::run_with_registry` to reuse all CLI commands, or use
`Project::load` in an embedded application. Dependency versions are part of the
public provider API; use the workspace's DataFusion 55 and matching Arrow types.
