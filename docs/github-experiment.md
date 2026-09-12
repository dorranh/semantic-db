# GitHub experiment findings

These are design findings from the September 11 baseline. Configured CLI loading
and exact issue-state pushdown have since been added; see the
[current connector guide](building-connectors.md) and [example](../examples/github/README.md).


1. **Provider integration works without engine changes.** Ossie's field
   projection preserves execution through the provider. Result equivalence
   through the importer is now tested, beyond successful HTTP requests.
2. **Scope lacks a typed catalog contract.** Required API arguments live in
   connector configuration. The compiler cannot validate requested repositories
   against actual scope or estimate cost. Add inspectable scope/capabilities.
3. **Join knowledge is descriptive.** Relationships, uniqueness, and metric grain
   are not enforced. Duplicate team mappings can inflate counts. Validated
   cardinality and a metric correct across label joins are useful next steps.
4. **Federation does not imply efficient remote execution.** Local predicates
   and join keys do not prune scans. Add exact repository/state pushdown and
   remote field selection with equivalence tests before batched dependent joins.
5. **Execution policy needs query scope.** Provider-local caps cannot coordinate
   multiple scans. Shared deadlines, cancellation, request/point budgets, and
   tracing need a contract above connectors. Request totals currently belong
   to the client lifetime, not individual EXPLAIN operators.
6. **Consistency and provenance need explicit semantics.** Live tables have no
   shared snapshot. Caching will need credential/scope-aware keys, freshness
   policies, and provenance that distinguishes partial from complete results.

These motivate specific contract extensions before a universal GraphQL-to-Ossie
schema translator.
