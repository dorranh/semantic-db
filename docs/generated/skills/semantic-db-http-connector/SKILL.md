---
name: semantic-db-http-connector
description: "Build a custom Semantic DB connector for an HTTP API, including pagination, source bindings, and conservative pushdown."
---

# Build an HTTP API connector

Use this for custom developer work, not for ordinary local files or existing PostgreSQL/ClickHouse bindings. Standard binaries cannot load runtime Rust plugins. Start from a source checkout matching the release and inspect `examples/connectors`, `semantic-sources`, and the current connector traits. Use path dependencies from that checkout until compatible crates are published.

Inspect API schema, pagination, authentication, filtering, sorting, quotas, and consistency behavior. Use official API documentation or observed responses. Keep credentials in host-resolved environment references, never source YAML values or diagnostic output.

Implement a lazy, bounded scan first. Preserve one stable Arrow schema, nullability, pagination termination, and row multiplicity. Project fields through Ossie mappings rather than creating a second semantic catalog. Connection/schema inspection may fetch metadata; row retrieval belongs in query execution. Sanitize remote errors, time out requests, and honor cancellation/budgets using the current runtime contracts.

Register a ConnectorFactory in a custom host's Registry, reuse Project loading and the CLI library entry point, and preserve any read/write resource hooks introduced by the installed version. A read provider must not claim write capability or a cross-source snapshot. Model guidance is prepared before resource loading; it must not bypass resource identities or transaction bindings.

Add pushdown only for operations proven equivalent to local execution, including null, type, ordering, and limit semantics. Local evaluation must handle unsupported operations correctly. Explain which upstream API changes would reduce transfer; do not assume the user can modify the upstream service.

Deliver connector code, a source config example with secret references, a matching Ossie model, and deterministic fixture tests. Test pagination, empty results, nulls, errors, cancellation and pushed/local equivalence as applicable. Validate through a custom CLI query; identify remaining limitations and the source-build requirement.
