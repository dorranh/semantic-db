# Where Semantic DB fits

Semantic DB is an experimental, embedded Rust query engine combining an Ossie
model, DataFusion execution, and federated providers. Its useful distinction is
bringing authored meaning and executable relations into an application process,
including relations backed by APIs. It is not currently a durable transactional
database, distributed warehouse, or complete governed metrics service.

This comparison was checked against the linked first-party documentation on
2026-09-13. It compares architectural choices, not measured speed or exhaustive
feature parity. Projects and product editions change; evaluate a concrete query
and deployment before choosing.

## Architecture comparison

| System | Where meaning lives | Execution and data access | Main tradeoff relative to Semantic DB |
| --- | --- | --- | --- |
| **Semantic DB** | Ossie descriptions, logical types, keys, and authored SQL views; bounded or SQL-generating natural-language compilation | In-process DataFusion; CSV and compiled providers, including GitHub and experimental ClickHouse federation; optional local materializations | Small Rust integration surface, but experimental APIs and a limited connector set. Ossie metrics and relationships are not executable metric/join planning features today. |
| **Wren AI Core** | MDL models, relationships, calculated fields, views, and cubes | Rust/DataFusion semantic engine; query planning for target databases and a separate browser execution path | A particularly close semantic-engine comparison. MDL is its modeling contract; assess its modeling and deployment interfaces against an Ossie/provider-based Rust integration. [Core overview](https://docs.getwren.ai/oss/introduction), [MDL](https://docs.getwren.ai/oss/engine/concept/what_is_mdl), [browser SDK](https://docs.getwren.ai/oss/sdk/wasm) |
| **Cube** | Central definitions of metrics, dimensions, joins, and access rules | Semantic layer served over SQL and application APIs, with caching/pre-aggregations | A stronger fit to evaluate for shared BI/application metrics and governed access; a service architecture introduces a different operational boundary from an embedded engine. [Semantic layer](https://cube.dev/articles/what-is-a-semantic-layer), [query pushdown](https://cube.dev/blog/query-push-down-in-cubes-semantic-layer) |
| **Snowflake semantic views** | Schema-level semantic objects defining business entities, relationships, and metrics | Native Snowflake objects queried with SQL and used by Cortex Agents | An example of a database incorporating a semantic layer directly. Evaluate when data and operations already center on Snowflake. [Semantic views overview](https://docs.snowflake.com/en/user-guide/views-semantic/overview) |
| **DuckDB** | SQL schemas/views; the cited attachment mechanism does not supply an Ossie business model | Embedded analytics, with attachments including PostgreSQL, MySQL, and SQLite | Evaluate for embedded analytical execution and existing source extensions when you can manage business semantics elsewhere. [Multi-database support](https://duckdb.org/2024/01/26/multi-database-support-in-duckdb) |
| **Trino** | SQL catalogs and connector metadata; a business modeling layer is a separate design concern | Federated SQL across connector catalogs, with connector-specific pushdown | Evaluate for a federation service with a broad connector ecosystem. Pushdown is still operation/source dependent, not automatic dialect equivalence. [Connectors](https://trino.io/docs/current/connector.html), [pushdown](https://trino.io/docs/current/optimizer/pushdown.html) |

Wren and Snowflake are especially relevant to the newer semantic-engine and
database-native semantic-layer direction. Sharing DataFusion with Wren does not
establish equivalent supported models or source behavior. Likewise, importing
Ossie is not equivalent to implementing every construct in that specification.
The [Ossie reference](../ossie-reference.md) is the contract for this repository.

## Choosing what to prototype

- **An embedded Rust application joining an API to relational data:** try Semantic
  DB with a narrow, representative dataset and measure source requests/bytes.
- **Shared metrics across BI tools and applications:** compare Cube and Wren's
  modeling contracts, governance, and integration interfaces before building
  those services around Semantic DB.
- **Business definitions over a Snowflake-centered estate:** evaluate native
  semantic views before introducing an additional query layer.
- **Embedded SQL analytics with established storage/source extensions:** compare
  DuckDB; determine whether you actually need the semantic compilation layer.
- **A centrally operated federated SQL service:** evaluate Trino and the exact
  connector capabilities required by your queries.

These are engineering judgments based on the architectures above, not product
rankings. Natural-language availability, semantic modeling, and correctness are
separate questions: a planned query can still embody the wrong business meaning.

## Boundaries to test

For Semantic DB, test the [supported type contract](supported-types.md), source
scope and credentials, join multiplicity, NULLs, empty aggregates, and the
[federation performance path](performance.md). There is no cross-source snapshot
or distributed transaction. Keys are descriptive, and importing a relationship
does not synthesize a join. The engine blocks SQL DDL/DML through its query API.

Use the same fixture and result semantics when comparing engines. Record local
versus remote execution, transferred bytes, request count, cache freshness,
concurrency, and peak memory alongside latency. A remote aggregation and a local
full scan are different workloads even when they return identical rows. Existing
connector benchmark artifacts are diagnostic evidence for those runs, not a
cross-database performance ranking.
