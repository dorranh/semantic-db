# Ossie integration exploration

Assessment: September 11, 2026. This is an integration design and validated input
fixture; no Ossie importer or runtime dependency has been added.

## Recommendation

Build a small, optional Rust importer for Ossie's **core semantic models** as the
next interoperability slice. Keep the original document alongside a validated
execution projection. Continue using DataFusion providers for data access and
Semantic DB relations for query execution.

Ossie is a useful way to receive authored business definitions from other tools.
Its repository contains a specification, Python model types, validation tools,
and vendor converters. Those are useful interchange components, but adopting them
does not supply Semantic DB's query planning, source connections, or deterministic
grounding implementation. See the [upstream project][project] and
[converter guide][converters].

Treat ontology support as a separate follow-up. Ossie's ontology defines entity
and value concepts, relationships, derivation rules, constraints, and mappings to
datasets. That is directly relevant to our planned concept grounding, including
units and domain predicates. It needs much more than translating those rules into
our current `Concept.definition` string: we would need typed semantics and an
evaluator. The [ontology specification][ontology] and [flights example][flights]
are useful design inputs now.

## Reproducible baseline

Inspected upstream commit:
`28365cd638f3833765c5b940ada5b8cbc65f1c42` (September 10, 2026).
All upstream file links below use that commit.

- The core specification and JSON Schema target `0.2.0.dev0`; the prose explicitly
  calls this a draft. Use [the JSON Schema][schema] for document structure.
- GitHub's releases API returned no releases. The remote tag listing contained
  `osi-0.1.1-rc1`, resolving to `faf581054dcf7964d5fe0ceae7d6f415c8ce32a5`.
  That is a release candidate, not a stable baseline. For this experiment, pin the
  inspected commit as well as the document version. Recheck before shipping.
- The [expression language document][expressions] is marked “Proposed Final” and
  proposes an `Ossie_SQL_2026` dialect. That dialect is absent from the inspected
  core schema. Do not implement proposed syntax as if it were already accepted.
- The [Python package][python-package] is also `0.2.0.dev0`, requires Python 3.11+,
  and uses Pydantic and PyYAML. It is useful as a development reference; importing
  YAML into an embedded Rust library does not justify requiring Python at runtime.

The version alone is insufficient to reproduce a moving development schema.
Record upstream commit, schema digest, document digest, and adapter revision in
an import report. These are proposed provenance fields, not current catalog APIs.

## Mapping into the current skeleton

The [core specification][spec] describes the source constructs. The right-hand
column below is our proposed policy, based on the current catalog and engine.

| Ossie construct | Proposed Semantic DB mapping and boundary |
| --- | --- |
| Model and dataset names | Select one model explicitly. Map dataset names to unique, lowercase, unqualified relation names; reject collisions and unsupported names rather than silently normalizing them. Preserve original identity separately. |
| Dataset `source` | Pass through an application-owned source binding. A physical reference is not automatically a Semantic DB SQL name. Treat query-valued sources as unsupported in the first importer. |
| Dataset description | Populate `Relation.description`. Model descriptions need a separate metadata home. |
| Identity field expressions | Expose exactly the declared fields, in order. Check references against the bound provider. Never expose extra physical columns through an implicit `SELECT *`. |
| Aliases and computed fields | Later, lower checked scalar expressions to a projection provider or a view over a private source. Do not register the raw provider under the semantic name and pretend the expressions ran. |
| Logical `datatype` | Check compatibility with a resolved Arrow type. Obtain width, decimal precision/scale, timezone, and nullability from the provider or an explicit application schema contract. Do not guess `Integer = Int64`. |
| Field descriptions, labels, time roles | Preserve in the imported document. Add typed field metadata before claiming these reach grounding. Arrow metadata alone is currently omitted from the compiler prompt. |
| Primary/unique keys and relationships | Preserve declarations; diagnose unsupported execution semantics. Do not translate keys into a claim of enforced uniqueness or automatically plan joins yet. |
| Metrics | Preserve aggregate definitions; block executable import in the first profile. A metric requires grouping/filter context and join semantics, so a metric name cannot simply become a fixed SQL view. |
| `ai_context` | Preserve as source metadata. Later expose selected synonyms/examples as catalog evidence, with a deliberate policy for free-form instructions. |
| Custom extensions | Retain opaque payloads and identify them in diagnostics. Unknown extensions may carry filters or other behavior, so preservation alone does not establish executable equivalence. |
| Ontology documents | Recognize and report an unsupported document kind in the core importer. Plan concept grounding separately. |

Two implementation details deserve attention:

1. `RelationBackend::resolve` currently receives a `Relation` that already contains
   an Arrow schema. Ossie does not supply that full schema. For the first slice,
   let the application bind each source to an existing `TableProvider`, derive the
   schema from that same provider, and reuse it during registration. Avoid a dummy
   schema or resolving the source twice. This can live in the adapter without
   changing the existing backend trait.
2. `Engine::register_table` compares complete Arrow schemas, including metadata.
   Attaching Ossie annotations directly to only the relation schema would break
   registration. Keep semantic annotations separate from physical Arrow schemas.

The relevant code is in [the catalog](../crates/semantic-catalog/src/lib.rs),
[the engine](../crates/semantic-engine/src/lib.rs), and the compiler's
[`catalog_context`](../crates/semantic-compiler/src/lib.rs). The compiler currently
receives relation descriptions/grain, view SQL, and column names/types/nullability;
it does not consume relationships, metrics, field descriptions, or ontology rules.

