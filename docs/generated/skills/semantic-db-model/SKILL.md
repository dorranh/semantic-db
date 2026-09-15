---
name: semantic-db-model
description: "Author or refine executable Ossie models and business-definition SQL views for Semantic DB, including CSV/JSON parsing guidance."
---

# Define meaning with Ossie and authored views

Inspect existing project bindings and the actual physical source schema before changing mappings. Ossie supplies semantic names, descriptions, logical types, and physical-column mappings. Connections, source paths, and secret references remain in project configuration.

A minimal mapped field looks like:

```yaml
version: "0.2.0.dev0"
semantic_model:
  - name: sales
    datasets:
      - name: products
        source: local.products
        description: One row per product.
        fields:
          - name: product_id
            datatype: String
            expression:
              dialects:
                - dialect: ANSI_SQL
                  expression: '"PRODUCT_ID"'
```

Physical spelling/case must match exactly. Quote physical names when needed; exposed names use the supported lowercase identifier profile. Declare identifiers with leading zeros as String. CSV/JSON loading uses declarations to guide parsing; databases and embedded-schema files retain their actual physical types, checked against declarations. A shared source must have consistent declarations across datasets. Decimal requires an explicit physical precision/scale override; do not invent one.

Document grain, units, keys, and business vocabulary. Keys are descriptive rather than enforced uniqueness constraints. Only select fields intended for exposure. Do not assume every construct in the upstream Ossie schema is executable: computed fields, metrics, and relationships require checking the installed importer profile.

Put executable business definitions in authored SQL views, registered with `views: {name: {sql_file: views/name.sql, description: ...}}` in project configuration. Files contain a SELECT query, not CREATE VIEW. Descriptions alone do not enforce predicates. Confirm ambiguous terms, thresholds and units with the user; explicit examples from existing business logic can establish them.

Validate offline, then with `--validate --connect`, and run bounded queries covering representative cases and boundary values. Demonstrate authored-view Ask with `.plan-views`/`.ask-views` or the batch `--ask-views` option. Do not describe evidence references or valid SQL as proof of business correctness. Preserve existing physical bindings and any transactional resource configuration.
