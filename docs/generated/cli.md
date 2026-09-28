# CLI reference

Use the [dataset guide](adding-datasets.md) to create a project configuration.
The `sdb` executable has explicit `repl`, `server`, and `init` commands. The
examples below run from the repository root with `cargo run -p semantic-cli --`;
replace that prefix with `sdb` after installation.

## Project configuration and checks

```sh
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.yaml --inspect
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.yaml --validate
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.yaml --validate --connect
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.yaml --query 'SELECT * FROM wells' --dry-run
```

`--project-config PATH` loads a YAML or JSON Semantic DB project, including its
Ossie model, source bindings, views, and cache settings. Model, CSV, and view
paths in the project resolve relative to the project file. `--file` and `.env`
resolve relative to the working directory. For a project with multiple models,
select one in the project configuration. See the [configuration reference](connectors.md).
When `--project-config` is omitted, `sdb repl` loads `semantic-db.yaml` from the
current directory if present and reports the path on stderr. Use `--no-project`
to open an empty session; it conflicts with `--project-config`.

`--inspect` lists fields, physical mappings, source IDs, connector names, and
configured views. `--validate` checks the model, bindings, connector options,
view syntax, and view dependencies offline. Neither reads credentials or
constructs providers. `--validate --connect` also constructs providers and checks
physical schemas. CSV inference reads a sample; other connectors may request
metadata. No query rows are executed by validation.

`--dry-run` requires a query, file, natural-language request, or write. SQL mode
prints a logical plan without executing query rows. Natural-language mode still
calls the model. Loading sources may infer schemas before planning.

## SQL and interactive use

```sh
# Interactive SQL
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.yaml

# Execute one statement from a file
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.yaml \
  --file examples/geospatial/query.sql

# Query a view authored in the project configuration
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.views.yaml \
  --query 'SELECT well_id FROM deep_wells ORDER BY well_id'
```

In the interactive session:

```text
.tables
.schema wells
SELECT well_id, basin FROM wells ORDER BY well_id;
EXPLAIN SELECT * FROM wells WHERE basin = 'North Basin';
.quit
```

SQL can span lines; submit one statement at a time, ending the last line with `;`.
Dot commands occupy one line. Ctrl-C clears pending SQL; Ctrl-D exits. Author
views as SQL files referenced by the project configuration. `--query`, `--file`,
and piped stdin accept one SQL statement; multi-statement scripts are not supported.
CLI results are collected in memory, so use `LIMIT` for exploratory queries.
Library consumers can stream results.

Read options apply to direct SQL, file input, piped SQL, interactive SQL, and
executed Ask queries. `--read-consistency snapshot` requests a single-domain
snapshot and bypasses materializations. `--read-cache bypass` skips configured
materializations; `--read-cache max-age=300` accepts generations no older than
300 seconds. `--explain-read` reports dependencies and consistency checks without
query rows; `--read-report` prints execution evidence to stderr. These read
controls cannot accompany validation, cache maintenance, writes, or `--dry-run`.
`--query-timeout-seconds` applies to execution and accepts 1 through 86,400.

## Natural-language queries

Copy `.env.example` to `.env` in the repository root and fill in `OPENAI_API_KEY`.
The default model is `gpt-4.1-mini`; set `OPENAI_MODEL` and `OPENAI_BASE_URL` for
another OpenAI-compatible provider. The base URL is the API root, including its
version prefix (for example, `https://api.openai.com/v1`). The adapter appends
`/chat/completions`. `.env` is ignored by Git. Process environment variables take
precedence over `.env`; SQL-only sessions do not require provider configuration.

```sh
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.yaml \
  --ask "Return well_id for active wells in 'North Basin' with total_depth_m >= 2500, ordered by well_id"

# Inspect SQL and model-proposed evidence without executing rows
cargo run -p semantic-cli -- repl --project-config examples/geospatial/semantic-db.yaml \
  --ask "Count wells by basin" --dry-run
```

Interactive sessions also support `.ask REQUEST` and `.plan REQUEST` (compile
without execution). Each request is independent: when asked for clarification,
resubmit the complete request with the missing definition. Provider configuration
is loaded from the current directory's `.env` on first use and reused in the REPL.

The compiler sends relation names, column types/nullability, descriptions, grain,
and view definitions to the provider. Imported model/field descriptions, AI
context, declared keys, and provenance are also included. It does not sample rows or include base
source paths. It returns SQL with evidence, a clarification question, or an
unsupported reason. Clarification/unsupported outcomes do not execute a query;
they are successful compilation outcomes and exit with status 0. Configuration,
provider, validation, and execution errors exit nonzero in batch mode.

Generated SQL must be a query over registered, unqualified catalog relations.
DataFusion validates syntax, names, types, and functions before execution. Invalid
output gets at most one repair call by default; transport failures, refusals,
truncated output, and HTTP errors fail immediately. Requests time out after 60
seconds (`OPENAI_TIMEOUT_SECONDS`); response bodies are capped at 1 MiB. There is
no explicit token budget yet. Set `OPENAI_JSON_MODE=false` for servers that do not
support `response_format`; local JSON/schema validation still applies.

This is an initial LLM-to-grounded-SQL slice. Evidence references are checked for
existence; their interpretation and coverage are model proposals. SQL validity
does not prove domain correctness. The prompt asks for clarification when units,
thresholds, or meanings are missing, but deterministic semantic enforcement,
retrieval over large catalogs, and richer typed intent lowering remain future
work. Use a view with an explicit definition or put definitions in the request.
