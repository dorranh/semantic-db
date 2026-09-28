//! Run the standard Semantic DB CLI with the example GitHub connector registered.

#[tokio::main]
async fn main() -> semantic_cli::Result<()> {
    semantic_cli::run_with_registry(example_github::registry()?).await
}
