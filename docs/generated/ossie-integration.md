# Ossie integration assessment

This document records the original upstream assessment and implementation. For
current executable support, including column aliases and offline inspection, use
the [Ossie reference](ossie-reference.md). For onboarding, use
[Add a dataset](adding-datasets.md). Identity-only limitations below describe the
initial September 11 baseline.

Assessment and first implementation: September 11, 2026. The optional Rust
importer now loads the wells and orders core models with explicit source bindings.
The research findings below refer to the pinned upstream checkout.

## Recommendation

The `semantic-ossie` crate imports a supported subset of Ossie's **core semantic
models**. It retains the original document alongside a validated execution
projection and uses DataFusion providers for data access. The facade exposes it
through an opt-in `ossie` feature; no Python runtime is needed.

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
`Relation.semantics.origin` records the upstream commit, schema digest, original
document digest, document version, model/dataset identity, and adapter package
version. The latter is not a Git revision; release management must version adapter
changes if it needs to distinguish builds of this unpublished package.

## Mapping into the current skeleton

The [core specification][spec] describes the source constructs. The right-hand
column below distinguishes the implemented profile from future capabilities.

| Ossie construct | Semantic DB mapping and boundary |
| --- | --- |
| Model and dataset names | Select one model explicitly. Map dataset names to unique, lowercase, unqualified relation names; reject collisions and unsupported names rather than silently normalizing them. Preserve original identity separately. |
| Dataset `source` | Resolve only through an explicit application-owned source binding. The string is an opaque key; it is never automatically interpreted as a file, URL, or executable query. |
| Dataset description | Populate `Relation.description`; retain model descriptions in `Relation.semantics`. |
| Identity field expressions | Expose exactly the declared fields, in order. Check references against the bound provider. Never expose extra physical columns through an implicit `SELECT *`. |
| Aliases and computed fields | Later, lower checked scalar expressions to a projection provider or a view over a private source. Do not register the raw provider under the semantic name and pretend the expressions ran. |
| Logical `datatype` | Check compatibility with a resolved Arrow type. Obtain width, decimal precision/scale, timezone, and nullability from the provider or an explicit application schema contract. Do not guess `Integer = Int64`. |
| Field descriptions, labels, time roles | Retain in typed `Relation.semantics.fields` and expose to the compiler. These annotations remain separate from physical Arrow metadata. |
| Primary/unique keys and relationships | Retain keys with an `unenforced_keys` warning. Reject relationship import until join semantics are implemented. |
| Metrics | Preserve aggregate definitions; block executable import in the first profile. A metric requires grouping/filter context and join semantics, so a metric name cannot simply become a fixed SQL view. |
| `ai_context` | Expose strings or known instructions/synonyms/examples as catalog evidence. Reject unknown context properties; instructions cannot override compiler rules, and examples cannot supply implicit defaults. |
| Custom extensions | Retain in the original document and reject executable import. Unknown extensions may carry filters or other behavior. |
| Ontology documents | Reject through core schema validation. Ontology execution remains a separate future capability. |

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
it now also consumes imported `Relation.semantics`, including field descriptions
and AI context. Relationships, metrics, and ontology rules remain unsupported.

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

## Implemented import profile

`semantic-ossie` is exposed through the facade's opt-in `ossie` feature. The CLI
uses the same library with `--ossie PATH`, optional `--ossie-model NAME`, and
repeatable `--source-csv SOURCE=PATH` bindings. See the
[wells instructions](../examples/geospatial/README.md) and
[embedding guide](embedding.md#ossie-models) for runnable commands.

Supported: one selected core model, explicit nonempty field lists with identity
`ANSI_SQL` expressions, provider-derived schemas, compatible logical types,
dataset/model/field descriptions, field labels/time roles, known AI context
properties, and declared primary/unique keys. Key columns must reference distinct
declared fields. Row uniqueness and non-nullness are not enforced; import returns
a warning for datasets with keys. Other supplied dialect variants are retained in
the original document with a warning that ANSI_SQL was selected.

The adapter rejects missing/empty fields, aliases/computed expressions, missing
ANSI_SQL expressions, duplicate dialects/names, unknown sources/columns,
incompatible logical types (including `Opaque`), relationships, metrics, and
custom extensions. Unknown structured AI context properties also fail instead of
being discarded. Full schema validation precedes capability checks. Diagnostics
carry stable codes and document paths. Schema validation checks every model;
executable capability checks apply to the selected model. Model names must be
unique across the document. Original text and parsed JSON remain available on
`OssieDocument` even when executable import fails.

The [wells model](../examples/geospatial/wells.ossie.yaml) runs through the importer
and returns **W-001 and W-004**. Tests compare schema and query results with the
direct CSV path, compose a view, check declared-column visibility and type
compatibility, and verify that authored metadata reaches an offline compiler stub.
This verifies the compiler input contract, not a live LLM's interpretation.

The simpler [orders model](../examples/ossie/team_catalog.yaml) is also supported;
bind its `warehouse:orders` source to the application's provider. Neither fixture
establishes interoperability with a real team's production model.

Next, add selected computed expressions, then relationships/metrics with tests
for join fanout and aggregation, guided by a real team model. The full upstream
TPC-DS model is still outside the supported execution profile. Catalog persistence
and ontology execution remain separate increments; Ossie files do not themselves
provide revisions, transactions, or materialization freshness.

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
