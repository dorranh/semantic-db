//! Embed the same GitHub + local CSV project used by the CLI.
//! Reads process environment only. For .env, custom queries, or the REPL, use semantic-cli.
use semantic_db::{
    engine::pretty_format_batches,
    sources::{Project, Registry},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project = Project::from_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/github/semantic-db.yaml"
    ))?;
    let imported = project
        .load(&Registry::standard(), &|name| std::env::var(name).ok())
        .await?;
    for warning in imported.warnings {
        eprintln!("Warning: {warning}");
    }
    let batches = imported
        .engine
        .query(include_str!(
            "../../../examples/github/open_issues_by_team.sql"
        ))
        .await?;
    println!("{}", pretty_format_batches(&batches)?);
    Ok(())
}
