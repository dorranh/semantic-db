# Semantic compiler architecture review

Review date: 2026-09-28. Reviewed the proposed
[architecture](semantic-compiler-architecture.md), the compiler/catalog/plan/engine
implementation at `40e806f`, and primary compiler and text-to-SQL references.
The architecture document is a proposal; the findings below identify contracts
to settle, rather than claiming that its future features already have bugs.
No runtime benchmarks or model evaluations were run for this review.

Status: the findings were incorporated into the architecture on 2026-09-28,
together with a compiler tracing and performance-observation contract. This
review preserves the rationale; the architecture contains the current decisions.

**Recommendation: retain the architecture, and tighten its semantic and scaling
contracts before treating it as the implementation specification.** The separation
of interpretation, binding, semantic lowering, and backend planning is sound. The
largest risk is implementing each box independently while leaving relation scope,
grain analysis, metadata access, and invalidation implicit between them.

## What should stay

- An unresolved Intent IR and a separately validated Bound Query IR. The proposed
  private validation boundary is particularly valuable with LLM-produced input.
- Authoritative catalog objects with provenance, immutable revisions, and
  rebuildable search projections.
- Deterministic expansion of governed metrics and relationships, with DataFusion
  owning physical planning and optimization.
- Explicit unknowns, negative qualifications, ambiguity, unsupported scope, and
  parameter-dependent applicability.
- The distinction between declared dependency completeness and actual semantic
  sufficiency. The document correctly avoids treating retrieval or a requirement
  ledger as proof that natural language was fully understood.
- Separate evaluation of interpretation, retrieval, lowering, and backend
  behavior, including a human-curated sufficient-context comparator.

