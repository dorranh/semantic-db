# Semantic DB

> Experimental software, developed with the help of Codex. APIs and configuration
> may change; packages are not published yet.

Query your datasets with SQL or natural language using an **Ossie semantic model
and configured data sources**. Start with the CLI, or embed the same source loader
and DataFusion/Arrow engine in a Rust application.

Adding a dataset on an available connector takes model and configuration edits.
Adding a new connector means supplying a DataFusion provider and registering its
factory; the CLI, importer, and query workflow stay the same.

## Quickstart

Install [rustup](https://rustup.rs/). The repository pins Rust 1.94.0 for
DataFusion 55. The first build downloads and compiles a substantial dependency graph.
From the repository root:

```sh
# Inspect the model and source bindings without credentials or source I/O.
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml --inspect

# Run the fixture query: returns W-001 and W-004.
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml \
  --file examples/geospatial/query.sql

# Interactive SQL, .tables, .schema, .view, .ask and .plan.
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml
```

The project file selects an Ossie document and binds its source to a CSV:

```yaml
ossie: wells.ossie.yaml
connections:
  local:
    connector: csv
sources:
  fixtures.geospatial.wells:
    connection: local
    path: wells.csv
```

Paths inside the project file are relative to that file. Credentials stay in the
application's secret resolver; the CLI uses process environment variables and the
working directory's `.env`, with process values taking precedence.

For natural language, copy `.env.example` to `.env`, set `OPENAI_API_KEY`, and run:

```sh
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml \
  --ask "Count wells by basin"
```

SQL needs no model key. See the [CLI reference](docs/cli.md) for model settings,
dry runs, direct CSV flags, streaming limits, and compilation behavior.

## Choose your next step

| Task | Guide |
| --- | --- |
| Bring a dataset on an available connector | [Add a dataset](docs/adding-datasets.md) |
| Implement an API or another data backend | [Build a connector](docs/building-connectors.md) |
| Configure CSV, GitHub, scope and credentials | [Connector and configuration reference](docs/connectors.md) |
| Check supported Ossie constructs and diagnostics | [Ossie reference](docs/ossie-reference.md) |
| Use providers or the configured loader in Rust | [Embed Semantic DB](docs/embedding.md) |

## Query GitHub with the same CLI

The [GitHub example](examples/github/README.md) binds live repository-scoped
issues and labels plus a local team CSV. Set `GITHUB_TOKEN` in the environment or
`.env`, edit repository scope in the project file, then run:

```sh
cargo run -p semantic-cli -- --config examples/github/semantic-db.yaml \
  --file examples/github/open_issues_by_team.sql
```

Available connectors are CSV and experimental GitHub GraphQL. ClickHouse and
Snowflake are not implemented. GitHub streams pages lazily and pushes exact
issue-state equality to the API; other filters and all joins/aggregates stay
local. A working connector does not guarantee efficient remote federation.

The importer supports single-column mappings and aliases, descriptions, AI
context, and declared keys. Computed fields, relationships, metrics, and ontology
execution remain unsupported. Keys are not enforced; natural-language grounding
is model-driven and does not prove business correctness.

## Development and architecture

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo test -p semantic-cli --example custom_connector --locked
```

Tests use fixtures and local HTTP servers; no API key is needed. Optional live
checks are ignored by default. Commit `Cargo.lock` for reproducible builds.
A project license has not yet been selected.

The facade is `crates/semantic-db`. Source configuration lives in
`crates/semantic-sources`; catalog, Ossie import, execution, compiler, and GitHub
providers remain separate crates. The CLI is `apps/semantic-cli`, with binary
name `semantic-db` and a reusable library entry point for custom connectors.

See [architecture notes](docs/generated/README.md), the
[catalog comparison](docs/catalog-prior-art.md), and the historical
[Ossie integration assessment](docs/ossie-integration.md) for design context.
