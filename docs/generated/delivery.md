# Delivery and integration contract

Semantic DB ships a CLI/REPL (`semantic-db`), a PostgreSQL/HTTP server
(`semantic-server`), agent skills, a runnable starter, and a Rust extension path.
The first supported deployment is local or explicitly trusted. CLI and server
use the same project, semantic model, and authored views.

## Release contents

Each platform archive contains matching CLI/server binaries, generated guides,
and five self-contained skill directories. A SHA-256 sidecar verifies the archive.
The release workflow builds Linux x86-64 (GNU/glibc), macOS Apple Silicon and Intel,
and Windows x86-64. These are native binaries; Linux is not advertised as a fully
static musl build. Platform availability is established by successful release CI.

Tag releases with the workspace package version (`v0.1.0`, for example). The
packager rejects CLI/server version mismatches. Tagged builds create a draft
GitHub release; workflow dispatch produces downloadable CI artifacts only.
No release is published by developing this workflow. Crates.io distribution is
separate; until then, use a source checkout matching the binary release for Rust
embedding and custom connectors.

## Built-in sources

Both binaries include PostgreSQL, ClickHouse, CSV, NDJSON, Parquet, Avro, and
Arrow IPC files. The `file` connector automatically selects the format from the
path. See [file configuration and capabilities](file-connectors.md).

GitHub demonstrates GraphQL connector development. It is excluded from default
binary builds, remains an optional `github` feature, and can be exercised with
`just repl github` or a source build using `--features github`. A whole-workspace
Cargo build may unify example features; release builds select only the two apps.
Custom Rust connectors require a matching source build; binaries do not load
runtime plugins.

## Ask and applications

The primary walkthrough uses authored-view Ask. Define business meaning in
Ossie and SQL views, ask a question, inspect the SQL and binding evidence, and
execute it. The general `.ask` mode remains available for SQL exploration but
has different grounding guarantees from `.ask-views`.

The server exposes `/health`, `/catalog`, and `/compile` over HTTP. `/compile`
uses the general compiler over the full catalog, preferring authored views when
their definitions fit and composing queries over views or base relations as
needed. It returns SQL/evidence, clarification, or unsupported; SQL execution uses the
PostgreSQL frontend. The same model configuration works across both apps when
`OPENAI_API_KEY`, `OPENAI_MODEL`, and any provider URL are explicitly set.
SQL-only operation requires no model credentials.

PostgreSQL wire support is not a promise of PostgreSQL dialect, ORM, transaction,
or dashboard compatibility. The existing package-maintenance application is the
full source example of `pg` integration and optional Ask. Keep its application
write boundary separate from query compilation.

## Transactional integration

`ConnectorFactory::prepare_source` normalizes model-guided options offline before
`SourceConnection::resource` loads a source. The loader retains the connector's
resource identity and read/write bindings, validates reversible model mappings
and exposed target keys, and preserves existing authorization and cache scopes.
Ossie is optional for application-only projects. Application tables without a
model use the configured options directly; file readers infer their schema.

File factories supply a storage namespace but do not certify a transaction
domain, resource identity, or snapshot. Files and ClickHouse remain read-only.
PostgreSQL supports writes through the transaction interfaces documented in
[writes and reconciliation](writes-and-reconciliation-implementation.md).
These interfaces define the supported guarantees; PostgreSQL wire compatibility
alone does not imply them.

## Verification

`cargo test -p semantic-sources -p semantic-cli -p semantic-server --test files --locked`
checks all five formats, gzip CSV/NDJSON, model-guided parsing, directories,
cross-format joins, and both executable frontends. Existing database connector
tests remain the authority for their pushdown and type contracts. The full
workspace suite also checks transaction, checked-input, and write behavior.

Remote file storage, JSON CLI output, runtime plugins, and production service
hardening remain separate milestones. Hand-curated top-level docs are unchanged.
