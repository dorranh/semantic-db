# Embedding Semantic DB

A team owns its catalog definitions, source connections, and model configuration.
Semantic DB turns a snapshot of those definitions into a queryable session. Start
with the [complete example](../crates/semantic-db/examples/team_catalog.rs):

```sh
cargo run -p semantic-db --example team_catalog
```

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
to these dependencies can affect the public API. Packages remain unpublished
and no project license has been selected yet.

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
view definitions for the full loaded catalog. It omits owners, base source
identifiers, arbitrary Arrow metadata, and row samples. Filter the catalog to
the caller's permitted scope before loading it. SQL validation and evidence
existence checks are implemented; deterministic enforcement of business
definitions, join cardinalities, and units is still future work.
