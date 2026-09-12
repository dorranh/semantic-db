# CLI reference

Use the [dataset guide](adding-datasets.md) for the configured onboarding path.
All commands below run from the repository root unless a different directory is
specified. Install locally with `cargo install --path apps/semantic-cli --locked`
to use `semantic-db` directly instead of `cargo run -p semantic-cli --`.
Packages and prebuilt binaries are not published yet.

## Project configuration and checks

```sh
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml --inspect
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml --validate
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml --validate --connect
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml --query 'SELECT * FROM wells' --dry-run
```

`--config` selects a YAML/JSON project; see the [configuration reference](connectors.md).
Model and CSV paths inside it resolve relative to the project file. `--file`
paths and `.env` resolve relative to the working directory. Configured loading
cannot be mixed with `--csv`, `--ossie`, `--ossie-model`, or `--source-csv`.
It supports the same SQL, file, ask, view, piped-stdin, and interactive modes.

`--inspect` lists fields, physical mappings, source IDs and connector names.
`--validate` checks the model's executable profile, required bindings and all
configured connector options offline. Neither reads credentials or constructs
providers. They reject query/view options so a check cannot accidentally execute
rows. `--validate --connect` additionally constructs required providers and checks
physical schemas. CSV inference reads a sample; other connectors may request
metadata. GitHub uses fixed schemas, so this does not prove token permissions or
row access. No query rows are executed by the check.

`--ossie PATH --inspect` and `--ossie PATH --validate` work without project config,
but check only the model and report required providers; they do not verify bindings.
Add `--validate --connect --source-csv SOURCE=PATH` for a connected direct CSV check.

`--dry-run` requires `--query`, `--file`, or `--ask`. SQL mode prints a logical
plan without executing it. Natural-language mode still calls the model. Neither
estimates remote cost or proves permissions. Schema inference may happen while
loading sources, before planning.

## Direct source flags

The original `--csv`, `--ossie` and `--source-csv` flags remain available for
one-off use. Prefer project files when sharing or repeating dataset setup.

### Direct CSV examples

Install [rustup](https://rustup.rs/). The repository pins Rust 1.94.0, matching
DataFusion 55's compiler requirement. The first build downloads and compiles a
substantial dependency graph.

From the repository root:

```sh
# Interactive SQL
cargo run -p semantic-cli -- --csv wells=examples/geospatial/wells.csv

# Run the example query: returns W-001 and W-004
cargo run -p semantic-cli -- --csv wells=examples/geospatial/wells.csv \
  --file examples/geospatial/query.sql

# Define a view and query it in one session
cargo run -p semantic-cli -- --csv wells=examples/geospatial/wells.csv \
  --view 'deep_wells=SELECT * FROM wells WHERE total_depth_m >= 2500' \
  --query 'SELECT well_id FROM deep_wells ORDER BY well_id'
```

In the interactive session:

```text
.tables
.schema wells
.view deep_wells=SELECT * FROM wells WHERE total_depth_m >= 2500
SELECT well_id, basin FROM deep_wells ORDER BY well_id;
EXPLAIN SELECT * FROM deep_wells WHERE basin = 'North Basin';
.quit
```

SQL can span lines; submit one statement at a time, ending the last line with `;`.
Dot commands occupy one line. Ctrl-C clears pending SQL; Ctrl-D exits. History is
session-local. `--query`, `--file`, and piped stdin accept one SQL statement;
multi-statement scripts are not supported. Use `--view` or `.view` to create views;
SQL DDL and DML are disabled to keep the metadata catalog in sync.

CSV files require headers; schema inference uses DataFusion's defaults. Relations
and views last only for the current process. Names currently use lowercase,
unqualified SQL identifiers. CLI results are collected in memory, so use `LIMIT`
for exploratory queries over large inputs. Library consumers can stream results.


## Natural-language queries

Copy `.env.example` to `.env` in the repository root and fill in `OPENAI_API_KEY`.
The default model is `gpt-4.1-mini`; set `OPENAI_MODEL` and `OPENAI_BASE_URL` for
another OpenAI-compatible provider. The base URL is the API root, including its
version prefix (for example, `https://api.openai.com/v1`). The adapter appends
`/chat/completions`. `.env` is ignored by Git. Process environment variables take
precedence over `.env`; SQL-only sessions do not require provider configuration.

```sh
cargo run -p semantic-cli -- --csv wells=examples/geospatial/wells.csv \
  --ask "Return well_id for active wells in 'North Basin' with total_depth_m >= 2500, ordered by well_id"

# Inspect SQL and model-proposed evidence without executing rows
cargo run -p semantic-cli -- --csv wells=examples/geospatial/wells.csv \
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