These choices fit established compiler practice. MLIR provides precedents for
progressive lowering and explicit conversion legality; Calcite provides precedent
for separating query languages, relational representations, optimization, and
adapters. These are architectural references, not recommendations to introduce
either framework into this Rust project.
[MLIR conversion](https://mlir.llvm.org/docs/DialectConversion/),
[Calcite paper](https://arxiv.org/abs/1802.10233).

## Prioritized findings

P1 means a foundational decision to settle before freezing the affected contract.
P2 means a required refinement before enabling the corresponding capability.

| Finding | Priority | Design sections | Decision needed |
| --- | --- | --- | --- |
| R1. Make catalog access and provider resolution demand-driven | P1 | 5, 13, 17 | Separate catalog publication from request-local hydration and executable provider creation. |
| R2. Make wide-schema retrieval and graph expansion bounded from the first retrieved mode | P1 | 6, 11, 17 | Define field discovery, expansion limits, and incomplete-search outcomes. |
| R3. Give every relation use its own identity and scope | P1 | 7, 8.2, 9.1 | Distinguish catalog definitions from their occurrences in a query. |
| R4. Define grain analysis and semantic planning before the validated boundary | P1 | 8.4, 9.2 | Specify which decisions binding settles and which transformations lowering may perform. |
| R5. Track lookup dependencies as well as selected objects | P2 | 5, 14 | Make incremental reuse correct when names, alternatives, or search results change. |
| R6. Specify pass verification, analysis invalidation, and work limits | P2 | 9, 11 | Keep derived properties valid and avoid unbounded deterministic compilation. |
| R7. Include comparison semantics in the portable contract | P2 | 8.3, 9.1, 10 | Make equality/grouping/ordering semantics explicit across engines. |
| R8. Define value grounding and a compact model proposal protocol | P2 | 6, 7, 8, 13 | Distinguish user literals, catalog value mappings, and unsupported inference. |

### R1. Prompt selection must also eliminate catalog-wide host work

The lifecycle and cache design are sensible, but they do not establish a request
cost model or say which catalog operations must be lazy. Reducing model context
can leave compilation proportional to the entire catalog on the host.

The current implementation shows why this needs an explicit migration contract:

- `Compiler::compile_internal` constructs JSON for every relation and column
  ([compiler](../../crates/semantic-compiler/src/lib.rs), line 71).
- `validate_outcome` rebuilds a set of every relation and field reference for
  each grounded proposal, including repair attempts (line 165).
- `Engine::from_catalog` resolves every base provider and plans every view while
  loading the catalog ([engine](../../crates/semantic-engine/src/lib.rs), line
  169). This is catalog-load work, not evidence that it runs for every request.
- `registration_order` repeatedly scans pending definitions (line 340). A
  dependency chain in reverse lexical order causes quadratic candidate visits.

**Recommended contract:** a request pins a cheap immutable snapshot handle, resolves
object/field IDs through indexes, hydrates selected definitions, and creates or
reuses providers only for the query's execution dependency closure. Snapshot
publication validates definitions against recorded schema/capability contracts;
live provider construction is a separate operation. Metadata refreshes can run
in the catalog lifecycle without forcing every compile to contact every source.

Use shared immutable records and structural sharing between snapshots. Keep
large source archives and prose out of frequently traversed indexes. Do not copy
the entire manifest, hash all definitions, expand all views, or reconstruct all
field-reference strings merely to validate a small query. Use an indegree queue
and typed dependency edges for topological ordering; a deterministic ready set
can preserve stable output with logarithmic scheduling overhead.

DataFusion exposes custom catalog/schema providers and asynchronous table lookup,
which offers an appropriate adapter boundary. The async lookup exists in the
locally installed, repository-pinned DataFusion 55.0.0 interfaces as well.
[DataFusion catalogs](https://datafusion.apache.org/library-user-guide/catalogs.html).

The scaling target should be expressed as:

```text
initial catalog/index build: proportional to catalog bytes and dependency edges
incremental publication:     changed inputs + affected dependency closure
request work:                search work + hydrated slice + query plan work
provider resolution:         selected execution dependencies + cache misses
```

This is a target for work accounting, not a universal complexity bound. Common
search terms can have large postings, and a change to a shared definition can
legitimately affect most of the catalog. Instrument those costs explicitly.

### R2. Move wide-table selection and bounded graph discovery into stage C

Section 6 allows a compact field inventory for wide relations, while stage G
defers wide-table pruning. A compact inventory still grows with every field, and
mandatory context can grow with a highly connected dependency graph. A single
very wide relation can therefore exhaust the budget even when the question needs
three fields.

**Recommended contract:** retrieve through several independently searchable levels:
domains, relations/concepts, and fields with parent references. Start with a
small relation contract and selected field details; supply more inventory through
bounded lookup or pagination. Global field search must remain available so that
a field-only clue can discover a relation missed by relation-level ranking.

Publish explicit typed dependencies for mandatory interpretation facts. Hydrating
one field must also hydrate its governing qualifications, applicable model rules,
and the relevant relationship contract. Keep execution dependencies out of model
context when an adequate authored view/metric contract already encapsulates them,
as section 6.2 correctly permits.

Specify separate limits for candidate count, hydrated bytes/tokens, visited graph
nodes/edges, relationship-path candidates, and expansion rounds. Broadening a
search must expose whether exploration was exhausted or cut short. A traversal
limit cannot establish that a relationship does not exist. Do not recursively
enumerate every simple path or every alternative of every alternative.

Section 6's fallback policy is useful, but returning context-limit for a normal
three-field query over a wide table should be a failed scale acceptance case,
not the intended operating behavior. Truly irreducible mandatory context may
still require clarification, a narrower request, or an unresolved outcome.

The literature supports evaluating the tradeoff rather than prescribing one
selection algorithm: *The Death of Schema Linking?* shows the cost of pruning
needed schema where full context fits; *Extractive Schema Linking* explicitly
studies selection's precision/recall tradeoff. Neither establishes that full
context is practical for an arbitrarily large enterprise catalog.
[Schema-linking comparator](https://arxiv.org/abs/2408.07702),
[extractive selection](https://arxiv.org/abs/2501.17174).

### R3. Catalog identity is insufficient for query name resolution

`ObjectRef` identifies a catalog definition. The Bound Query sketch has relation
references and relationship paths, but does not specify an identity for each use
of that definition. Relational node/field IDs arrive later, in section 9.1.

Consider “show billing-customer region and shipping-customer region.” Both fields
refer to the same catalog definition, `customers.region`, but belong to different
relation occurrences. The same issue appears with employees and managers,
self-joins, repeated parameterized views, and later nested queries.

**Recommended contract:** introduce occurrence identity during binding:

```text
RelationInstance { instance_id, definition: ObjectRef, scope_id }
BoundFieldRef    { instance_id, field_id }
BoundJoin       { left_instance, right_instance, relationship_ref, role }
OutputSlot      { slot_id, expression, display_name }
```

Aliases and display names are presentation; instance IDs and output slots carry
meaning. Specify lexical scope, outer references for supported nested queries,
duplicate output names, and resolution precedence. Unsupported correlation must
fail explicitly. Require relationship endpoint references to name occurrences,
not only catalog entities.

For the first implementation, correlation can remain unsupported. A query with
two roles over one relation must nevertheless be representable without ambiguity.
Add fixtures for billing/shipping, employee/manager, and two instances of the same
parameterized view before freezing Bound Query serialization.

### R4. Make grain analysis executable and settle ownership of semantic planning

Section 8 promises that accepted bound queries pass grain and applicability
checks, while section 9.2 still asks lowering to plan semantic joins, metric grain,
and row preservation. This leaves an important ownership question: has the bound
artifact already selected the complete business meaning, or can lowering still
choose an interpretation?

The distinction matters for two fact tables. Aggregating both to “month, region”
does not decide whether unmatched groups survive, how null grouping keys align,
whether a filter affects one metric or both, or whether region means current
region versus event-time region. These are observable semantics.

**Recommended contract:** make semantic analysis/planning an internal part of
binding before constructing the validated wrapper. It resolves relationship
roles, time roles, filter scopes, requested grain, group-domain behavior,
allocation, and missing-group behavior. It can share analyses with lowering;
it does not need a fifth public IR or another model call.

Represent the supporting analyses explicitly:

- A grain consists of scoped entity/dimension identities and temporal grain.
- Keys and functional dependencies carry evidence and scope, independently of
  the authored row meaning.
- Join multiplicity is directional and accounts for nullable keys and unmatched
  rows.
- Metric aggregation describes its permitted merge/finalize behavior where
  relevant, such as average represented by sum and count, or a distinct count
  that cannot be added across overlapping groups.
- Metric composition declares its group domain, alignment, and empty/null/zero
  behavior.

Lowering may choose among proven equivalent relational implementations. It must
not discover a business choice and silently settle it. A resource/capability
failure can still happen after binding and should be reported as such.

For a simple additive metric over a many-to-one dimension join, write down the
actual rule: the chosen predicate must match at most one dimension row per fact
row under the relevant comparison semantics and evidence scope; row retention
must match the metric contract. Pre-aggregation alone is not a general repair for
fan-out. An order total grouped by item category still requires an explicit
allocation or different measure, even if there is one pre-aggregated row per order.

MetricFlow is useful precedent for entity-driven join rules that avoid fan-out
and chasm joins. Its supported rule set is a reference to examine, not evidence
that arbitrary joins or warehouse declarations are safe.
[MetricFlow joins](https://docs.getdbt.com/docs/build/join-logic).

Decide the initial acceptance profile before shipping metrics: strict verified
constraints, specified authored assumptions with disclosure, or both as distinct
modes. This affects usability on sources without enforced keys and the possible
cost of runtime obligations; it should not remain an implicit implementation default.

### R5. Incremental reuse needs dependencies on lookup results and absence

Section 14 correctly identifies the new-competing-definition problem. Reverse
edges from selected objects alone cannot implement the promised invalidation.

Suppose `lookup("revenue", sales_scope)` returned one definition. A new matching
definition changes that lookup without modifying the selected object. Negative
lookups have the same problem: a previously missing concept can appear later.
Inherited policy and namespace visibility changes can also change the result.

**Recommended contract:** make scoped lookup results tracked inputs, for example
`ResolveName(scope, token)`, `Alternatives(concept, scope)`, and
`EffectiveAnnotations(object, scope)`. Cache dependencies include their revisions,
including empty results. For ranked/vector retrieval, pin the index generation
and retrieval configuration; where narrower invalidation cannot be shown sound,
use a conservative index or namespace revision.

Distinguish three reuse questions: replaying an explicitly pinned artifact,
re-lowering a still-valid bound query, and answering the same natural-language
request against the latest catalog. Adding a competing metric may invalidate the
last operation while leaving the first two meaningful under their explicit
bindings. Otherwise the cache either serves stale interpretations or loses all
reuse on unrelated edits.

Salsa's tracked-query dependency model and unchanged-output backdating are good
references for incremental analysis. An explicit dependency cache may be enough
initially; adopting Salsa itself should follow a small prototype.
[Salsa algorithm](https://salsa-rs.github.io/salsa/reference/algorithm.html).

### R6. Specify a small pass framework, including deterministic work budgets

Sections 9.2–9.3 list sensible transformations and preconditions. Add a contract
for how derived type, grain, key, lineage, and policy analyses remain valid after
each transformation. An outer join, for example, changes nullability and can
invalidate properties that were true of its inputs.

Use immutable pass outputs or explicit analysis invalidation; do not leave mutable
metadata attached to rewritten nodes without an owner. Validate IR structure and
types at trust/stage boundaries and after each pass in tests/debugging. Re-run
affected semantic analyses where a pass changes their inputs. Checks should include
reference scope, output slots, aggregate/window placement, requirement mappings,
and target legality. A failed pass must not yield a validated artifact.

MLIR's pass infrastructure separates analyses from transformations and requires
preservation to be declared; its conversion framework distinguishes successful
full legalization from partial conversion. These are useful patterns to implement
with ordinary Rust enums and functions, without a general plugin framework.
[Pass/analysis management](https://mlir.llvm.org/docs/PassManagement/),
[conversion legality](https://mlir.llvm.org/docs/DialectConversion/).

The controller already budgets model calls, tokens, time, and cancellation. Extend
that to expression depth, IR nodes, expanded definitions, relationship search,
metadata requests, and emitted SQL size. Acyclic definitions can still expand
exponentially when shared dependencies are copied repeatedly. Preserve a DAG or
memoized expansion keyed by definition, arguments, and scope. Share immutable
structure without changing evaluation count for volatile expressions. Keep
provenance in referenced records rather than copying full ancestry into every node.

At the service boundary, add bounded concurrency, cache memory limits, snapshot
retention, and cancellation checkpoints in CPU-bound loops. Token budgets alone
do not constrain compilation memory or host work.

### R7. Equality and grouping need a semantic profile too

The proposed type/function contracts cover many important differences, including
nulls, decimals, calendars, and volatility. The relational expression sketch still
leaves comparison semantics implicit. Collation affects equality as well as
sorting, and therefore joins, grouping, distinct counts, and uniqueness evidence.
The existing [federation note](grounding-and-federation.md) already mentions
collation; the compiler contract should connect to that requirement explicitly.

For example, grouping `"A"` and `"a"` under a case-insensitive comparison can produce
one group where a byte-sensitive comparison produces two. PostgreSQL documents
nondeterministic collations that can consider different byte sequences equal.
[PostgreSQL collation semantics](https://www.postgresql.org/docs/current/collation.html).

Define a versioned comparison profile, globally or on affected expressions, with
overrides for collation, case/Unicode behavior, null-safe equality, and supported
floating-point special-value behavior. Join key evidence must be valid under the
comparison actually used. Backend capability checks must establish compatibility
or reject the operation. Local fallback is valid only when local execution also
implements the accepted semantics.

Keep optimizer statistics separate from semantic constraint evidence. An estimated
distinct count or observed uniqueness can guide a physical strategy; it cannot
justify changing query meaning. Require differential fixtures for each supported
connector's exact pushdown claims.

### R8. Define value grounding and keep the model proposal smaller than the IR

Schema grounding does not settle data-value grounding. “UK customers” might map
to a country label, a governed country code, or a market definition. A well-typed
string comparison can still select the wrong value.

Specify three cases: an explicit user literal; a catalog-backed enum/code/alias
mapping; and an unresolved value phrase. Carry the mapping's identity and revision
into the bound parameter. Do not require proof that every explicit literal occurs
in current data: a correct query may return no rows. Conversely, do not translate
a business phrase into an internal code without supporting evidence.

This fits the no-query-execution invariant using authored dictionaries and value
contracts. If live value lookup is later desired, define a separate authorized,
budgeted I/O interface with freshness and provenance; do not introduce hidden
profiling during compile. CHESS is relevant because it treats database-value
retrieval as a distinct part of the text-to-SQL problem.
[CHESS](https://arxiv.org/abs/2405.16755).

Give the LLM a deliberately smaller response schema than the internal Bound Query
IR: intent, candidate references/roles, literal proposals, and requested context
expansion. The host should fill in object revisions, effective policies, inferred
types, output schema, and deterministic provenance. Do not ask the model to copy
the catalog's complete metric definitions or invent a certificate of validity.

Use a stable bounded schema, request-local handles alongside readable names, and
host validation against the pinned catalog. Avoid enumerating all catalog IDs or
all possible field combinations in the output schema. Constrained decoding can
reduce malformed proposals; PICARD provides evidence for parser-constrained
generation, not a guarantee of correct business interpretation.
[PICARD](https://aclanthology.org/2021.emnlp-main.779/).

## Recommended implementation order and scope

Keep the four conceptual representations. Treat Catalog IR as the shared symbol
and semantic database used by the pipeline. Initially make the Relational IR a
small internal, serializable, version-tagged representation; do not promise a
long-lived public relational wire format before a second independent consumer
needs one. Intent and accepted semantic queries are the more valuable public
contracts to stabilize. Preserve replay using explicit compiler versions.

The main roadmap adjustment is to exercise a complete deterministic slice earlier,
and move basic scaling mechanisms out of the final optimization stage.

| Milestone | Concrete scope and exit condition |
| --- | --- |
| 1. Baselines and catalog access | Capture current failures/costs; add synthetic catalog workloads and mock-model host measurements. Define stable IDs, snapshot handles, indexed field lookup, and cold/warm cost accounting. |
| 2. Small deterministic vertical slice | A structured client submits projections, filters, ordering, and limits over base relations and authored views. Bind with scoped instances, lower to a minimal relational core, and execute through DataFusion with no LLM dependency. |
| 3. LLM interpretation and scalable retrieval | Feed the same binder from a compact proposal protocol. Add exact/lexical relation and field retrieval, inherited facts, alternative lookup, bounded expansion, and wide-table handling. Compare with full compact context where feasible. |
| 4. One metric and relationship slice | Implement additive measures over many-to-one relationships, including nulls and unmatched rows. Establish grain/constraint transfer rules, acceptance profiles, and a fan-out rejection fixture. |
| 5. Composition and advanced semantics | Add multi-fact composition, ratios, temporal rules, windows, and selected conversions only with explicit semantics and adversarial fixtures. |
| 6. Measured optimization and additional backends | Embeddings/reranking, finer cache reuse, more emitters, and interchange follow measured demand. Basic incremental storage and field selection already exist. |

Milestones can overlap, but large-scale retrieval should not become coupled to a
temporary model-to-SQL contract. Keep the existing compatibility mode explicitly
labeled, as the proposal already requires.

Avoid a custom cost-based optimizer, a distributed graph service, a universal
ontology, or a general rewrite plugin system in the first version. Keep the
DataFusion adapter thin and the supported relational subset small. Extensibility
comes first from explicit contracts and exhaustive matches, not infrastructure.

## Scale and correctness acceptance plan

Section 16 has the right evaluation dimensions. Add a concrete workload matrix
and host-side counters so that an inexpensive model call cannot hide expensive
catalog work. The following are proposed test sizes, not measured capacity claims:

| Axis | Suggested cases |
| --- | --- |
| Total schema size | 100, 1,000, and 10,000 relations; extend to at least 1,000,000 fields overall. |
| Width | A few fields through a synthetic 10,000-field relation, including a query needing only three fields. |
| Graph shape | Chains, shared dependency diamonds, dense relationship hubs, disconnected domains, and repeated self-roles. |
| Metadata volume | Short definitions, extensive scoped prose, many aliases, conflicting definitions, and large value dictionaries. |
| Change pattern | One-field edits, comment-only changes, shared policy edits, additions/removals of synonyms and alternatives, and source-schema drift. |
| Service load | Cold/warm requests, concurrent readers during publication, slow metadata sources, cancellation, and bounded-cache pressure. |

Measure total and stage p50/p95/p99 latency, allocations/peak memory, objects and
fields visited, graph edges traversed, provider resolutions, metadata I/O, generated
plan size, all model tokens, and update/invalidation work. Separate queue time,
provider latency, and deterministic compilation. Initially choose numerical SLOs
from the deployment envelope and observed workloads; do not present arbitrary
latency targets as established feasibility.

Use these architectural acceptance properties alongside throughput and latency:

1. Growing unrelated catalog content does not cause a simple selected query to
   enumerate or serialize that content during binding/lowering.
2. A single-field update recomputes its real dependents without rebuilding all
   providers; shared-definition updates may legitimately have wider impact.
3. A new competing definition invalidates latest-catalog interpretation reuse,
   while an unrelated edit preserves eligible bound-plan reuse.
4. A wide-table query succeeds with bounded context when its required facts fit.
5. Exceeding graph or compilation budgets gives a precise bounded failure, never
   a claim that missing data or an unavailable join was proved absent.
6. Concurrent publication cannot mix object/index revisions within one request.

Add semantic fixtures for repeated relation roles; filters before versus after
outer joins; duplicate and nullable dimension keys; null group alignment in
multi-fact composition; missing months; ratio denominators; and collation-sensitive
joins. Use small distinguishing datasets, property tests, and mutation tests that
remove a required predicate or switch a relationship role. Verify both the direct
DataFusion path and emitted-SQL path where supported, while keeping independently
specified expected results so shared lowering bugs cannot pass by agreement.

For LLM evaluations, hold out domains and phrasing, measure incorrect acceptance
alongside useful coverage and clarification rates, and inject missing facts,
misleading aliases, reordered context, and catalog-text prompt injection. A system
that refuses every difficult query has low incorrect acceptance but is not useful.
Spider 2.0 is a relevant source of enterprise-shaped tasks: its databases often
exceed 1,000 columns and require broader metadata understanding. Supplement it
with this project's governed metric and ambiguity cases; benchmark text-to-SQL
success is not a substitute for semantic-layer correctness.
[Spider 2.0](https://arxiv.org/abs/2411.07763).

The next design revision should make R1–R4 explicit, record the initial constraint
acceptance profile, and define the first vertical slice and scale workloads. That
would turn an already sound architectural direction into a practical foundation
with testable boundaries.
