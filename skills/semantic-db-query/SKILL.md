---
name: semantic-db-query
description: Discover an existing Semantic DB project and answer read-only data questions through its CLI, using general Ask when configured and bounded SQL otherwise. Use for querying, not project setup or connector development.
---

# Query a Semantic DB project

Find the user's project config and check the installed CLI's help before running commands. Current source builds `sdb` and places query flags under `sdb repl`; older installations may use another executable or command shape. Use `--config PATH --inspect` to discover datasets, fields, views, and bindings. Inspect the relevant Ossie descriptions and authored view SQL when interpretation matters; a `TODO:` description is missing meaning, not evidence.

For a natural-language question with Ask configured, use general `--ask QUESTION --dry-run` first. This contacts the model and prints generated SQL and evidence without executing rows. Check that the SQL is read-only, matches the user's terms, and has an appropriate bound. Execute the reviewed SQL with `--query` so a second Ask call cannot generate a different query. General Ask can query the full catalog and has different grounding guarantees from authored-view Ask. Treat evidence as a model proposal and explain material interpretation choices.

Ask can return clarification or unsupported with exit status zero. Detect those outcomes from the output and do not answer by silently substituting another meaning. Ask the user for a missing business definition when needed. When Ask is not configured, build a read-only SQL query from the inspected catalog. Use `LIMIT` for exploratory rows because the CLI collects displayed results in memory; use bounded aggregates or filters for summaries. Do not run writes through this skill.

If connection or query execution fails, distinguish offline configuration, connected-schema, permission, and row-access failures. `--validate --connect` checks physical schemas but does not prove row access. Report the SQL, result, and any meaningful limit, freshness, or consistency caveat. Do not claim that valid SQL or Ask evidence proves a business definition.

When the user asks why a query is slow, inspect `--dry-run` or `--explain-read` before execution and use `--read-report` on a bounded run when relevant. Distinguish local operators from source pushdown, compare latency and transferred rows where measurable, and preserve results when suggesting an improvement. Dry-run is a plan, not a remote-cost estimate. Connector implementation or source changes belong to a separate task.
