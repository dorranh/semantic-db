# Grounding with authored views

Business definitions can live in project SQL views. For example, a team may
define `active_deep_wells` with `status = 'active' AND total_depth_m >= 2500`.
SQL over that view applies the definition as written. The Ossie model itself
does not define a universal meaning of “deep.”

## Author a view

Add a `views` mapping to `semantic-db.yaml`, alongside the model, connections,
and sources:

```yaml
views:
  deep_wells:
    description: Wells meeting our team's depth convention of at least 2500 metres.
    sql_file: views/deep_wells.sql
  active_deep_wells:
    description: Active wells meeting our team's depth convention.
    sql_file: views/active_deep_wells.sql
```

`views/deep_wells.sql` contains a query, not `CREATE VIEW`:

```sql
SELECT * FROM wells WHERE total_depth_m >= 2500;
```

`views/active_deep_wells.sql` may compose it:

```sql
SELECT * FROM deep_wells WHERE status = 'active';
```

Paths are relative to the project YAML directory. The loader checks names,
syntax, dependencies, and cycles offline, then plans output columns and types
when connecting. Views load in dependency order and are available to SQL,
the REPL, general Ask, and the HTTP `/compile` endpoint.

## Query and ask

```sh
sdb validate --project-config examples/geospatial/semantic-db.views.yaml
sdb sql --project-config examples/geospatial/semantic-db.views.yaml \
  "SELECT well_id FROM active_deep_wells WHERE basin = 'North Basin' ORDER BY well_id"
sdb ask --project-config examples/geospatial/semantic-db.views.yaml \
  --compile-only "List well IDs for active deep wells in North Basin"
```

The SQL query returns W-001 and W-004. `--compile-only` contacts the model and
prints proposed SQL and grounding evidence without reading rows. Review that
proposal before execution. Ask can also return a clarification or unsupported
outcome.

General Ask sees both base relations and authored views. Its prompt prefers a
view when the definition fits the request, but that choice is a model judgment.
Validation checks generated SQL and evidence references; it cannot prove the
right view was chosen or that every natural-language constraint was retained.
There is no separate view-only compiler mode. Query a view directly with
`sdb sql` when its use must be explicit.

The offline Rust example runs SQL over an authored view without a model call:

```sh
cargo run -p example-authored-views --locked
```
