# Format the Rust workspace.
format:
    cargo fmt --all

# Lint every target and feature, treating warnings as errors.
lint:
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Type-check every target and feature without building executables.
check:
    cargo check --workspace --all-targets --all-features --locked

# Run offline workspace tests and the connector template tests.
test:
    cargo test --workspace --all-features --locked
    cargo test -p semantic-cli --example custom_connector --locked

# Run connector integration tests
test-connectors:
    cargo test --workspace --all-features --locked connector_integration -- --ignored

# Build and run the CLI, forwarding arguments unchanged.
[positional-arguments]
cli *args:
    cargo run -p semantic-cli --locked -- "$@"

# Open an example REPL: geospatial (default), or github (requires GITHUB_TOKEN).
[positional-arguments]
repl example="geospatial":
    cargo run -p semantic-cli --locked -- --config "examples/$1/semantic-db.yaml"
