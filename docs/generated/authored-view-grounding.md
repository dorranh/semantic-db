# Grounding with authored views

Business definitions live in SQL views. The optional authored-view compiler
selects one of those definitions and lowers a typed query over it. There is no
separate concept registry or duplicate predicate definition to maintain.

For example, a team can explicitly define `active_deep_wells` using
`status = 'active' AND total_depth_m >= 2500`. Once selected, that view is always
the generated query's source. The model cannot substitute the base table, rewrite
the threshold, or cite the view while returning unrelated SQL.

## Try it

From the repository root, with the usual model configuration for natural-language
queries:

```sh
cargo run -p semantic-cli --locked -- \
  --config examples/geospatial/semantic-db.views.yaml \
  --ask-views "List well IDs for active deep wells in North Basin, ordered by well_id" \
  --dry-run
```

Remove `--dry-run` to execute. A faithful selection produces W-001 and W-004.
This project authors an example convention; the underlying Ossie model explicitly
does not define a universal meaning of “deep.” Views over Ossie datasets use the
importer's logical fields and projections as usual.

The REPL offers `.ask-views REQUEST` and `.plan-views REQUEST` over loaded views or
definitions registered with `.view`. Existing `--ask`, `.ask`, and `.plan` retain the general
SQL-proposal mode. The view mode never falls back to that mode on failure.

An offline Rust example loads the same Ossie model and directly lowers a supplied
typed selection, without an API key or model call:

```sh
cargo run -p semantic-db --features ossie --example authored_views --locked
```

## Author views in the project

Add a `views` mapping to `semantic-db.yaml`, alongside `ossie`, `connections`, and
`sources`. Each entry requires a `sql_file` and accepts an optional `description`:

```yaml
views:
  deep_wells:
    description: Wells meeting our team's depth convention of at least 2500 metres.
    sql_file: views/deep_wells.sql
  active_deep_wells:
    description: Active wells meeting our team's depth convention.
    sql_file: views/active_deep_wells.sql
```

`views/deep_wells.sql` contains the defining query, without `CREATE VIEW`:

```sql
SELECT * FROM wells WHERE total_depth_m >= 2500;
```

`views/active_deep_wells.sql` can compose that definition:

```sql
SELECT * FROM deep_wells WHERE status = 'active';
```

SQL paths are relative to the project YAML directory, including paths written as
`views/...`; they are not relative to the process's working directory. Absolute
paths also work. Files contain exactly one query; an optional trailing semicolon
is accepted. Inline `sql` and unknown view options are rejected in this profile.

Map order does not matter. The loader checks names and dependency graphs before
connecting, then infers each view's schema and registers it in dependency order.
Names must follow the engine's lowercase identifier rules and cannot collide with
datasets in the selected Ossie model or other views. Descriptions become catalog
metadata and are available to both compiler modes. Project views are scoped to
the selected model's datasets; cross-model references are not supported.

`--inspect` lists view files, descriptions, and direct dependencies in loading
order. `--validate` checks files, query syntax, relation references and cycles
without credentials or source access. `--validate --connect` additionally checks
columns and types against provider schemas. It plans views without executing
their rows. Errors identify `/views/<name>/sql_file` or the affected view name.

The shared Rust loader handles the same definitions: `Project::from_path` reads
and captures SQL files; `Project::load` registers them. `Project::new` also reads
configured SQL files relative to its supplied base directory. `Project::inspect`
keeps its original model-inspection return type and also validates project views.
`Project::inspect_project` returns `ProjectInspection`, with the Ossie inspection
under `.model` and ordered view definitions under `.views`.

Keep the YAML and SQL files in version control. Each new Project or CLI startup
reloads their definitions. An existing Project retains the SQL it read, so editing
a file between inspection and load does not change that Project's definition.
There is no file watching or write-back: `.view` and `--view` still add session-only
definitions, and duplicate names are rejected instead of replacing loaded views.
Persistent revision tracking, transactions, and materialization remain future work.

