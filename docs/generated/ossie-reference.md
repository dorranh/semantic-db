# Supported Ossie profile

This is the executable import reference. Start with [Add a dataset](adding-datasets.md)
for a walkthrough. The [historical assessment](ossie-integration.md) records the
upstream evaluation and original identity-only implementation.

The bundled core JSON Schema is `0.2.0.dev0`, pinned to upstream commit
`28365cd638f3833765c5b940ada5b8cbc65f1c42`. YAML/JSON parsing rejects duplicate keys;
schema validation is offline and requires no Python or downloads. Provenance
retains schema/document digests, the revision, and model/dataset identities.

## Supported constructs

| Construct | Behavior |
| --- | --- |
| Model selection | One exact model name, or automatic selection if the document contains exactly one. Duplicate model names fail. |
| Dataset and field names | Unique lowercase, unqualified SQL identifiers: letters, digits and underscores, starting with a letter or underscore. |
| Dataset `source` | Opaque binding key, explicitly resolved by the application/project. Never interpreted as SQL, a path, or URL. |
| Fields | Required nonempty list; expose only declared fields, in document order. |
| Field expressions | One `ANSI_SQL` column identifier, optionally double-quoted. Field `name` can rename it. |
| Physical column names | Exact provider spelling. Use `"ORDER_ID"`, `"Order ID"`, or doubled quotes inside a quoted name. Unquoted simple identifiers also resolve exactly; no implicit case folding or qualified-table lookup. |
| Logical `datatype` | Optional; checked against the bound provider's physical type. Physical width, precision/scale, timezone, nullability, and metadata come from the provider. |
| Descriptions, labels, time roles, AI context | Retained as semantic metadata and made available to the compiler. |
| Primary/unique keys | Validate references to distinct declared semantic fields; emit `unenforced_keys`. Do not scan or enforce uniqueness/non-nullness. |
| Other dialect variants | Retained with a warning when `ANSI_SQL` is selected. Duplicate dialect entries fail. |

A field named `order_id` with expression `"ORDER_ID"` reads the physical column
and exposes only `order_id`. SQL fragments such as `x AS y`, `table.column`,
`price * quantity`, casts, and functions are unsupported. A quoted physical name
containing a dot is a literal column name, not a table qualification.

## Unsupported constructs

Computed expressions, relationships, metrics, custom extensions, unknown
structured AI context properties, and `Opaque` physical-type mappings are not
executable. Ontology documents fail core-schema validation. Unsupported supplied
semantics cause errors; they are never silently discarded to make a model run.

Logical compatibility: String → Arrow string types; Integer → signed/unsigned
integers; Float → floating types; Decimal → decimal types; Boolean → Boolean;
Date → Date32/Date64; Time → Time32/Time64; DateTime → timestamps without timezone;
DateTimeTz → timestamps with timezone. No automatic coercion is performed.

## Validation stages and diagnostics

`OssieDocument::parse` validates the entire document against the pinned schema.
`inspect(model)` selects a model, validates its executable semantics, and returns
source/field requirements plus warnings without providers. `load(model, bindings)`
uses the same checks, then validates bindings and physical schemas and constructs
a fresh engine. No partial engine is returned. Provider I/O already performed
cannot be rolled back.

Project `--validate` adds offline binding/connector-option checks. Use
`--validate --connect` for physical checks. Raw `--ossie --validate` checks only
the model. Diagnostics carry codes and document/configuration paths; common
remedies are in the [dataset guide](adding-datasets.md#fix-common-onboarding-errors).

`original_text()` and `json()` preserve the original schema-valid document even
when executable import fails. Semantic annotations stay separate from Arrow
metadata. Natural-language evidence is model-proposed; valid SQL and descriptive
units, keys, or grain do not prove business correctness.
