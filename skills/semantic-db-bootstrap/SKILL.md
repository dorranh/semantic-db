---
name: semantic-db-bootstrap
description: Create or extend a Semantic DB project from available data sources, interactively author its Ossie model and SQL views, and verify a real query. Use for data onboarding, not ad hoc querying or connector development.
---

# Bootstrap a Semantic DB project

Find the user's source locations and the questions the project should answer. Inspect an existing project before editing it; do not replace its model or bindings to obtain a fresh scaffold. For a new project, use the installed CLI's `init` command. Check `<cli> --help` and `<cli> repl --help` first: the current source builds `sdb` with `sdb init DIRECTORY` and `sdb repl --project-config semantic-db.yaml`, while older installations may use a different executable or command shape.

Use built-in connectors for supported local files, PostgreSQL, and ClickHouse. The current `file` connector covers CSV, newline-delimited JSON, Parquet, Avro, and Arrow IPC. Check the installed version's options and transport limits before configuring a source. If no connector supports the source, explain the custom source-build path rather than pretending its source key is a URL or file path. Never weaken a required transport policy to make a connection work. Keep secret values in the host environment or `.env`, with references in project config and examples in `.env.example`.

Inspect the physical schema and a bounded, representative sample when access permits. Work with the user to establish row grain, exposed fields, identifiers, units, aliases, and definitions needed for their questions. Use those answers to populate Ossie. Ask about ambiguous business rules; leave unresolved descriptions as explicit `TODO:` placeholders and identify them in the handoff. Do not infer a threshold, unit, key, or business predicate from a column name. Declare keys only when confirmed; Ossie does not enforce uniqueness.

The dataset `source` must match a configured source key exactly. Map each exposed field to one physical column with its exact spelling; quote physical names when needed. Use logical datatypes to guide CSV/JSON parsing and check them against database or embedded-file schemas. Identifiers with leading zeros should remain strings. Use physical type overrides only for missing details such as a confirmed decimal precision and scale. Preserve existing bindings and resource settings when extending a project.

Create authored SQL views for confirmed, reusable business definitions. Register each SELECT file in the project `views` map; putting a file in `views/` alone does not load it. If a definition remains unclear, skip that view rather than making executable SQL from a guess. Descriptions and TODOs are metadata, not enforced filters.

Run offline validation, then connected validation, then a bounded SQL query that checks representative rows or aggregates against an expected result. With the current CLI these are `sdb validate`, `sdb validate --connect`, and `sdb sql 'SELECT ... LIMIT 10'`. Connected validation checks schemas, not row access or every value. If Ask is already configured, optionally preview and verify an authored-view question with `sdb ask --compile-only QUESTION`; clarify or report unsupported outcomes rather than executing a fallback with invented meaning. SQL verification is required even when Ask works.

Finish with the working project, verified query and result, remaining TODOs, and source or model limitations. Do not describe validation, evidence, or a successful query as proof that unconfirmed business definitions are correct.
