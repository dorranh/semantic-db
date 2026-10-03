# Compiler catalog scale measurements

Measured locally on 2026-09-28 with the release-mode `catalog_scale` example.
These are single-run host-cost measurements, not end-to-end model quality or
production latency claims. No provider, network, database or row reads occurred.

Reproduce with:

```sh
cargo run --release -p semantic-catalog --example catalog_scale --offline -- 1000 100
cargo run --release -p semantic-catalog --example catalog_scale --offline -- 10000 100
```

Each relation has 100 Int64 fields. The selective search uses one rare field
alias. The update adds one alias to one field on one relation, followed by index
refresh. Shared common-name postings remain intact when their contents do not
change. Timings use `Instant`; construction counters are shared with compiler
context selection.

| Work | 1,000 relations / 100,000 fields | 10,000 relations / 1,000,000 fields |
|---|---:|---:|
| Register and hash catalog | 278.133 ms | 2,473.443 ms |
| Initial graph validation | 0.359 ms | 3.335 ms |
| Cold search index | 1,849.184 ms | 20,776.720 ms |
| Selective search | 0.010 ms | 0.011 ms |
| Atomic alias publication | 0.283 ms | 0.298 ms |
| Incremental search update | 2.662 ms | 2.805 ms |
| Cold objects visited | 101,000 | 1,010,000 |
| Cold source bytes charged | 3,733,901 | 37,348,901 |
| Update objects visited | 102 | 102 |
| Update bytes charged | 12,308 | 12,308 |
| Publication objects revalidated | 1 | 1 |
| Search postings visited | 3 | 3 |

The million-field process completed in about 23.58 seconds total. `/usr/bin/time`
could not collect host resource statistics because `sysctl kern.clockrate` was
denied by the sandbox; peak RSS is therefore unmeasured. Do not infer a memory
capacity guarantee from successful completion. Later debug test linking also
exhausted local disk space; obsolete test executables from this work were removed
before retrying. This is separate from index runtime memory.

These fixtures establish useful architectural evidence: a warm metadata update
and selective lookup do not revisit a million fields, while initial indexing is
still substantial work. They do not justify enabling retrieved mode by default.
That gate still needs repeated held-out sufficient-context/interpretation tests,
incorrect-acceptance measurements, end-to-end cost comparisons, and latency
percentiles. Full compact context remains the default typed selection policy.
