#[tokio::main]
async fn main() -> semantic_cli::Result<()> {
    semantic_cli::run_with_registry(semantic_sources::Registry::standard()).await
}
