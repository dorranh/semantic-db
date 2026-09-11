# Catalog prior art and integration direction

Assessment: September 11, 2026. These are design recommendations, not implemented
external-catalog integrations.

## Recommendation

Use DataFusion's `TableProvider` as the executable relation contract now. Keep
Semantic DB's `Catalog` as a small, validated session projection that teams can
populate from their own metadata. Evaluate **Apache Ossie (formerly Open Semantic
Interchange/OSI)** before designing a durable semantic catalog format. Ossie is
an incubating project targeting vendor-neutral semantic model exchange, with
JSON/YAML specifications and reference converters for other formats. That makes
it the closest standard candidate found in this pass.
[Apache Ossie project](https://github.com/apache/ossie)

There are three separate integration needs: business definitions, physical table
discovery, and executable scans. Our recommendation is to adapt each at its own
boundary rather than make one external catalog service mandatory for embedding.

## Comparison

| Prior art | What it supplies | Fit for this project |
| --- | --- | --- |
| [DataFusion catalogs and providers](https://datafusion.apache.org/library-user-guide/catalogs.html) | Catalog/schema/table hierarchy and pluggable table providers | Best execution interface. Already used. An existing `SchemaProvider` can supply providers behind `RelationBackend`; business definitions still need a semantic projection. |
| [Apache Ossie core specification](https://github.com/apache/ossie/blob/main/core-spec/spec.md) | Datasets, fields, keys, relationships, metrics, dialect-specific expressions, AI context, and extensions | Strongest candidate for portable semantic definitions. The inspected main-branch spec is explicitly a `0.2.0.dev0` draft; pin a released schema/version when prototyping an importer. |
| [dbt semantic models / MetricFlow](https://docs.getdbt.com/docs/build/semantic-models) | Semantic graphs over dbt models, entities/join keys, dimensions, and metric definitions | Good upstream source for teams already using dbt, and useful modeling precedent. Adopting its whole model would orient this general relation engine toward metric analytics. Current dbt docs also describe Ossie documents as an alternative authoring format. |
| [Iceberg REST catalog](https://iceberg.apache.org/rest-catalog-spec/) | Standard HTTP API for Iceberg catalog operations | Good physical discovery boundary for an Iceberg backend. It does not supply this engine's business-meaning and grounding layer; use it behind an adapter when Iceberg data is the actual need. |
| [Apache Gravitino relational metadata](https://gravitino.apache.org/docs/next/manage-relational-metadata-using-gravitino/) | Catalog/schema/table metadata management through APIs and clients | Useful upstream metadata service for teams already operating it. Requiring that service would add infrastructure to the minimal library path. |
| [Substrait logical relations](https://substrait.io/relations/logical_relations/) | Relational plan interchange, including reads of named tables | Relevant to a future plan/federation boundary. It does not replace semantic catalog authoring or a connector's table-resolution implementation. |

These fit assessments are project-specific judgments. The source links describe
the underlying contracts, not a claim that integrations already exist here.

## How an Ossie adapter would map

Ossie's dataset name/source/description map naturally to relation identity,
backend source key, and description. Its field expressions and logical data
types would need explicit lowering and schema resolution: for example, its
logical `Integer` does not fix Arrow integer width or signedness. Keys,
relationships, and metrics need richer runtime contracts than the current
`Relation`. The draft's AI context and extension fields also require a deliberate
projection policy. [Ossie core specification](https://github.com/apache/ossie/blob/main/core-spec/spec.md)

The next concrete experiment should import a small real team model. Pin the
interchange version; map physical references through the team's backend; resolve
Arrow schemas; reject unsupported expressions and semantics with explicit
diagnostics. Preserve source identifiers and version information for evidence.
Test equivalent SQL results and grounding behavior against the hand-authored
catalog. Do not silently discard metrics, relationships, or dialect differences
and claim a lossless import.

Keep units, domain concepts, and cardinality enforcement as explicit requirements
to evaluate in that experiment. The current engine has descriptions and grain,
but does not enforce these semantics. A richer interchange file alone cannot
provide deterministic semantic correctness.

## What this pass implements

`Engine::from_catalog` accepts owned relation definitions from any iterator;
`RelationBackend` resolves base relations to standard DataFusion providers;
the engine validates schemas and derives view dependencies. The facade provides
one library dependency and an optional compiler. No external catalog service,
YAML format, or custom scan protocol is required.

The current load is eager for provider resolution and intended for a bounded
catalog snapshot. For large or remote catalogs, a later design should combine
permission-aware retrieval with on-demand resolution, stable IDs, and revisions.
That is the point to consider deeper integration with DataFusion's catalog
hierarchy rather than expanding this snapshot loader into a separate catalog
server.
