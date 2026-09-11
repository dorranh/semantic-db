//! Repository-scoped GitHub GraphQL + Ossie + local CSV federation.
use std::{collections::HashMap, path::PathBuf, time::Duration};

use clap::Parser;
use semantic_db::{
    Compiler, GroundingOutcome,
    compiler::provider::{OpenAiConfig, OpenAiProvider},
    engine::pretty_format_batches,
    github::{GitHub, GitHubConfig},
    ossie::{OssieDocument, SourceBindings},
};

#[derive(Parser)]
#[command(about = "Query GitHub issues through an Ossie model and DataFusion")]
struct Args {
    /// Repository scope; repeat for multiple repositories. No global GitHub scan.
    #[arg(long, default_value = "apache/ossie")]
    repo: Vec<String>,
    #[arg(long, conflicts_with_all = ["file", "ask"])]
    query: Option<String>,
    #[arg(long, conflicts_with_all = ["query", "ask"])]
    file: Option<PathBuf>,
    #[arg(long, conflicts_with_all = ["query", "file"])]
    ask: Option<String>,
    /// Compile/plan only: no GitHub row requests. --ask still calls the model.
    #[arg(long)]
    dry_run: bool,
    /// Local, authored repository/team mapping (not GitHub organization teams).
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/github/repository_teams.csv"))]
    teams: String,
    #[arg(long, default_value_t = 100)]
    page_size: usize,
    /// Hard request budget for each scan, including nested label pages.
    #[arg(long, default_value_t = 100)]
    max_requests: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let file = match dotenvy::from_path_iter(".env") {
        Ok(values) => values
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(|_| "invalid .env syntax")?,
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            HashMap::new()
        }
        Err(_) => return Err("could not read .env".into()),
    };
    let env = |key: &str| std::env::var(key).ok().or_else(|| file.get(key).cloned());
    let token = env("GITHUB_TOKEN")
        .filter(|v| !v.trim().is_empty())
        .ok_or("set GITHUB_TOKEN in the environment or .env")?;
    let mut config = GitHubConfig::new(token, args.repo.clone());
    config.page_size = args.page_size;
    config.max_requests_per_scan = args.max_requests;
    let github = GitHub::new(config)?;
    let mut sources = SourceBindings::new();
    sources.bind("github.scoped.issues", github.issues()?)?;
    sources.bind("github.scoped.issue_labels", github.issue_labels()?)?;
    sources
        .bind_csv("local.repository_teams", &args.teams)
        .await?;
    let document =
        OssieDocument::parse(include_str!("../../../examples/github/github.ossie.yaml"))?;
    let imported = document.load(Some("github_maintenance"), &sources)?;
    for warning in imported.warnings {
        eprintln!("Warning: {warning}");
    }
    let engine = imported.engine;
    eprintln!("Repository scope: {}", args.repo.join(", "));
    let sql = if let Some(request) = args.ask {
        let mut config = OpenAiConfig::new(
            env("OPENAI_API_KEY").ok_or("set OPENAI_API_KEY to use --ask")?,
            env("OPENAI_MODEL").unwrap_or_else(|| "gpt-4.1-mini".into()),
        );
        if let Some(url) = env("OPENAI_BASE_URL") {
            config.base_url = url;
        }
        if let Some(seconds) = env("OPENAI_TIMEOUT_SECONDS") {
            config.timeout = Duration::from_secs(
                seconds
                    .parse()
                    .map_err(|_| "invalid OPENAI_TIMEOUT_SECONDS")?,
            );
        }
        if let Some(value) = env("OPENAI_JSON_MODE") {
            config.json_mode = value.parse().map_err(|_| "invalid OPENAI_JSON_MODE")?;
        }
        let compilation = Compiler::new(OpenAiProvider::new(config)?)
            .compile(&engine, &request)
            .await?;
        eprintln!("{}", serde_json::to_string_pretty(&compilation)?);
        match compilation.outcome {
            GroundingOutcome::Grounded { query } => query.sql,
            _ => return Ok(()),
        }
    } else if let Some(path) = args.file {
        std::fs::read_to_string(path)?
    } else {
        args.query.unwrap_or_else(|| {
            include_str!("../../../examples/github/open_issues_by_team.sql").into()
        })
    };
    if args.dry_run {
        let frame = engine.plan_sql(&sql).await?;
        println!("{}", frame.logical_plan().display_indent());
    } else {
        let result = engine.query(&sql).await;
        eprintln!("GitHub HTTP requests: {}", github.request_count());
        let batches = result?;
        if batches.iter().all(|batch| batch.num_rows() == 0) {
            println!("0 rows.");
        } else {
            println!("{}", pretty_format_batches(&batches)?);
        }
    }
    Ok(())
}
