# Add a dataset

Use this path when your data is accessible through an [available connector](connectors.md).
You need an Ossie model, a configured connection, and a binding for each source.
No Rust or Arrow schema definitions are required. If your application has already
configured the source bindings, only the Ossie model needs to change.

## 1. Start with a working project

From the repository root:

```sh
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml --validate
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml --validate --connect
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml --file examples/geospatial/query.sql
```

The last command returns W-001 and W-004. Copy the `examples/geospatial` folder
as a starting point, or use the minimal model below. For GitHub, start with
`examples/github/semantic-db.yaml`; both projects use the same CLI.

## 2. Describe your dataset in Ossie

For a CSV with an `ORDER_ID` header, save this as `orders.ossie.yaml`:

```yaml
version: 0.2.0.dev0
semantic_model:
  - name: sales
    datasets:
      - name: orders
        source: sales.orders
        description: Customer orders. One row per order.
        fields:
          - name: order_id
            description: Identifier assigned to an order.
            datatype: Integer
            expression:
              dialects:
                - dialect: ANSI_SQL
                  expression: '"ORDER_ID"'
```

`orders` and `order_id` are SQL names. `sales.orders` is a binding key, not SQL
or a URL. The expression selects one physical column, with an optional rename
through the field's `name`. Use double quotes for physical names with uppercase,
spaces, or punctuation. Physical names must match the provider exactly.

List only the fields you want exposed, in the desired order. Providers supply
physical Arrow types and nullability; optional logical `datatype` declarations
are checked against them. Describe grain, units, and ambiguous terminology.
Descriptions help natural-language queries but do not execute predicates.

This is a supported subset of Ossie. Computed expressions, metrics,
relationships, and custom extensions are rejected. Declared primary/unique keys
produce warnings because their constraints are not enforced. See the
[Ossie reference](ossie-reference.md) before importing an existing model.

## 3. Bind the source

Place your `orders.csv` next to this `semantic-db.yaml`:

```yaml
ossie: orders.ossie.yaml
connections:
  local:
    connector: csv
sources:
  sales.orders:
    connection: local
    path: orders.csv
```

The binding key must match the dataset's `source`. Model and CSV paths resolve
relative to this configuration file, so the project works from another directory.
A second CSV dataset can reuse `local`: add its Ossie definition and its own entry
under `sources`. A source reused by multiple datasets is resolved only once per load.

Connection-specific settings and secrets never belong in Ossie. API connectors
may require bounded scope such as a repository set; SQL queries cannot expand
that scope. The [configuration reference](connectors.md) lists the actual options.

## 4. Validate, inspect, query

Using the local binary installed with `cargo install --path apps/semantic-cli --locked`:

```sh
semantic-db --config semantic-db.yaml --validate
semantic-db --config semantic-db.yaml --inspect
semantic-db --config semantic-db.yaml --validate --connect
semantic-db --config semantic-db.yaml --query 'SELECT order_id FROM orders LIMIT 10'
semantic-db --config semantic-db.yaml
```

Offline validation checks executable model support, required bindings, connection
references, and connector options. It needs no credentials or source access.
Connected validation additionally obtains providers and checks column names,
types, and mappings. CSV inference reads a sample; remote providers may obtain
metadata. Fixed-schema API providers can pass without making an API request,
so run a bounded query to check actual row access.

Start with explicit SQL before adding natural language. Set `OPENAI_API_KEY` in
the environment or working directory's `.env` to use `--ask` or interactive `.ask`.
Use `--ask 'Count orders' --dry-run` to inspect generated SQL without reading query
rows; this still calls the model. See [CLI reference](cli.md).

## Fix common onboarding errors

| Diagnostic | Action |
| --- | --- |
| `unknown_connector` | Use an available connector, or compile/register your custom factory. |
| `missing_binding` | Add the reported Ossie source key under `sources`. |
| `missing_connection` | Define the referenced name under `connections`. |
| `missing_secret` | Supply the configured variable through environment/`.env` or your embedded resolver. |
| `missing_column` | Check the physical spelling/case in the field expression and the selected source. |
| `type_mismatch` | Correct the logical type or normalize the source/provider schema. |
| `unsupported_expression` | Use a single column reference; prepare calculations upstream or create an explicit SQL view. |
| `unsupported_feature` | Remove or remodel the unsupported feature; do not assume it was executed. |
| `unenforced_keys` | A warning: the key is descriptive. Validate source uniqueness separately if you rely on it. |

Add one SQL fixture and expected result to your dataset folder. For changing API
data, use a deterministic fixture for correctness and a separate optional live
smoke query. A model file does not establish a transactional snapshot or enforce
business meaning.
