# Format the Rust workspace.
format:
    cargo fmt --all

# Lint every target and feature, treating warnings as errors.
lint:
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Type-check every target and feature without building executables.
check:
    cargo check --workspace --all-targets --all-features --locked

# Run offline workspace tests, including the example packages.
test:
    cargo test --workspace --all-features --locked

# Run connector integration tests
test-connectors:
    cargo test --workspace --all-features --locked connector_integration -- --ignored

# Run controlled performance workloads (requires Docker).
test-performance:
    cargo test -p semantic-performance --release --locked --test clickhouse_controlled -- --ignored --nocapture

# Run the opt-in public ClickHouse smoke benchmark.
benchmark-public:
    SEMANTIC_PUBLIC_CLICKHOUSE=1 cargo run -p semantic-performance --release --locked --bin clickhouse_public

# Build and run the CLI, forwarding arguments unchanged.
[positional-arguments]
cli *args:
    cargo run -p semantic-cli --locked -- "$@"

# Open an example REPL: geospatial (default), or github (requires GITHUB_TOKEN).
[positional-arguments]
repl example="geospatial":
    cargo run -p semantic-cli --locked -- --config "examples/$1/semantic-db.yaml"

# Install the pinned documentation dependencies.
docs-install:
    npm --prefix docs/site ci --no-audit --no-fund

# Launch the Astro documentation site (requires Node >=22.12 and npm).
[positional-arguments]
docs *args: docs-install
    npm --prefix docs/site run dev -- "$@"

# Build the GitHub Pages site and check internal links.
docs-build: docs-install
    npm --prefix docs/site run build
