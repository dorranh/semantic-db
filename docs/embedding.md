# Embedding Semantic DB

Use the `semantic-db` facade as one dependency. Packages are unpublished; for a
local checkout:

```toml
[dependencies]
semantic-db = { path = "../semantic-db/crates/semantic-db", features = ["sources", "github"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Load the same project as the CLI

```rust,ignore
use semantic_db::sources::{Project, Registry};

let project = Project::from_path("examples/geospatial/semantic-db.yaml")?;
let registry = Registry::standard();
let inspection = project.inspect(&registry)?; // Offline; no credentials/providers.
let imported = project.load(&registry, &|name| std::env::var(name).ok()).await?;
let batches = imported.engine.query("SELECT * FROM wells LIMIT 10").await?;
```

The application chooses its secret resolver; the library does not read process
variables or `.env` itself. Use `Project::new` with a parsed `ProjectConfig`,
`OssieDocument`, and explicit base directory for in-memory configuration. Register
custom factories with `Registry::register` or construct an empty `Registry::new`
to allow only selected connectors. See [configuration](connectors.md) and
[connector development](building-connectors.md).

`features = ["sources"]` includes Ossie and CSV loading; adding `github` registers
GitHub in the standard registry. For SQL-only use set `default-features = false`.
The optional `compiler` feature remains independent of configured loading.

If you already have providers, use the direct Ossie bindings below. If you own
another catalog representation with Arrow schemas, use `Engine::from_catalog`
and `RelationBackend`. The [team catalog example](../crates/semantic-db/examples/team_catalog.rs)
demonstrates that lower-level path.

## Catalog definitions

Use `semantic_db::{Catalog, Relation}` and Arrow schema types from
`semantic_db::catalog`. `Relation::base(name, schema, source)` declares a physical
relation; `source` is an opaque identifier interpreted by your backend. The SQL
name can differ from a qualified warehouse table name or internal source ID.
Keep credentials and clients in the backend.

Add descriptions, owner, and grain using `with_description`, `with_owner`, and
`with_grain`. Grain documents what a row represents; it does not enforce a unique
key. `Relation::view(name, schema, sql)` declares a curated query and its expected
output schema. The engine derives direct lineage from SQL, including subqueries
and excluding local CTE names. Imported dependency lists are recomputed.

`Engine::from_catalog` accepts any `IntoIterator<Item = Relation>`: a `Catalog`,
`Vec<Relation>`, or your existing store's projection. Your catalog does not need
to implement a new storage trait or move into this project's storage format.
Load/filter your application catalog before handing its definitions to the
engine. The engine owns a fixed metadata snapshot; rebuild it to refresh schemas
or definitions. Providers can read live data, so this is not a shared source-data
snapshot or cross-source transaction.

## Relation backend

The only required backend method is:

```rust
use std::sync::Arc;
use semantic_db::{Relation, RelationBackend, TableProvider};
use semantic_db::datafusion::error::Result;

struct Backend {
    table: Arc<dyn TableProvider>,
}

impl RelationBackend for Backend {
    async fn resolve(&self, _relation: &Relation) -> Result<Arc<dyn TableProvider>> {
        Ok(self.table.clone())
    }
}
```

This minimal backend serves one provider; the complete example routes on the
base relation's `source`. A production backend can route to different connectors,
own connection pools, or ask an existing DataFusion `SchemaProvider` for a table.
Return a `DataFusionError` for unsupported or inaccessible sources; the engine
adds the relation name and preserves the cause. Backend errors may contain your
diagnostics, so sanitize them in your adapter if they can contain credentials.

Construction resolves every base relation once, sequentially. SQL views are
planned by the engine and never passed to the backend. Providers retain any
clients they need through owned values or `Arc`s; the engine does not retain a
borrow of the backend. Follow DataFusion's lazy scan contract: resolving a
provider should obtain metadata and a scan implementation, with row reads left
to query execution. Pushdown and remote execution capabilities depend on that
provider; this API adds no automatic federation optimizer.

When you already have a provider, use
`engine.register_table(relation, provider)` to install a base relation directly.
`register_csv` and `create_view` remain convenient schema-inference APIs for
interactive use. The latter derives an output schema when you do not need a
declared view contract.

## Ossie models

Enable the facade's `ossie` feature to use `semantic_db::ossie`. This is independent
of `compiler`: SQL-only applications can set `default-features = false` and
`features = ["ossie"]`. Python and schema downloads are not required at runtime.

```rust,ignore
use semantic_db::ossie::{OssieDocument, SourceBindings};