## Contract and execution

`Compiler::compile_views(&engine, request)` returns the normal `Compilation`, with
`view_selection` populated only for a successfully lowered selection. Applications
may also call `compiler::views::lower_view_selection` with a `plan::ViewSelection`
they have constructed or reviewed themselves. Both paths plan without collecting
rows and generate binding evidence from the actual registered view definition.

The model sees the catalog's views, their SQL definitions, descriptions, semantic
annotations, and output schemas. Base relations are not selection candidates.
Nested views are supported; their definitions remain executable in the engine.
No-view catalogs return unsupported without invoking the model.

The typed response selects one view, an exact binding phrase from the request,
output columns, optional filters, and optional ordering. The supported operations
are:

| Operation | Checked behavior |
| --- | --- |
| Projection | Nonempty, unique, exact output column names; identifiers are quoted |
| Comparisons | `eq`, `not_eq`, `lt`, `lte`, `gt`, `gte`; all filters are ANDed |
| Literals | Text, JSON-number spelling, or lowercase `true`/`false`, copied verbatim from the request and compatible with the column's physical type |
| Null tests | `is_null` and `is_not_null` |
| Ordering | Exact output column names, ascending or descending, always nulls last |

Numeric spelling is retained without a float round trip in generated SQL; numeric
fragments such as `500` inside `2500`, `-500`, or `2,500` are rejected. Text values
are quoted as SQL literals. Text-to-number and other implicit literal-kind
conversions are rejected. Normal DataFusion numeric comparison/coercion rules
still apply; the mode does not introduce exact arithmetic beyond those rules.
Numbers written with grouping separators or words need clarification into a
supported spelling. Empty text literals and date/time literals are outside this
first profile.

The model cannot submit arbitrary expressions, joins, OR, aggregates, DISTINCT,
limits, or SQL. Such operations may exist inside an authored view; additional
operations outside it require an unsupported outcome. Invalid output receives the
same bounded repair policy as general compilation. Well-formed clarification and
unsupported outcomes return immediately.

## What this proves, and what it does not

The deterministic guarantee is that the selected view's definition is applied,
its fields are resolved, additional literal values came from the request, and
the resulting query can be planned. Binding evidence is generated by the compiler,
not accepted as a model-written explanation of arbitrary SQL.

Choosing the applicable view remains a model judgment. An exact phrase match does
not prove its meaning. Literal occurrence does not prove that the value belongs
to that column, that the comparison direction is right, or that a unit matches.
The model can still omit an additional user constraint, select an overly broad
view, or add a semantically inappropriate filter using a mentioned value.
Missing location-quality data and competing definitions are prompted to produce
unsupported or clarification, but there is no deterministic completeness checker
over unrestricted natural language yet.

Authored definitions must therefore be reviewed for applicability, units, join
grain and correctness. Descriptions help interpretation; they do not execute unit
conversions or enforce business constraints. No permission filtering, persistent
definition revisioning, or source snapshot guarantee is added. The caller must
execute the returned SQL against the same catalog session for the recorded
binding to apply.

The next increment should be driven by a real team's definitions and requests:
evaluate wrong-view selection and dropped constraints, then extend the typed
contract only for concrete operations those requests need.

## Verification

Compiler tests compare nested-view results with explicit reference SQL, reject
base-table/SQL substitutions and invented literals, and exercise bounded repair
and unresolved outcomes. Lowering tests cover all comparison operators, decimal
spelling, quoted identifiers, SQL-looking text, nulls, sorting, type mismatches,
and compilation without reading rows. Project tests cover file reloads, dependency
ordering, offline rejection and schema diagnostics. CLI tests exercise inspection
from another working directory and both execution and dry run over configured
Ossie-backed views using a local HTTP model stub. These deterministic
tests verify the contracts, not a live model's interpretation accuracy.
