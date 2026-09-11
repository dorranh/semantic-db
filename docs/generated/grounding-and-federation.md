# Grounding and federation

## Semantic compilation

Consider the request: “Find active deep wells in the northern basin, excluding
records with uncertain locations.”

1. Interpretation preserves the requested entity and unresolved predicates:
   wells, active, deep, northern basin, and uncertain location.
2. Retrieval finds candidate relations, enum definitions, coordinate-quality
   fields, and curated depth conventions.
3. Grounding binds each phrase to a relation, column, value, or operator and keeps
   the evidence for that binding.
4. Missing or conflicting definitions produce a clarification or unsupported
   outcome. The compiler must not invent a depth threshold or quality flag.
5. Only a fully specified interpretation reaches relational compilation.

`semantic-plan` currently models concept predicates with Boolean composition and
explicit grounding outcomes. Temporal relations, projections, aggregates, typed
parameters, and spatial operators require future IR extensions. It is not yet a
complete semantic query language or an executable compiler.

For the supplied fixture, “active,” `North Basin`, and an explicitly selected
depth cutoff can be expressed as:

```sql
SELECT well_id, well_name, total_depth_m
FROM wells
WHERE status = 'active'
  AND basin = 'North Basin'
  AND total_depth_m >= 2500;
```

This returns W-001 and W-004. The fixture has no location-quality field, so it
cannot satisfy the exclusion about uncertain locations. The SQL above represents
only the supported portion, and a future compiler must not present it as the
complete answer to the original request.

## Structured and semantic predicates

Ground concepts into structured predicates wherever the catalog supports an
exact interpretation. For “survey notes describe fractured rock,” a future
semantic-search operator may be needed if the evidence exists only in documents.

Such an operator needs explicit input IDs, document/model/index versions,
similarity metric, score interpretation, filtering behavior, and candidate
limits. Approximate top-k retrieval is not equivalent to evaluating a predicate
over every row. A similarity score is not automatically a probability, and no
default score threshold should be invented during grounding.

No `SEMANTIC_MATCH`, vector backend, or spatial function is registered today.
Unsupported operations should fail clearly. An extension must have declared
semantics and an executable backend before grounded SQL can use it.

For future temporal requests such as “a pressure anomaly followed by a maintenance
event within two hours,” first define partition keys, ordering, time zones, tie
handling, and event multiplicity. Lower to supported relational/window operations
where possible; otherwise implement and test an extension. The initial design
does not assume that illustrative pattern syntax is available in DataFusion.

## Validation boundary

| Check | Current owner/status |
| --- | --- |
| SQL syntax, relation/column resolution, supported functions and type coercion | DataFusion during planning |
| Duplicate relation names and registration consistency | Engine |
| SQL DDL/DML and session statements excluded from query path | Engine via DataFusion SQL options |
| Meaningful joins, grain, enum membership, units, CRS, and definition applicability | Future semantic validator |
| Authorization, query budgets, connector access, and execution policy | Future service/runtime layer |

SQL type compatibility is weaker than domain correctness. Two numeric columns
can use different units; two string identifiers can refer to unrelated entities.
Validation must eventually include those catalog constraints. A compiler repair
loop should receive structured diagnostics, preserve the original intent, and
have bounded retries before asking for clarification.

## Federation target

DataFusion's `TableProvider` is the source integration boundary. It exposes a
schema and builds a physical scan using requested projection, filters, and limits.
Providers declare filter pushdown as exact, inexact, or unsupported. Inexact
filters require residual evaluation; unsupported filters stay local. Follow the
[custom provider contract](https://datafusion.apache.org/library-user-guide/custom-table-providers.html).

Start each connector with correct scans and Arrow conversion. Then add exact
pushdown with equivalence tests. Remote join or aggregate subplans may need
additional logical/physical planning and capability negotiation. DataFusion does
not automatically translate arbitrary API or search requests into remote SQL.

For example, structured well attributes could come from a SQL store, pressure
measurements from an API, and survey-document candidates from a search backend.
Providers return stable IDs and Arrow batches; DataFusion joins them and performs
remaining predicates and aggregations locally.

Before pushing down an operation, account for NULL semantics, collation,
timestamps, numeric precision, pagination, ordering, limits, and backend errors.
Spatial operations additionally need compatible units and coordinate systems.
Never push a candidate limit below an inexact filter if it changes completeness.

Execution eventually needs cancellation, backpressure, memory limits, timeouts,
retry rules, and metrics. Source failures should fail a query unless partial
results are explicitly requested and labeled. The present engine is local and
collects CLI results; its DataFrame API offers the first streaming escape hatch.
