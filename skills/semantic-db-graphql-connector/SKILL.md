---
name: semantic-db-graphql-connector
description: "Build a custom Semantic DB source connector for a GraphQL API, including relational pagination, applicable mutations, and conservative pushdown."
---

# Build a GraphQL connector

GitHub is a worked GraphQL example, not a core Semantic DB data source. Standard binaries exclude it; a source checkout can enable the `github` feature for experiments. Custom connectors require a matching source build. Inspect `examples/github`, `crates/semantic-github`, and current `semantic-sources` interfaces for reusable patterns, not a fixed schema to copy.

Read the actual GraphQL schema and service documentation. Identify stable entity IDs, connection/edge pagination, nullable fields, rate limits, API errors, mutation capabilities, and authorization scope. Model nested collections as separate relations with explicit keys when appropriate; preserve multiplicity and avoid accidental Cartesian expansion.

Keep credentials host-resolved. Make configured scope explicit (for example an allowed collection or repository set); query text must not silently expand it. Schema discovery and project validation must not fetch arbitrary data rows. Fetch rows lazily with bounded pages, deadlines, budget accounting and cancellation.

Inspect the current ConnectorFactory/SourceConnection resource hooks. Validate configuration offline, resolve credentials through the host, and preserve resource identities and any read/write bindings when integrating model-guided source preparation; read-only APIs do not acquire write or snapshot guarantees by becoming tables.

Assess GraphQL mutations alongside reads. If the service can safely implement supported writes, use the current `WriteConnection` and target-key contracts, make the capability opt-in where appropriate, and test failures after partial upstream effects. If mutations cannot satisfy the requested write contract, expose a read-only binding and explain the limit. GraphQL mutation success does not imply a cross-request transaction or shared read snapshot.

Test a correct paginated scan before optimization. Push filters, projections and limits only when GraphQL service behavior matches local query semantics. Treat partial data plus GraphQL errors according to an explicit fail/partial-result contract; default to failing a query that would otherwise appear complete. Compare optimized results with a scan baseline and count upstream requests.

Finish with a registered connector, config/secret example, Ossie mappings, runnable custom-host command, and deterministic mock-server tests. Cover pagination termination, nulls, authorization scope, errors, relevant pushdown cases, and supported mutations. Verify a bounded query with the installed CLI command shape. Keep live mutations separate from fixture tests. Explain upstream schema/API changes that could improve efficiency without assuming permission to make them.
