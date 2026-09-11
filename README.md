# Semantic DB

An exploration of a domain-agnostic semantic query engine over heterogeneous data
sources, using Rust, Apache DataFusion, and Arrow.

The intended pipeline is **intent → catalog retrieval → grounding → relational
planning → federated execution**. The current foundation runs ordinary SQL over
CSV data and composable views. Natural-language compilation and remote federation
are future work.

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

## Workspace

| Package | Responsibility |
| --- | --- |
| `crates/semantic-catalog` | Relation schemas, definitions, lineage, and concept metadata |
| `crates/semantic-plan` | Serializable semantic intent and grounding result contracts |
| `crates/semantic-engine` | DataFusion sessions, CSV registration, views, and SQL execution |
| `apps/semantic-cli` | Interactive and batch SQL frontend; binary name `semantic-db` |

The semantic-plan crate is an independent contract scaffold; it is not wired to
an LLM or automatically converted to SQL. The geospatial fixture contains entirely
synthetic wells and demonstrates ordinary relational queries, not spatial kernels.

Architecture and next steps are in [docs/generated](docs/generated/README.md).

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Commit `Cargo.lock` for reproducible application builds. Workspace members are
unpublished by default. A project license has not yet been selected.
