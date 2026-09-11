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

## Next increments

| Increment | Deliverable | Acceptance criteria |
| --- | --- | --- |
| Catalog authoring and persistence | Qualified IDs, metadata editing, versioned definitions and relationships | Restart preserves views; schema drift and dependency cycles fail with useful diagnostics |
| Deterministic grounding | Structured lookup and a small curated concept registry | A concept maps to a checked predicate with evidence; ambiguity and missing units require clarification |
| Retrieval | Hybrid lexical/semantic search over catalog projections | Results retain stable IDs and revisions; access scope is applied before results reach the compiler |
| Intent compiler, next stage | Richer typed IR and deterministic lowering beyond the delivered LLM adapter | Expand live evaluations and validate semantic coverage, units, grain, and concept definitions |
| First remote provider | One backend with Arrow scans and explicit capability reporting | Results match a local reference across NULLs, pagination, errors, and supported pushdown |
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
- First external connector and whether an existing federation extension fits it.
- Materialization freshness rules and source snapshot guarantees.
- Project license and any future public API stability policy.

None of these decisions prevents developing and testing the current local SQL
and relation-registration foundation.
