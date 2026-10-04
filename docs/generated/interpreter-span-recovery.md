# Exact model offset recovery

The recovery path is implemented and its unit/integration tests pass. Live-provider
acceptance verification remains separate.

The interpreter may correct offsets in model-authored intent evidence when the existing
requirement source text occurs exactly once, byte for byte, as a contiguous substring
of the original host request. The source text, operations, identities, alternatives and
original response remain unchanged. Valid spans, including discontinuous spans joined
by a single space, stay unchanged. Uniqueness includes overlapping occurrences.

Host evidence disables recovery. Missing mappings, empty span vectors, duplicate targets, unknown targets,
paraphrases, repeated phrases, empty source text and graph output/order/limit entries
are left to the strict compiler validator. The correction never establishes semantic
completeness and introduces no model retry.

Normalization records contain model attempt numbers, numeric requirement and evidence
positions, and previous/replacement byte ranges. They exclude request text and authored
identities. Entry traversal and byte-comparison work have finite budgets and check the
existing cancellation token and deadline. The original response and token/call accounting
retain their existing behavior.

Compiler preflight diagnostics are preserved when validation fails before binding
a snapshot and the current catalog is unchanged. Successful compilations still
require the recorded pinned snapshot; changed or explicitly mismatched snapshots
remain failures.
