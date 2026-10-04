# Shared model context

Implemented and verified: 118 interpreter tests passed, all-target Clippy passed,
and evaluator/CLI builds passed. Full live acceptance runs are in progress; live
provider token measurements remain pending.

Context payload version 2 retains model-level description and AI context once in a
`shared_model_contexts` map. Each selected relation references its exact model context
through `semantics.model_context_ref`. Dataset, field, relationship and governance
facts remain scoped to their original relation and keep their existing provenance.

Shared identities contain the pinned document SHA and model name. A content revision
covers the complete authored description and AI context. Equal hints from different
models do not merge. Catalog entries without imported model provenance use a
relation-scoped identity, avoiding an invented shared model identity.

The renderer includes entries only for selected authorized relations. Auditing resolves
each reference to the pinned model definition and rejects stale, missing, mismatched
or orphan shared entries. Cache renderer and selection revisions change with the new
payload, and the model protocol explains how references apply to relations. No
instructions, examples, synonyms or knowledge are truncated or selected using golds.

The regression fixture compares the referenced payload with an equivalently audited
legacy inline payload and verifies a reduction exceeding 100,000 UTF-8 JSON bytes.
This measures serialized context bytes, not provider token usage.
