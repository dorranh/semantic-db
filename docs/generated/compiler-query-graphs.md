# Semantic query graphs

The graph entry point extends the typed row/aggregate compiler with explicit set
and fact-composition operators. It is a versioned proposal contract, not executable
SQL. Every row-query leaf goes through the existing binder and retains its policies,
metric definitions, calendar interpretation, and runtime obligations.

## Contract

`semantic_plan::graph::GraphQuery` contains version 1, named nodes with request
source text, a root node, final output-slot ordering, an optional final limit, and
unresolved choices. Nodes can appear in any order. Compilation computes a bounded
topological order and rejects cycles, missing edges, duplicate identities, and
nodes that do not contribute to the root. An output edge refers to a requirement
or graph output ID, independently of its display alias.

A `rows` node contains a typed `RowQuery`. Its output requirement IDs become the
node's slots. All local aggregation/filter/window stages remain inside that node.

A `set` node names left/right inputs, `union`, `intersect`, or `except`, and explicit
`all` or `distinct` duplicate semantics. Each aligned output names a left slot,
right slot, new output ID and alias. Types must match exactly. Nulls compare as
equal for set membership. No implicit cast, case folding, or positional column
alignment is introduced.

DataFusion 55's native DataFrame INTERSECT/EXCEPT ALL lowering uses membership
joins, which do not implement duplicate-count subtraction. The compiler instead
numbers occurrences within each complete output tuple and joins on null-safe tuple
equality plus occurrence number. INTERSECT uses a semi join; EXCEPT uses an anti
join. Identical values make the arbitrary within-tuple ordering immaterial. SQL
emission uses the same semantic rule through separately constructed SQL ASTs.

A `compose` node combines independently aggregated facts. Alignment must cover
both complete group-key tuples exactly once. A grouped alignment names an authored
relationship owner, name and business role; slot lineage must match its complete
key pairs. No cardinality declaration is needed to assume uniqueness: each input's
aggregation establishes at most one row per complete key. Unaggregated facts and
partially aligned grains fail binding.

Composition resolves these choices before lowering:

- Group domain: `union`, `intersection`, `left`, or `right`.
- Null grouping keys: `match` or `never_match`.
- Each measure output: input side and slot, output ID and alias, and missing-group
  behavior `null` or numeric `zero`.

Presence markers distinguish a missing group from a present group whose measure
is null. Zero filling changes only the missing group. A union with never-matching
nullable keys can retain two null-key rows; its output is not falsely advertised
as having a unique merged group key for subsequent composition. Global scalar
aggregates use an empty alignment and no relationship reference.

## Integration and evidence

`compile_graph` returns the common compilation record and a sealed `CompiledGraph`
on success. It supports deterministic DataFusion planning, parameterized engine
execution, and engine read-consistency/cache options. HTTP uses the existing
admission and metrics controls at `/v1/compile/graph`; no model is required.

Legacy structured graphs intentionally carry no request-span guarantee.
`GraphIntentQuery` adds a versioned evidence envelope with the exact original
request. Typed targets cover every graph node, row-leaf requirement,
set/composition output, final ordering position and final limit. The compiler
rejects missing, duplicate and orphan targets; validates nonempty ordered UTF-8
spans; and checks node/leaf source text exactly. Scoped record identities use
JSON-Pointer escaping, so adversarial node and requirement IDs cannot collide.
`/v1/compile/graph-intent` accepts `{intent, request_context?}` without a model.
Model proposals can return `status:graph_intent` in the same interpretation call.
Replay retains the exact graph evidence and rejects caller replacement.

The graph execution fixtures pass through both SQL and direct DataFusion paths as
part of 48 typed compiler tests at the latest focused checkpoint. See
`query_graph_sets_preserve_duplicates_nulls_parameter_slots_and_requirement_coverage`
and `fact_composition_aggregates_before_alignment_and_distinguishes_missing_from_null_groups`
and the `graph_intent_*` tests in the typed compiler suite. Model-produced graph
proposals pass scoped context hydration and binding tests.
Generated CTE names avoid catalog objects, and a separate expansion budget rejects
shared graphs whose backend expansion would grow exponentially. Bounded graph capture/replay additionally pins pipeline, catalog, host context and
artifact identity while revalidating the replay caller's scope. Its focused
regression test passes. Graph caching, broader conversions, temporal relationship
alignment, allocation, and release evaluation gates remain outstanding.
