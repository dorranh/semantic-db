//! Embed the example GitHub + local CSV project.
//! Reads process environment only. For .env, custom queries, or the REPL, use sdb-github.
use semantic_db::{engine::pretty_format_batches, sources::Project};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project = Project::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/semantic-db.yaml"))?;
    let imported = project
        .load(&example_github::registry()?, &|name| {
            std::env::var(name).ok()
        })
        .await?;
    for warning in imported.warnings {
        eprintln!("Warning: {warning}");
    }
    let batches = imported
        .engine
        .query(include_str!("../open_issues_by_team.sql"))
        .await?;
    println!("{}", pretty_format_batches(&batches)?);
    Ok(())
}
