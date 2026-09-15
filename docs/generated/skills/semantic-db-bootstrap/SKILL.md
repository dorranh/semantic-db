---
name: semantic-db-bootstrap
description: "Bootstrap a Semantic DB project from local files, PostgreSQL, or ClickHouse and verify SQL and authored-view Ask."
---

# Bootstrap a Semantic DB project

Find the user's source locations and intended business questions. The standard binaries support PostgreSQL, ClickHouse, CSV, newline-delimited JSON, Parquet, Avro, and Arrow IPC files. Local file directories are supported; cloud/HTTP storage and custom API connectors require separate development. Do not interpret an Ossie source key as a path or connection URL.

Use `semantic-db --version` and `semantic-server --version` to check the installed release. Run `semantic-db init DIRECTORY` to create a working starter; initialization refuses existing scaffold files. For an existing project, inspect and extend its current configuration. Do not replace existing models to obtain a fresh scaffold.

For local files, start with this shape in `semantic-db.yaml`:

```yaml
ossie: model.ossie.yaml
connections:
  local:
    connector: file
sources:
  local.products:
    connection: local
    path: data/products.csv
```

The Ossie dataset's `source` must equal `local.products`. Paths resolve relative to the project config. File format is inferred from extensions; for directories specify `format` and the relevant `extension`. CSV and JSON reader types are guided by mapped Ossie field declarations. Do not duplicate those declarations in a physical schema; overrides are for missing physical details such as decimal precision/scale.

For databases, use the installed release's connection options and credential environment references. PostgreSQL currently requires `sslmode=disable`; explain that constraint if the user's source requires TLS. Do not silently weaken a source's required transport policy. If a connector is missing, explain the source-build path before doing connector development.

Populate descriptions, mappings, units, and row grain. Author SQL views for meaningful business definitions. The `semantic-db-model` skill can help if installed, but this workflow must work independently. Ask for undefined thresholds instead of inventing them.

Run `semantic-db --config semantic-db.yaml --validate`, then `--validate --connect`, then a bounded SQL query with an expected result. Connection validation does not prove row access or validate all data. Keep credentials in environment/.env, and example values in `.env.example`.

For Ask, set `OPENAI_API_KEY`, `OPENAI_MODEL`, and optionally `OPENAI_BASE_URL`. Preview a representative request with `--ask-views QUESTION --dry-run`, then run it. SQL is usable without model configuration. Explain SQL/evidence, clarification, and unsupported outcomes. General `--ask` has different grounding guarantees.

Finish with the working project, source limitations, verified SQL, and representative questions. Offer the same config to the REPL or `semantic-server --config semantic-db.yaml`; applications execute SQL through PostgreSQL and compile Ask over HTTP. Do not claim production readiness or publish the project as part of bootstrap.
