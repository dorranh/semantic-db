# Implementation roadmap

## Foundation delivered

- Cargo workspace with libraries under `crates/` and the CLI under `apps/`.
- Shared package settings, dependency versions, lints, pinned toolchain, and CI.
- In-memory catalog with Arrow schemas and base/view metadata.
- Serializable semantic intent and grounding outcome types.
- DataFusion SQL planning and execution over registered CSV sources.
- Composable in-memory views with inferred schemas and direct lineage.
- Interactive SQL, one-statement file/stdin/query modes, and catalog commands.
- Synthetic geospatial fixture and integration tests for queries, nested views,
  subquery lineage, and failed registration/query handling.

## First LLM slice delivered

- Pluggable model-provider trait and OpenAI-compatible Chat Completions adapter.
- Catalog projection with schema and view definitions, excluding base source paths
  and row samples.
- Strict grounding outcome decoding, evidence-reference checks, generated-SQL
  planning over registered relations, and bounded validation repair.
- Batch `--ask`/`--dry-run`, interactive `.ask`/`.plan`, and lazy `.env` configuration.
- Offline compiler/HTTP/CLI tests and an opt-in live semantic evaluation for exact,
  ambiguous, and unsupported requests.

This starts the intent-compiler increment with combined interpretation and SQL
proposal. Rich typed IR lowering, curated semantic enforcement, and large-catalog
retrieval are still outstanding.

## Embedding slice delivered

- Single `semantic-db` facade with optional natural-language compiler.
- Authored relation metadata and iterator-based ingestion from a team's catalog.
- Async relation backend using DataFusion's standard `TableProvider` contract.
- Direct provider registration with schema checks; dependency-ordered catalog
  loading with missing-reference, cycle, and schema-drift diagnostics.
- Runnable custom backend example and offline consumer integration tests.
- [Prior-art comparison](../catalog-prior-art.md), identifying Apache Ossie/OSI as
  a semantic interchange candidate to evaluate before choosing a durable format.

## Ossie importer delivered

- Optional `semantic-ossie` library, exposed by the facade's `ossie` feature.
- Offline pinned JSON Schema validation and explicit source-to-provider bindings.
- Identity-field projections with logical/physical type checks and hidden raw columns.
- Model/field annotations and declared keys preserved separately from Arrow schemas
  and supplied to the compiler; keys are explicitly not enforced.
- CLI `--ossie`, `--ossie-model`, and `--source-csv` support.
- Wells query equivalence, compiler metadata, rejection, and CLI integration tests.

See the [Ossie reference](../ossie-reference.md) for the supported profile.

## GitHub remote experiment delivered

- Optional `semantic-github` providers for repository-scoped issues and labels.
- Live page streams, independent nested cursors, and explicit per-scan budgets.
- Ossie model, SQL/natural-language example, and local CSV ownership joins.
- Remote/local result equivalence tests and HTTP/error/cancellation probes.

See the [experiment and findings](../../examples/github/README.md). Other filters and
joins remain local; typed source capabilities, efficient pushdown, shared query
budgets, and snapshot semantics are still open.

## Dataset and connector onboarding delivered

- Shared `semantic-sources` project loader and extensible factory/connection registry.
- CSV and GitHub use the same configured CLI and embedded loading path.
- Offline model/config validation and inspection; optional connected schema checks.
- Single-column Ossie aliases, including exact quoted physical names.
- Reusable CLI host, runnable paginated connector template, and result-conformance helper.
- Exact GitHub issue-state equality pushdown, compared with local/reference scans.
- Task-oriented [dataset](../adding-datasets.md) and [connector](../building-connectors.md) guides.

## Authored-view grounding slice delivered

- `Compiler::compile_views` and CLI/REPL view-query modes select existing authored
  views without introducing a separate concept registry.
- Typed selections lower to SQL with a fixed view source, checked columns,
  request-literal comparisons, null tests and ordering.
- Binding evidence comes from the applied view definition; selected intent remains
  available in `Compilation.view_selection`.
- Offline nested-view equivalence, invalid binding/literal, lowering and CLI tests,
  plus a runnable Ossie-backed Rust example.

This preserves the selected definition deterministically. Definition selection,
natural-language coverage and units remain interpretation/validation gaps; see
[the contract](authored-view-grounding.md).

## Next increments

| Increment | Deliverable | Acceptance criteria |
| --- | --- | --- |
| Catalog interoperability expansion | Selected computed expressions, relationships, and metrics guided by a real team model | Correct projections and aggregation across joins; unsupported semantics retain explicit diagnostics |
| Catalog persistence and identity | Qualified IDs, metadata editing, versioned definitions and relationships | Restart preserves definitions and revisions; source identity survives interchange and execution projections |
| Deterministic grounding | Extend checked authored definitions using real team requests; evaluate whether any separate concept representation is needed | Validate definition applicability and requested-constraint coverage; ambiguity and missing units require clarification |
| Retrieval | Hybrid lexical/semantic search over catalog projections | Results retain stable IDs and revisions; access scope is applied before results reach the compiler |
| Intent compiler, next stage | Richer typed IR and deterministic lowering beyond the delivered LLM adapter | Expand live evaluations and validate semantic coverage, units, grain, and concept definitions |
| Remote provider optimization | Typed scope/capabilities, repository pruning and remote field selection beyond state equality | Optimized results match the baseline across NULLs, pagination, residual filters, and limits; request counts improve |
| Semantic operators | One precisely defined search/table-function integration | Candidate limits and approximation are visible; model/index versions are recorded |
| Durable derived relations | Definition revisioning, dependency invalidation, optional materialization | Views compose after reload; stale materializations cannot silently satisfy a query |
| Service execution | Streaming, cancellation, resource budgets, auth, and observability | Resource limits and cancellation propagate to connectors; query provenance is inspectable |

Build a vertical slice through one increment at a time. Defer multiple backends
and custom optimizer nodes until a real query demonstrates why they are needed.

## Verification

Run the formatting, Clippy, and workspace test commands in the root README. The
integration fixture should return exactly W-001 and W-004 for the northern deep
well query. Test view composition and lineage alongside execution so metadata
does not become disconnected from the plan it describes.

As features arrive, add semantic evaluation cases separate from deterministic
engine tests. Connector tests should compare remote pushdown with local residual
execution over the same data. Query-plan snapshots can help diagnose changes,
but avoid using unstable plan formatting as the sole correctness assertion.

## Open design decisions

- Persistent catalog format and transaction strategy.
- Namespace, ownership, authorization, and catalog revision model.
- Exact types for parameters, units, relationships, temporal and spatial intent.
- Retrieval provider, broader semantic evaluation data, and token/cost budgets.
- Scope/capability contracts and whether an existing federation extension fits API joins.
- Materialization freshness rules and source snapshot guarantees.
- Project license and any future public API stability policy.

None of these decisions prevents developing and testing the current local SQL
and relation-registration foundation.