## Validation findings

These are observed results against the pinned checkout, not assumptions about
future Ossie versions. The temporary validation environment used PyYAML 6.0.3,
jsonschema 4.26.0, SQLGlot 30.12.0, and Pydantic 2.12.5.

| Probe | Observed result | Consequence for our importer |
| --- | --- | --- |
| Upstream TPC-DS example | Passed the upstream validator; contains 5 datasets, 31 fields, 4 relationships, and 5 metrics. `customer_full_name` is a computed concatenation. | Useful later as a coverage fixture; cannot be claimed as fully supported by an identity-field importer. |
| Wrong document version, missing version, unknown dataset property | JSON Schema rejected each case; the Python document model accepted each. | Parsing through upstream Python types is not equivalent to schema validation. |
| Relationship with two source columns and one target column, including missing source fields | Passed the validator's schema, unique-name, reference, and SQL checks. | Add our own equal-length, nonempty, duplicate-column, and resolvable-reference checks. Validate against bound schemas when fields are omitted. |
| Flights ontology | Passed ontology JSON Schema validation when its core-schema reference was bound to the pinned local schema. | Bundle/register referenced schemas for offline validation. The upstream CLI attempted to fetch a `main` URL and failed without network access. |

These probes exercise [the validator][validator], [Python models][python-models],
[TPC-DS model][tpcds], and [ontology schema][ontology-schema]. The flights result
establishes structural validity only; it does not validate or execute ontology
constraints. SQLGlot syntax acceptance likewise does not establish DataFusion
compatibility, reference binding, or correct aggregate behavior.

Use separate validation stages: safe parsing with duplicate-key detection, pinned
JSON Schema validation, semantic reference checks, capability checks, then provider
binding and DataFusion planning. Return document paths and stable diagnostic codes
for every unsupported construct. Produce no runnable import when blocking
diagnostics remain. An inspection mode may preserve and report the whole document
without presenting it as an executable catalog.

## First implementation slice

Proposed location: a `semantic-ossie` crate, exposed through an opt-in `ossie`
feature on the `semantic-db` facade. Its first executable profile should support
one selected core model, explicit nonempty field lists containing identity
`ANSI_SQL` column expressions, dataset descriptions, and provider-derived schemas.
Missing/empty fields, computed expressions, query sources, non-ANSI-only
expressions, and unsupported semantic annotations should produce explicit
diagnostics. Empty optional collections are harmless. Keep unsupported definitions
available to inspection, but require a clean executable profile to load a catalog.

Start with [team_catalog.yaml](../examples/ossie/team_catalog.yaml), authored from
our existing [Rust orders example](../crates/semantic-db/examples/team_catalog.rs).
It is a synthetic compatibility fixture, not evidence that a real team's model
has been imported. It deliberately covers only the base `orders` relation; the
application continues to supply Arrow nullability, owner/grain, and the
`completed_orders` view.

The [wells mapping](../examples/geospatial/README.md) also expresses the existing
geospatial fixture in Ossie, including its key, field descriptions, units in
prose, and AI context. It is a richer metadata-preservation example beyond the
minimal orders profile; the existing query still returns W-001 and W-004.

Acceptance criteria for that implementation:

1. Import the fixture using the existing in-memory orders provider. Compose the
   same application-owned `completed_orders` view and return order IDs **1 and 3**.
   Compare schema and results with the hand-authored catalog.
2. Produce the same compiler catalog projection after applying the same
   application metadata. Use an offline provider stub to test evidence references;
   do not depend on a live LLM for deterministic importer tests.
3. Reject version drift, duplicate YAML keys/names, name collisions, unknown
   sources/columns, incompatible logical types, unsupported dialects, and semantic
   constructs outside the supported profile. Check that no partially loaded engine
   is returned and that extra source columns are never exposed.
4. Inspect the complete TPC-DS model and report every unsupported construct with
   source paths, while preserving the original document. Do not count this as
   executable TPC-DS support.

After this slice, add field metadata and selected computed expressions, then
relationships/metrics with tests for join fanout and aggregation. Use a real team
model to choose which capabilities come next. Catalog persistence and ontology
execution remain separate increments; Ossie files do not themselves provide
catalog revisions, transactions, or materialization freshness.

To reproduce structural validation of the local fixture with an upstream checkout
and a Python environment containing the versions listed above:

```sh
git clone https://github.com/apache/ossie.git /tmp/ossie
git -C /tmp/ossie checkout 28365cd638f3833765c5b940ada5b8cbc65f1c42
python /tmp/ossie/validation/validate.py examples/ossie/team_catalog.yaml
python /tmp/ossie/validation/validate.py /tmp/ossie/examples/tpcds_semantic_model.yaml
```

[project]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/README.md
[spec]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/core-spec/spec.md
[schema]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/core-spec/ossie-schema.json
[expressions]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/core-spec/expression_language.md
[ontology]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/ontology/ontology.md
[ontology-schema]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/ontology/ontology.json
[flights]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/examples/flights.yaml
[tpcds]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/examples/tpcds_semantic_model.yaml
[validator]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/validation/validate.py
[python-package]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/python/pyproject.toml
[python-models]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/python/src/ossie/models.py
[converters]: https://github.com/apache/ossie/blob/28365cd638f3833765c5b940ada5b8cbc65f1c42/converters/README.md
