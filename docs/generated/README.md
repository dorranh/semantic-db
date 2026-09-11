# Architecture notes

These documents turn the initial design discussion into a domain-neutral starting
point. They are maintained Markdown, not output from a code generation command.

1. [System architecture](architecture.md): boundaries, crate ownership, and query flow.
2. [Catalog and derived relations](catalog-and-derived-relations.md): metadata, views, row types, and lineage.
3. [Grounding and federation](grounding-and-federation.md): semantic compilation, validation, and backend execution.
4. [Implementation roadmap](roadmap.md): what runs today, what remains, and acceptance criteria.

The examples use synthetic wells, subsurface intervals, and survey documents.
These are example datasets, not assumptions embedded in the core abstractions.
