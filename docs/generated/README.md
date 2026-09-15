# Architecture notes

These documents turn the initial design discussion into a domain-neutral starting
point. They are maintained Markdown, not output from a code generation command.

Start with [Add a dataset](../adding-datasets.md) or [Build a connector](../building-connectors.md).
The [embedding guide](../embedding.md) covers library integration; the
[catalog comparison](../catalog-prior-art.md) records standards and prior art.

## Current guides

- [Types and extensibility](supported-types.md): physical/logical types and the UDF extension boundary.
- [Database comparison](database-comparison.md): Semantic DB, semantic layers, and other query engines.
- [Performance and federation](performance.md): pushdown, joins, shared budgets, and materialization.
- [Documentation site](docs-site.md): run `just docs`, maintain content, and configure GitHub Pages.

## Design and implementation notes

These notes include historical design stages; use the current guides above for
the supported type and execution contracts.

1. [System architecture](architecture.md): boundaries, crate ownership, and query flow.
2. [Catalog and derived relations](catalog-and-derived-relations.md): metadata, views, row types, and lineage.
3. [Grounding and federation](grounding-and-federation.md): semantic compilation, validation, and backend execution.
4. [Implementation roadmap](roadmap.md): what runs today, what remains, and acceptance criteria.
5. [Grounding with authored views](authored-view-grounding.md): checked view binding, typed query lowering, and the remaining interpretation boundary.
6. [Reads, writes, and reconciliation strategy](writes-and-reconciliation-strategy.md): proposed read/commit boundaries, snapshot and visibility guarantees, and eventual convergence.
7. [Reads, writes, and reconciliation technical design](writes-and-reconciliation-design.md): proposed read/write APIs, snapshot sessions, checked merges, connector contracts, and implementation stages.

The examples use synthetic wells, subsurface intervals, and survey documents.
These are example datasets, not assumptions embedded in the core abstractions.
