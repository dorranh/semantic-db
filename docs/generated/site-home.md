# Query data with its meaning attached

Semantic DB combines an Ossie semantic model with DataFusion SQL execution.
Use it from Rust or the CLI to query files, databases, and API-backed relations
through one catalog.

> Experimental software. APIs and configuration may change; packages are not
> published yet.

## Start with a working dataset

With Rust and [just](https://just.systems/man/en/installation.html) installed,
launch the bundled geospatial example from the repository root:

```sh
just repl
```

Then query the synthetic wells:

```sql
SELECT well_name, basin, total_depth_m
FROM wells
WHERE total_depth_m >= 2500;
```

The example runs locally without credentials. Natural-language queries require
an OpenAI-compatible provider configuration; see the [CLI guide](../cli.md).

## Build your integration

| Your next step | Guide |
| --- | --- |
| Connect a dataset and describe its meaning | [Add a dataset](../adding-datasets.md) |
| Use the engine in a Rust application | [Embed Semantic DB](../embedding.md) |
| Bring an API or another backend | [Build a connector](../building-connectors.md) |
| Check physical types and extension points | [Types and extensibility](supported-types.md) |
| Compare semantic and database architectures | [Where Semantic DB fits](database-comparison.md) |
| Understand transfer, joins, and cache costs | [Performance and federation](performance.md) |

## Understand the boundary

Business descriptions guide interpretation; they do not create executable
functions, enforce keys, or guarantee the meaning of generated SQL. The
[Ossie reference](../ossie-reference.md) documents the supported model subset.
Cross-source reads do not share a transactional snapshot.

The navigation separates the existing guides from generated guides and
engineering notes. Historical assessments describe their original implementation
stage. For current types and execution costs, start with the guides above.
