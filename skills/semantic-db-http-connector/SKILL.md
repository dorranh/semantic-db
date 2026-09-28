---
name: semantic-db-http-connector
description: "Build a custom Semantic DB source connector for an HTTP API, including bounded reads, applicable writes, and conservative pushdown."
---

# Build an HTTP API connector

Use this for an HTTP API that needs a custom source connector, not for ordinary local files or existing database bindings. Standard binaries cannot load runtime Rust plugins. Start from a source checkout matching the release and inspect `examples/connectors`, `semantic-sources`, and the current connector traits. Use path dependencies from that checkout until compatible crates are published.

Inspect API schema, pagination, authentication, filtering, sorting, quotas, mutation operations, and consistency behavior. Use official API documentation or observed responses. Keep credentials in host-resolved environment references, never source YAML values or diagnostic output. Make the authorized collection or account scope explicit.

Implement a lazy, bounded scan first. Preserve one stable Arrow schema, nullability, pagination termination, and row multiplicity. Project fields through Ossie mappings rather than creating a second semantic catalog. Connection/schema inspection may fetch metadata; row retrieval belongs in query execution. Sanitize remote errors, time out requests, and honor cancellation/budgets using the current runtime contracts.

Register a ConnectorFactory in a custom host's Registry, reuse Project loading and `semantic_cli::run_with_registry`, and preserve the installed version's `SourceConnection::resource` read/write bindings and resource identities. Validate options offline; resolve secrets through the host. A read provider must not claim write capability or a cross-source snapshot. Model guidance is prepared before resource loading; it must not bypass resource identities or transaction bindings.

Assess writes as part of the source contract. If the API offers safe mutations, inspect the current `WriteConnection` and target-key contracts, expose only supported operations, and test partial failure and reconciliation behavior. Require explicit enablement where appropriate. If the API cannot meet those contracts, leave its write binding absent and state that it is read-only; do not equate an HTTP success response with an atomic transaction or read snapshot.

Add pushdown only for operations proven equivalent to local execution, including null, type, ordering, and limit semantics. Local evaluation must handle unsupported operations correctly. Explain which upstream API changes would reduce transfer; do not assume the user can modify the upstream service.

Deliver connector code, a source config example with secret references, a matching Ossie model, and deterministic fixture tests. Test pagination, empty results, nulls, errors, cancellation, pushed/local equivalence, and supported writes as applicable. Validate through a bounded custom CLI query, checking its installed command shape first; identify remaining limitations and the source-build requirement. Keep live mutations out of fixture tests.
