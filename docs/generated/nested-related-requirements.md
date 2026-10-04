# Nested related requirements

Compiler pipeline 38 and row lowering pass 8 implement this contract. Parent
verification passed 355 tests across 80 targets in semantic-plan, semantic-compiler,
semantic-interpreter, semantic-ossie and semantic-eval, followed by all-target
Clippy with warnings denied, formatting and evaluator/CLI builds. Logs are
`/tmp/semantic-eval-nested-{all-tests,clippy,build}.log`. Fresh live acceptance
remains a separate verification step.

`Related.target_requirements` is optional and defaults to an empty list. Its
requirements are conjunctive within the same target occurrence. Only `Filter`,
`ConceptFilter`, and `Related` are accepted. Nested existence and absence follow
an authored edge from their immediate parent occurrence; they preserve source
row multiplicity. This is not a general Boolean existence language.

Every nested requirement has a globally unique ID and exact request evidence.
Every occurrence has a globally unique instance ID. The shared requirement
visitor drives row and graph coverage, structural diagnostic locations, prepared
bindings and span normalization. Depth/work/deadline limits apply before descent;
predicate depth consumes the remaining nested depth budget. Concept expansion
reuses the current target relation and pinned typed argument validation. Target
row policies apply before semi/anti joins, and recursive relation/definition
references remain in execution scope and snapshot validation.

SQL uses correlated EXISTS with compiler-owned aliases independent of public
occurrence names. Regression tests cover root `rhs`, child `src`, and a nested
occurrence resembling a generated alias, preserving exact SQL/direct parity.
Direct plans
recursively filter right inputs with semi/anti joins. Neither path projects a
synthetic field or changes existence into a fanout join. Legacy predicates and
empty target requirements retain their behavior.
