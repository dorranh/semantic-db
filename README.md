# Semantic DB

A semantic query engine for all your data. Give your datasets business meaning then query across them - wherever they live.

> Experimental software, developed with the help of Codex (read: vibed). APIs and configuration
> may change; packages are not published yet.

## Overview

The primary goal of semantic DB is simple: provide a single, semantically-rich interface for working with data in your applications.
Define meaning once, then query across databases, files, and APIs through a common relational interface.

It accomplishes this with the help of [Ossie](https://ossie.apache.org/) (a format for specifying semantic models) and [DataFusion/Arrow](https://datafusion.apache.org/index.html) for queries.

It natively supports federation via DataFusion's federation module along with its own set of external connectors.

A common tradeoff with a unified data interface is performance and transactional guarantees.
Semantic DB lets you start with your existing data sources and adapt your storage as your application needs faster queries or stronger guarantees.

If you are only ever working with a single database you likely should explore its native semantic features if they exist.
However, if like most of us you have to work across many disparate data sources you should give Semantic DB a go!

## Quickstart

To play around with Semantic DB locally, ensure that you have [rustup](https://rustup.rs/) and
[just](https://just.systems/man/en/installation.html) installed then build and launch the repl against
the demo dataset with:

```bash
just repl
```

Note that if you want to use natural language queries you will need to provide an API key / config for an OpenAI-compatible API.
See the [example .env](./.env.example) for a full list of config.

Once in the repl you can use SQL or natural language to explore the example semantic DB:

```
> SHOW TABLES:

> .ask one deep oil well;

semantic> .ask one deep oil well;

    SQL (validated in 1 attempt(s)):
    SELECT well_id, well_name, basin, latitude_deg, longitude_deg, total_depth_m, status FROM wells WHERE total_depth_m >= 2500 LIMIT 1
      "deep oil well" → wells.total_depth_m: deep is interpreted as total_depth_m >= 2500 metres according to model instructions
      "one" → wells: requesting a single deep well row from the wells relation
    +---------+-----------+-------------+--------------+---------------+---------------+--------+
    | well_id | well_name | basin       | latitude_deg | longitude_deg | total_depth_m | status |
    +---------+-----------+-------------+--------------+---------------+---------------+--------+
    | W-001   | Juniper-1 | North Basin | 56.12        | 2.45          | 3200.0        | active |
    +---------+-----------+-------------+--------------+---------------+---------------+--------+

> .help
```

This example DB and related config is defined in [./examples/geospatial/](./examples/geospatial/).

## Supported Data Sources

1. Any standard [DataFusion Data Source](https://datafusion.apache.org/user-guide/features.html#data-sources) (csv, parquet, avro, etc.)
1. (Experimental) ClickHouse
1. Any other external database or API that exposes relations by building a custom connector.
   1. See the [GitHub GraphQL API example](./examples/github/) for one simple example of a non-SQL backend.

## Usage

There are two main ways to use semantic DB - either directly via its CLI or embedded in your own Rust application.

Use the CLI to query a configured dataset:

```bash
just cli --config examples/geospatial/semantic-db.yaml --query "SELECT * FROM wells LIMIT 10"
```

Leave out `--query` to open the repl.

To bootstrap a new project directory use:

```bash
just cli init /path/to/new/project
```

In Rust, enable the `sources` feature to load the same config and query it through the engine:

```rust
use semantic_db::sources::{Project, Registry};

let project = Project::from_path("examples/geospatial/semantic-db.yaml")?;
let imported = project
    .load(&Registry::standard(), &|name| std::env::var(name).ok())
    .await?;
let batches = imported.engine.query("SELECT * FROM wells LIMIT 10").await?;
```

See the guides below to use your own data or embed Semantic DB in your application.

| Task                                             | Guide                                                       |
| ------------------------------------------------ | ----------------------------------------------------------- |
| Bring a dataset on an available connector        | [Add a dataset](docs/adding-datasets.md)                    |
| Implement an API or another data backend         | [Build a connector](docs/building-connectors.md)            |
| Configure CSV, GitHub, scope and credentials     | [Connector and configuration reference](docs/connectors.md) |
| Check supported Ossie constructs and diagnostics | [Ossie reference](docs/ossie-reference.md)                  |
| Use providers or the configured loader in Rust   | [Embed Semantic DB](docs/embedding.md)                      |

## Development and architecture

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Tests use fixtures and local HTTP servers; no API key is needed. Optional live
checks are ignored by default. Commit `Cargo.lock` for reproducible builds.
Licensed under the [Apache License 2.0](LICENSE). Third-party components retain their own licenses and notices.

The facade is `crates/semantic-db`. Source configuration lives in
`crates/semantic-sources`; catalog, Ossie import, execution, compiler, and GitHub
providers remain separate crates. The CLI is `apps/semantic-cli`, with binary
name `semantic-db` and a reusable library entry point for custom connectors.

See [architecture notes](docs/generated/README.md), the
[catalog comparison](docs/catalog-prior-art.md), and the historical
[Ossie integration assessment](docs/ossie-integration.md) for design context.