let document = OssieDocument::parse(&yaml)?;
let mut bindings = SourceBindings::new();
bindings.bind("fixtures.geospatial.wells", provider)?; // Arc<dyn TableProvider>
let imported = document.load(Some("geospatial_wells"), &bindings)?;
for warning in imported.warnings {
    eprintln!("{warning}");
}
let engine = imported.engine;
```

`bind_csv(source, path).await?` is a convenience for local CSV fixtures. Custom
connectors bind their own providers with `bind`. Bindings are reused during
projection and registration; the adapter derives Arrow schemas from the bound
providers rather than guessing widths or nullability from logical Ossie types.
Each dataset exposes only its explicitly declared fields in document order, including
checked single-column aliases. `document.inspect(model)` returns source and field
requirements without bindings or I/O.
Source strings are opaque binding keys and are never automatically run as SQL or
interpreted as URLs. Duplicate bindings fail.

`load(None, ...)` selects the only model; otherwise provide an exact name.
It returns a fresh engine or document-path diagnostics, never a partial engine.
`OssieDocument::parse` checks the pinned JSON Schema, while `load` checks executable
capabilities and source contracts. `original_text()` and `json()` retain the
source document, including features that prevent executable import. See the
[supported profile](ossie-reference.md).

Metadata lives in `Relation.semantics`: model/field descriptions, AI context,
logical field types, time roles, labels, declared keys, and import provenance.
Keys are explicitly unenforced declarations. The compiler receives these
annotations as evidence, not as instructions that override its rules. Neither
units in descriptions nor valid field types prove deterministic domain correctness.

Run the complete [wells importer example](../crates/semantic-db/examples/ossie_wells.rs):

```sh
cargo run -p semantic-db --features ossie --example ossie_wells
```

## Validation and failure behavior

Names must be lowercase, unqualified SQL identifiers. Duplicate names, missing
view dependencies, cycles, and invalid view statement forms are rejected before
any backend is called. Definitions can arrive in any order. Views may reference
only catalog relations using unqualified names.

Provider and view output schemas must exactly equal their declared Arrow schema:
column names, ordering, types, nullability, and Arrow metadata. Mismatches identify
the relation and report expected/actual schemas. Relation-level description,
owner, and grain are separate from the Arrow schema and preserved as authored.
No partial engine is returned on failure, but already completed backend I/O
cannot be undone. Single-table registration validates before modifying either
the descriptive catalog or DataFusion's catalog. Duplicate registration never
replaces an existing provider.

The library exposes DataFusion `DataFrame`, `TableProvider`, and Arrow types.
Use its re-exports or a compatible DataFusion version in your connector; upgrades
to these dependencies can affect the public API. Packages remain unpublished.
Semantic DB is licensed under the [Apache License 2.0](../LICENSE).

## Queries and natural language

`engine.query(sql).await?` collects Arrow batches. For large results, obtain a
data frame with `engine.plan_sql(sql).await?` and consume
`frame.execute_stream().await?`. The caller owns streaming, cancellation, result
limits, and application-level access policy.

With the default `compiler` feature, configure a provider explicitly:

```rust,ignore
use semantic_db::{Compiler, GroundingOutcome};
use semantic_db::compiler::provider::{OpenAiConfig, OpenAiProvider};

let provider = OpenAiProvider::new(OpenAiConfig::new(api_key, model))?;
let compilation = Compiler::new(provider).compile(&engine, request).await?;
match compilation.outcome {
    GroundingOutcome::Grounded { query } => {
        let frame = engine.plan_generated_sql(&query.sql).await?;
        let stream = frame.execute_stream().await?;
        // Consume the stream in your application.
    }
    GroundingOutcome::NeedsClarification { question, .. } => { /* ask the user */ }
    GroundingOutcome::Unsupported { reason } => { /* explain the limitation */ }
}
```

The library does not load `.env` or read provider configuration from the process
environment. Supply a custom `ModelProvider` to use another model adapter. For
SQL-only use, disable the facade's default features.

The compiler receives descriptions, grain, column names/types/nullability, and
view definitions for the full loaded catalog. Imported `Relation.semantics` also
provides authored model/field annotations, declared keys, and provenance. It omits owners, base source
identifiers, arbitrary Arrow metadata, and row samples. Filter the catalog to
the caller's permitted scope before loading it. SQL validation and evidence
existence checks are implemented; deterministic enforcement of business
definitions, join cardinalities, and units is still future work.
