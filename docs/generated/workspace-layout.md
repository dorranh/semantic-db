# Workspace layout

- `crates/`: reusable libraries.
- `apps/`: application frontends.
- `examples/`: standalone example packages alongside their configuration and data fixtures.
- `tests/performance/`: performance workloads, separate from correctness tests.
- `tests/support/`: Docker fixtures shared by integration and performance tests.

## Examples

Run examples from the repository root:

```sh
cargo run -p example-team-catalog --locked
cargo run -p example-ossie-wells --locked
cargo run -p example-authored-views --locked
cargo run -p example-github --locked
cargo run -p example-custom-connector --locked -- --config examples/connectors/semantic-db.yaml --query "SELECT id FROM items WHERE id >= 4 LIMIT 1"
```

The GitHub example requires `GITHUB_TOKEN`. The other commands above run offline.
Each package declares the library features it needs. Connector template tests now
run automatically with `cargo test --workspace --all-features --locked`.

## Performance workloads

```sh
# Docker-backed controlled benchmark; writes the existing generated JSON report.
just test-performance

# Public endpoint smoke benchmark; prints JSON to stdout.
just benchmark-public
```

The underlying commands are:

```sh
cargo test -p semantic-performance --release --locked --test clickhouse_controlled -- --ignored --nocapture
SEMANTIC_PUBLIC_CLICKHOUSE=1 cargo run -p semantic-performance --release --locked --bin clickhouse_public
```

Normal workspace tests compile the workloads but do not run either benchmark.
The controlled benchmark is ignored unless explicitly selected. The public
benchmark retains its environment opt-in and request/resource limits. The
controlled report remains at `docs/generated/clickhouse-controlled-benchmark.json`;
public output can be saved under `docs/generated/` when recording a new run.
`CLICKHOUSE_TEST_TAG` selects the Docker image version for both integration tests
and the controlled benchmark.
