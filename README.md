# Semantic DB

A Rust library for teams to query their own catalog through their own data
backends, using Apache DataFusion and Arrow. The CLI is a runnable example of
embedding the engine.

The intended pipeline is **intent → catalog retrieval → grounding → relational
planning → federated execution**. The current slice runs SQL and catalog-aware
natural-language queries over application-provided DataFusion tables, CSV data,
and composable views. Built-in remote connectors and deterministic domain
grounding remain future work.

## Embed in your application

Use the `semantic-db` facade as one dependency. Packages are not published yet;
for a local checkout:

```toml
[dependencies]
semantic-db = { path = "../semantic-db/crates/semantic-db" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Define `Relation::base` and `Relation::view` entries with Arrow schemas and
descriptions, then implement one async `RelationBackend::resolve` method returning
`Arc<dyn TableProvider>`. The backend owns source routing and connection setup;
the engine plans SQL, composes views, and validates schema contracts.

```rust,ignore
let catalog = Catalog::from_relations([
    Relation::base("orders", schema.clone(), "warehouse:orders")
        .with_description("Customer orders")
        .with_grain("One row per order"),
    Relation::view("completed_orders", schema,
        "SELECT * FROM orders WHERE status = 'completed'"),
])?;
let engine = Engine::from_catalog(catalog, &backend).await?;
let batches = engine.query("SELECT * FROM completed_orders LIMIT 10").await?;
```

`from_catalog` also accepts an iterator of relations projected from your existing
catalog. Providers use DataFusion's standard interface; you can reuse connectors
or register a provider directly with `Engine::register_table`. DataFusion and
Arrow are re-exported by the facade so their types use matching versions.

Run the complete example, which defines a team catalog and custom backend and
returns order IDs 1 and 3:

```sh
cargo run -p semantic-db --example team_catalog
```

See the [embedding guide](docs/embedding.md) for the backend contract, model
configuration, and streaming. For SQL-only applications, use
`default-features = false` to omit the compiler and its HTTP provider dependency.
The [catalog comparison](docs/catalog-prior-art.md) recommends DataFusion for
execution and evaluates Apache Ossie/OSI for semantic interchange.
The [Ossie integration guide](docs/ossie-integration.md) describes the supported
import profile and its pinned upstream schema.

## Get started

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

## Load an Ossie model

Load the [wells model](examples/geospatial/README.md) and explicitly bind its
source reference to the CSV:

```sh
cargo run -p semantic-cli -- \
  --ossie examples/geospatial/wells.ossie.yaml \
  --source-csv fixtures.geospatial.wells=examples/geospatial/wells.csv \
  --file examples/geospatial/query.sql
```

This imports the model and returns **W-001 and W-004**. Use `--ossie-model NAME`
for documents containing multiple models. The same options work with `--query`,
`--ask`, `--dry-run`, `--view`, and the REPL; `--csv` is the separate direct-load
mode. For natural language, imported descriptions and AI context reach the compiler.

The importer validates the bundled `0.2.0.dev0` schema offline, checks source
bindings/types, and exposes only explicitly declared identity fields. Keys remain
declarations and produce an unenforced-key warning. Metrics, relationships,
computed fields, and custom extensions fail with document-path diagnostics.

Library consumers enable the optional `ossie` feature:

```sh
cargo run -p semantic-db --features ossie --example ossie_wells
```

See the [embedding guide](docs/embedding.md#ossie-models) for provider bindings.

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

## Workspace

| Package | Responsibility |
| --- | --- |
| `crates/semantic-db` | Single dependency for embedding; re-exports catalog, engine, optional compiler, DataFusion, and Arrow |
| `crates/semantic-catalog` | Relation schemas, definitions, lineage, and concept metadata |
| `crates/semantic-ossie` | Pinned schema validation, source bindings, field projections, and semantic metadata import |
| `crates/semantic-plan` | Serializable semantic intent and grounding result contracts |
| `crates/semantic-engine` | Catalog loading, relation backends, DataFusion sessions, views, and SQL execution |
| `crates/semantic-compiler` | Provider adapter, catalog prompt, grounding outcomes, validation, bounded repair |
| `apps/semantic-cli` | Interactive and batch SQL/natural-language frontend; binary name `semantic-db` |

The compiler uses semantic-plan's grounding outcome contracts. Its unresolved
`SemanticPlan` remains a scaffold for richer typed intent lowering. The geospatial
fixture contains entirely synthetic wells and demonstrates ordinary relational
queries, not spatial kernels.

Architecture and next steps are in [docs/generated](docs/generated/README.md).

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

These checks use fake providers/local HTTP servers and need no API key. Once
`.env` is configured, run the optional live semantic evaluation (3–6 API calls;
this incurs provider usage):

```sh
cargo test -p semantic-cli --test llm --locked -- --ignored --nocapture
```

The live evaluation checks exact results (W-001 and W-004), clarification for an
undefined depth threshold, and unsupported location-quality filtering. It is
excluded from normal tests and CI because model behavior is nondeterministic.

Commit `Cargo.lock` for reproducible application builds. Workspace members are
unpublished by default. A project license has not yet been selected.
