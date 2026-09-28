# CLI reference

Use the [dataset guide](adding-datasets.md) to create a project configuration.
The `sdb` executable provides `repl`, `sql`, `ask`, `validate`, `inspect`,
`cache`, `server`, and `init` commands. The examples below use
`cargo run -p semantic-cli --` from the repository root; replace that prefix
with `sdb` after installation.

## Projects and validation

`--project-config PATH` loads a YAML or JSON project. Commands discover
`semantic-db.yaml` in the current directory when the option is omitted and
announce the discovered file on stderr. `sdb repl --no-project` starts an empty
session; `sdb sql --no-project 'SELECT 1'` executes without a project. Ask,
validation, inspection, cache operations, and server startup require a project.

```sh
cargo run -p semantic-cli -- inspect --project-config examples/geospatial/semantic-db.yaml
cargo run -p semantic-cli -- validate --project-config examples/geospatial/semantic-db.yaml
cargo run -p semantic-cli -- validate --project-config examples/geospatial/semantic-db.yaml --connect
```

`inspect` lists model fields, source mappings, connectors, and configured views.
`validate` checks the model, bindings, connector options, view syntax, and
dependencies offline without credentials or source providers. `validate
--connect` also checks physical schemas; it does not read query rows or prove
row access. Paths inside a project resolve relative to its configuration file.

## SQL and REPL

```sh
cargo run -p semantic-cli -- sql --project-config examples/geospatial/semantic-db.yaml \
  'SELECT * FROM wells LIMIT 10'
cargo run -p semantic-cli -- sql --project-config examples/geospatial/semantic-db.yaml \
  --file examples/geospatial/query.sql
cat examples/geospatial/query.sql | cargo run -p semantic-cli -- sql \
  --project-config examples/geospatial/semantic-db.yaml -
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.yaml
```

`sql` accepts exactly one authored statement as positional SQL, from `--file
PATH`, or from stdin with the positional `-` marker. These inputs are mutually
exclusive. `--` ends option parsing; it does not mean stdin. Reads run through
the read engine. `INSERT`, `UPDATE`, `DELETE`, and `MERGE` require an explicit
write binding and run through the write dispatcher. Unsupported or multiple
statements fail. Ask never executes mutations.

`sql --plan 'SELECT …'` prints a raw logical plan without reading rows.
`sql --explain 'SELECT …'` reports read dependencies and consistency checks;
`sql --explain 'UPDATE …'` reports a write plan without applying changes.
`--plan` accepts reads only and conflicts with read consistency, cache policy,
and reports. `--read-report` reports an executed read on stderr and cannot be
combined with either preview. Read options are rejected for mutations.

`--read-consistency snapshot` requests a single-domain snapshot and bypasses
materializations. `--read-cache bypass` skips configured materializations;
`--read-cache max-age=300` allows generations no older than 300 seconds.
These read policies also apply to executed Ask queries and reads in the REPL.
`--query-timeout-seconds` accepts 1 through 86,400 seconds.

The REPL accepts SQL and dot commands such as `.tables`, `.schema NAME`,
`.ask REQUEST`, and `.plan REQUEST`. End SQL with `;`; use `.quit` to exit.
Run `sdb repl --help` for history, color, and read policy options.

## Natural-language reads

Copy `.env.example` to `.env` in the working directory and set
`OPENAI_API_KEY`. Set `OPENAI_MODEL` and `OPENAI_BASE_URL` for another
OpenAI-compatible provider. Process environment values override `.env`.
SQL-only use does not require a model key.

```sh
cargo run -p semantic-cli -- ask --project-config examples/geospatial/semantic-db.yaml \
  "Return well_id for active wells in 'North Basin' with total_depth_m >= 2500"
cargo run -p semantic-cli -- ask --project-config examples/geospatial/semantic-db.yaml \
  --compile-only "Count wells by basin"
```

`--compile-only` still calls the model and prints proposed SQL and grounding
evidence, but does not execute query rows. General Ask can use authored views
or base relations. It returns grounded SQL, a clarification question, or an
unsupported reason. Generated SQL must be a read over registered relations;
the engine validates it before execution. Evidence interpretation and request
coverage remain model judgments.

## Cache and server

```sh
sdb cache status
sdb cache refresh wells
sdb cache invalidate KEY_FROM_STATUS
sdb server
```

Cache status and invalidation do not construct source providers. Refresh may
connect to sources. `sdb server` discovers the current project's
`semantic-db.yaml`; `--project-config PATH` overrides it. The default
PostgreSQL and HTTP listeners are `127.0.0.1:5544` and `127.0.0.1:5545`.
