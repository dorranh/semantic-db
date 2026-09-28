---
name: semantic-db-custom-integration
description: Build a custom Semantic DB source connector for a database, API, or other backend through the Rust registry, including supported reads and writes. Use when an available connector cannot bind the source, not for application clients.
---

# Build a custom source connector

First check whether a built-in connector or a compatible DataFusion provider already serves the source. Custom connectors are compiled Rust dependencies; the standard binary does not load runtime plugins. Work from a source checkout matching the target Semantic DB release and inspect its current `semantic-sources` traits and `examples/connectors` template. Reuse an existing provider when it satisfies the source contract; implement a lazy stream only when needed.

Establish the source's schema, row grain, stable identifiers, authorization scope, pagination or cursor rules, limits, errors, and available read and write guarantees. A custom connector should assess both reads and writes. Implement writes only where the upstream API can safely support the requested operations; an upstream read-only source remains a valid read-only connector with that limit stated clearly. Do not infer snapshot isolation, atomicity, or cross-source transactions from a successful read or write.

Register a `ConnectorFactory` in a custom host's `Registry` and reuse `semantic_cli::run_with_registry` or `Project::load`. Validate connection and source options offline; resolve secret references through the host-supplied resolver during connection. Keep query text from expanding configured authorization scope. Use `prepare_source` only for offline model-guided normalization. Preserve connector-issued resource identities and the current `SourceConnection::resource` read and write bindings.

Make a correct scan before adding pushdown. Return a stable Arrow schema, fetch rows only during query execution, bound remote work, and honor deadlines and cancellation. A later page error must fail the query rather than make partial data look complete. Push filters, projections, ordering, joins, aggregates, or limits only where upstream and local SQL semantics agree; otherwise leave residual evaluation local. Compare optimized results with an unoptimized baseline and measure upstream work. Inspect current transaction and reconciliation contracts before exposing `ReadConnection`, `WriteConnection`, snapshots, or a writable `SourceResource`. Require explicit write enablement where appropriate, reversible mappings and usable target keys, and tests for failure outcomes.

For HTTP APIs, use `semantic-db-http-connector` if installed; for GraphQL, use `semantic-db-graphql-connector` if installed. This skill remains sufficient on its own: apply the same source contract, lazy scan, write-capability assessment, and equivalence rules without those specialists.

Deliver a registered connector and custom-host command, config with secret references, Ossie mappings, bounded SQL with expected results, and documented capabilities. Use deterministic fixtures for schema, pagination, errors, cancellation, read equivalence, and any write or transaction behavior. Keep live smoke tests separate and bounded; do not mutate a live upstream merely to test the connector without authorization.
