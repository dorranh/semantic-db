---
name: semantic-db-optimize
description: "Investigate Semantic DB federation performance using plans and measurements, then improve connector or query behavior without changing results."
---

# Optimize a Semantic DB query

Start from a concrete query, configured sources, expected result, and observed latency or transfer. Inspect the executable source capabilities and use EXPLAIN / CLI `--dry-run`. Dry-run validates/plans but is not a remote-cost estimate. Avoid running unbounded production scans to establish a baseline.

Distinguish local execution from source pushdown. PostgreSQL currently pushes projection and limits, whereas ClickHouse supports a qualified broader SQL subset. Parquet can prune columns and skip data using metadata; CSV/JSON generally scan. A cross-source join working correctly does not mean it executes remotely. Inspect actual plans and request/row/byte counters when available.

Check filters, projected columns, join cardinality, aggregation location, pagination, supported type conversions, and authored-view expansion. Attribute the bottleneck using measurements before changing code. For files, consider an appropriate physical layout such as Parquet; keep business meaning in Ossie/views.

Propose the smallest measured improvement. Preserve nulls, duplicate rows, filter/limit ordering and type semantics. Validate against an unoptimized baseline with independent expected results. Include cases where an operation must stay local. Do not label a faster incorrect query an optimization.

If runtime contracts include transaction bindings, consistency options, snapshots or reconciliation, preserve them and account for their costs explicitly. Do not remove correctness checks or add caching that violates freshness to improve a benchmark.

Report before/after latency and transfer, tested equivalence, remaining limits, and upstream changes that would help. Source/config changes do not authorize changes to an external service.
