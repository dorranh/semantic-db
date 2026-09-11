//! Repository-scoped GitHub GraphQL + Ossie + local CSV federation.
use std::{collections::HashMap, path::PathBuf, time::Duration};

use clap::Parser;
use rustyline::{DefaultEditor, error::ReadlineError};
use semantic_db::{
    Compiler, Engine, GroundingOutcome,
    compiler::provider::{OpenAiConfig, OpenAiProvider},
    engine::pretty_format_batches,
    github::{GitHub, GitHubConfig},
    ossie::{OssieDocument, SourceBindings},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(about = "Query GitHub issues through an Ossie model and DataFusion")]
struct Args {
    /// Explore the configured tables in an interactive SQL/natural-language REPL.
    #[arg(long, conflicts_with_all = ["query", "file", "ask", "dry_run"])]
    repl: bool,
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
async fn main() -> Result<()> {
    let args = Args::parse();
    let file = match dotenvy::from_path_iter(".env") {
        Ok(values) => values
            .collect::<std::result::Result<HashMap<_, _>, _>>()
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
    let mut engine = imported.engine;
    eprintln!("Repository scope: {}", args.repo.join(", "));
    if args.repl {
        return repl(&mut engine, &github, &env).await;
    }
    if let Some(request) = args.ask {
        return run_ask(&engine, &github, &compiler(&env)?, &request, args.dry_run).await;
    }
    let sql = if let Some(path) = args.file {
        std::fs::read_to_string(path)?
    } else {
        args.query.unwrap_or_else(|| {
            include_str!("../../../examples/github/open_issues_by_team.sql").into()
        })
    };
    run_query(&engine, &github, &sql, args.dry_run).await
}

fn compiler(env: &impl Fn(&str) -> Option<String>) -> Result<Compiler<OpenAiProvider>> {
    let mut config = OpenAiConfig::new(
        env("OPENAI_API_KEY").ok_or("set OPENAI_API_KEY to use --ask, .ask or .plan")?,
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
    Ok(Compiler::new(OpenAiProvider::new(config)?))
}

async fn run_ask(
    engine: &Engine,
    github: &GitHub,
    compiler: &Compiler<OpenAiProvider>,
    request: &str,
    dry_run: bool,
) -> Result<()> {
    let compilation = compiler.compile(engine, request).await?;
    println!("{}", serde_json::to_string_pretty(&compilation)?);
    if let GroundingOutcome::Grounded { query } = compilation.outcome {
        run_query(engine, github, &query.sql, dry_run).await?;
    }
    Ok(())
}

async fn run_query(engine: &Engine, github: &GitHub, sql: &str, dry_run: bool) -> Result<()> {
    if dry_run {
        let frame = engine.plan_sql(sql).await?;
        println!("{}", frame.logical_plan().display_indent());
    } else {
        let before = github.request_count();
        let result = engine.query(sql).await;
        eprintln!("GitHub HTTP requests: {}", github.request_count() - before);
        let batches = result?;
        if batches.iter().all(|batch| batch.num_rows() == 0) {
            println!("0 rows.");
        } else {
            println!("{}", pretty_format_batches(&batches)?);
        }
    }
    Ok(())
}

async fn repl(
    engine: &mut Engine,
    github: &GitHub,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<()> {
    println!("GitHub Semantic DB — end SQL with ; or use .help");
    let mut editor = DefaultEditor::new()?;
    let mut pending = String::new();
    let mut model = None;
    loop {
        let prompt = if pending.is_empty() {
            "github> "
        } else {
            "    ... "
        };
        match editor.readline(prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if pending.is_empty() && trimmed.starts_with('.') {
                    if matches!(trimmed, ".quit" | ".exit") {
                        break;
                    }
                    editor.add_history_entry(trimmed)?;
                    if let Err(error) = command(engine, github, &mut model, env, trimmed).await {
                        eprintln!("Error: {error}");
                    }
                    continue;
                }
                if pending.is_empty() && trimmed.is_empty() {
                    continue;
                }
                pending.push_str(&line);
                pending.push('\n');
                // Match the CLI's simple framing: one statement per submission.
                if trimmed.ends_with(';') {
                    editor.add_history_entry(pending.trim())?;
                    if let Err(error) = run_query(engine, github, &pending, false).await {
                        eprintln!("Error: {error}");
                    }
                    pending.clear();
                }
            }
            Err(ReadlineError::Interrupted) => pending.clear(),
            Err(ReadlineError::Eof) => {
                if !pending.trim().is_empty() {
                    eprintln!("Discarded unfinished SQL; terminate statements with ;");
                }
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

async fn command(
    engine: &mut Engine,
    github: &GitHub,
    model: &mut Option<Compiler<OpenAiProvider>>,
    env: &impl Fn(&str) -> Option<String>,
    line: &str,
) -> Result<()> {
    let (command, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    match command {
        ".help" => println!(
            ".tables                 List relations\n\
             .schema NAME            Show fields and descriptions\n\
             .view NAME=SELECT ...   Create a view (one line)\n\
             .ask REQUEST           Compile and execute natural language\n\
             .plan REQUEST          Compile without reading GitHub rows\n\
             .quit                   Exit (or Ctrl-D)\n\
             End SQL with ;. Ctrl-C clears pending SQL. History is session-local.\n\
             .ask and .plan call the configured model. Each query performs fresh reads."
        ),
        ".tables" => {
            for relation in engine.catalog().relations() {
                println!("{}", relation.name);
            }
        }
        ".schema" => {
            let relation = engine
                .catalog()
                .relation(rest)
                .ok_or_else(|| format!("unknown relation: {rest}"))?;
            println!("{}", relation.name);
            if let Some(description) = &relation.description {
                println!("{description}");
            }
            for field in relation.schema.fields() {
                println!(
                    "  {}: {}{}",
                    field.name(),
                    field.data_type(),
                    if field.is_nullable() {
                        " (nullable)"
                    } else {
                        ""
                    }
                );
            }
        }
        ".view" => {
            let (name, sql) = rest
                .split_once('=')
                .ok_or("expected .view NAME=SELECT ...")?;
            engine.create_view(name.trim(), sql.trim()).await?;
            println!("Registered view {}", name.trim());
        }
        ".ask" | ".plan" => {
            if rest.is_empty() {
                return Err("provide a natural-language request".into());
            }
            if model.is_none() {
                *model = Some(compiler(env)?);
            }
            run_ask(
                engine,
                github,
                model.as_ref().expect("initialized compiler"),
                rest,
                command == ".plan",
            )
            .await?;
        }
        _ => return Err(format!("unknown command: {command}; use .help").into()),
    }
    Ok(())
}
