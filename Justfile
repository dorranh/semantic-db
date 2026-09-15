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

# Set up and launch the package-maintenance dashboard, preserving application edits.
package-maintenance: package-maintenance-setup
    npm --prefix examples/package-maintenance start

# Install dependencies, start databases, apply migrations, and seed missing records.
package-maintenance-setup:
    npm --prefix examples/package-maintenance ci
    npm --prefix examples/package-maintenance run setup

# Stop the dashboard and databases, preserving their volumes.
package-maintenance-stop:
    npm --prefix examples/package-maintenance run stop

# DELETE the example database volumes and recreate the fixtures.
package-maintenance-reset:
    npm --prefix examples/package-maintenance run reset

# Run application unit tests, TypeScript checks, and the production UI build after setup.
package-maintenance-check:
    npm --prefix examples/package-maintenance test
    npm --prefix examples/package-maintenance run build

# Test the running dashboard; requires agent-browser and its Chromium installation.
package-maintenance-test:
    npm --prefix examples/package-maintenance run test:integration
    npm --prefix examples/package-maintenance run test:e2e

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
