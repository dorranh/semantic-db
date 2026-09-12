use std::{
    error::Error,
    io::{self, IsTerminal, Read},
};

use clap::{ArgGroup, Parser, Subcommand};
use semantic_compiler::{Compiler, GroundingOutcome, provider::OpenAiProvider};
use semantic_engine::{Engine, pretty_format_batches};
use semantic_ossie::{ModelInspection, OssieDocument, SourceBindings};
use semantic_sources::{Project, Registry};

mod config;
mod init;
mod repl;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(
    name = "semantic-db",
    version,
    about = "Query registered data with SQL or natural language",
    args_conflicts_with_subcommands = true,
    group(ArgGroup::new("model_input").args(["config", "ossie"])),
    group(ArgGroup::new("batch").args(["query", "file", "ask", "ask_views"]))
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    /// Disable colors in the interactive REPL.
    #[arg(long)]
    no_color: bool,
    /// Keep interactive command history in memory only.
    #[arg(long, conflicts_with = "history_file")]
    no_history: bool,
    /// Override the interactive history file (shared across projects by default).
    #[arg(long, value_name = "PATH")]
    history_file: Option<std::path::PathBuf>,
    /// Load an Ossie model and source connections from a YAML/JSON project file.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["csv", "ossie", "ossie_model", "source_csv"])]
    config: Option<std::path::PathBuf>,
    /// Check the model, bindings and connector options offline, without credentials.
    #[arg(long, requires = "model_input", conflicts_with_all = ["inspect", "query", "file", "ask", "ask_views", "view", "dry_run"])]
    validate: bool,
    /// Also construct providers and validate physical schemas; requires --validate.
    #[arg(long, requires = "validate")]
    connect: bool,
    /// Show model fields, source requirements and configured connector names offline.
    #[arg(long, requires = "model_input", conflicts_with_all = ["validate", "query", "file", "ask", "ask_views", "view", "dry_run"])]
    inspect: bool,
    /// Register a CSV with a header row; repeat for multiple sources.
    #[arg(long, value_name = "NAME=PATH", value_parser = parse_assignment)]
    csv: Vec<(String, String)>,
    /// Load a pinned Ossie YAML/JSON semantic model with explicit source bindings.
    #[arg(long, value_name = "PATH", conflicts_with = "csv")]
    ossie: Option<std::path::PathBuf>,
    /// Select a model (required when the Ossie document contains multiple models).
    #[arg(long, value_name = "NAME", requires = "ossie")]
    ossie_model: Option<String>,
    /// Bind an Ossie source identifier to a CSV; repeat for multiple sources.
    #[arg(long, value_name = "SOURCE=PATH", requires = "ossie", value_parser = parse_assignment)]
    source_csv: Vec<(String, String)>,
    /// Register a view before querying; repeat in dependency order.
    #[arg(long, value_name = "NAME=SQL", value_parser = parse_assignment)]
    view: Vec<(String, String)>,
    /// Execute one SQL statement and exit.
    #[arg(short, long, conflicts_with_all = ["file", "ask"])]
    query: Option<String>,
    /// Read one SQL statement from a file and exit.
    #[arg(short, long, conflicts_with = "ask")]
    file: Option<std::path::PathBuf>,
    /// Compile a natural-language request, show SQL/evidence, and execute it.
    #[arg(long, value_name = "REQUEST")]
    ask: Option<String>,
    /// Select an authored view and compile a bounded query without model-written SQL.
    #[arg(long, value_name = "REQUEST")]
    ask_views: Option<String>,
    /// Plan SQL or compile natural language without executing query rows.
    #[arg(long, requires = "batch")]
    dry_run: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Create a runnable project with a CSV source, semantic model, and SQL view.
    Init {
        /// Project directory; defaults to the current directory.
        #[arg(default_value = ".")]
        path: std::path::PathBuf,
    },
}

fn parse_assignment(value: &str) -> std::result::Result<(String, String), String> {
    let (name, value) = value.split_once('=').ok_or("expected NAME=VALUE")?;
    if name.is_empty() || value.is_empty() {
        return Err("name and value must be nonempty".into());
    }
    Ok((name.into(), value.into()))
}

/// Run the standard CLI with an application-owned connector registry.
/// Custom binaries can register connectors and reuse all commands and the REPL.
pub async fn run_with_registry(registry: Registry) -> Result<()> {
    let args = Args::parse();
    if let Some(Command::Init { path }) = args.command {
        return init::run(&path);
    }
    let mut engine = if let Some(path) = args.config {
        let project = Project::from_path(path)?;
        if args.inspect || args.validate {
            let inspection = project.inspect_project(&registry)?;
            show_inspection(&inspection.model, args.inspect, |source| {
                project.source_connector(source).map(str::to_owned)
            });
            for view in &inspection.views {
                println!(
                    "  View {} ← {} (dependencies: {})",
                    view.name,
                    view.sql_file.display(),
                    view.dependencies.join(", ")
                );
                if let Some(description) = &view.description {
                    println!("    {description}");
                }
            }
            if !args.connect {
                println!(
                    "Offline validation passed; view syntax and dependencies checked. Physical schemas, view columns/types and access have not been checked."
                );
                return Ok(());
            }
        }
        let env = config::environment()?;
        let imported = project.load(&registry, &env).await?;
        for warning in imported.warnings {
            eprintln!("Warning: {warning}");
        }
        imported.engine
    } else if let Some(path) = args.ossie {
        let document = OssieDocument::parse(&std::fs::read_to_string(path)?)?;
        if args.inspect || args.validate {
            let inspection = document.inspect(args.ossie_model.as_deref())?;
            show_inspection(&inspection, args.inspect, |_| None);
            if !args.connect {
                println!(
                    "Offline model validation passed; source bindings and physical schemas have not been checked."
                );
                return Ok(());
            }
        }
        let mut bindings = SourceBindings::new();
        for (source, path) in args.source_csv {
            bindings.bind_csv(source, &path).await?;
        }
        let imported = document.load(args.ossie_model.as_deref(), &bindings)?;
        for warning in imported.warnings {
            eprintln!("Warning: {warning}");
        }
        imported.engine
    } else {
        Engine::new()
    };
    if args.validate {
        println!(
            "Connected schema validation passed for {} relation(s); no query rows executed. This does not prove remote row access.",
            engine.catalog().relations().count()
        );
        return Ok(());
    }
    for (name, path) in args.csv {
        engine.register_csv(&name, &path).await?;
    }
    for (name, sql) in args.view {
        engine.create_view(&name, &sql).await?;
    }
    if let Some(request) = args.ask {
        let compiler = Compiler::new(config::provider()?);
        return run_ask(&engine, &compiler, &request, args.dry_run, false).await;
    }
    if let Some(request) = args.ask_views {
        let compiler = Compiler::new(config::provider()?);
        return run_ask(&engine, &compiler, &request, args.dry_run, true).await;
    }
    if let Some(query) = args.query {
        return run_sql(&engine, &query, args.dry_run).await;
    }
    if let Some(path) = args.file {
        return run_sql(&engine, &std::fs::read_to_string(path)?, args.dry_run).await;
    }
    if !io::stdin().is_terminal() {
        let mut query = String::new();
        io::stdin().read_to_string(&mut query)?;
        return run_query(&engine, &query).await;
    }
    repl::run(
        &mut engine,
        args.no_color,
        args.no_history,
        args.history_file,
    )
    .await
}

fn show_inspection(
    inspection: &ModelInspection,
    fields: bool,
    connector: impl Fn(&str) -> Option<String>,
) {
    println!("Model: {}", inspection.name);
    for dataset in &inspection.datasets {
        println!(
            "  {} ← {} ({})",
            dataset.name,
            dataset.source,
            connector(&dataset.source)
                .as_deref()
                .unwrap_or("explicit provider binding required")
        );
        if fields {
            for field in &dataset.fields {
                println!(
                    "    {} ← {}: {}",
                    field.name,
                    field.source_column,
                    field.datatype.as_deref().unwrap_or("provider-derived")
                );
            }
        }
    }
    for warning in &inspection.warnings {
        eprintln!("Warning: {warning}");
    }
}

async fn run_sql(engine: &Engine, sql: &str, dry_run: bool) -> Result<()> {
    if dry_run {
        let frame = engine.plan_sql(sql).await?;
        println!("{}", frame.logical_plan().display_indent());
        Ok(())
    } else {
        run_query(engine, sql).await
    }
}

async fn run_query(engine: &Engine, sql: &str) -> Result<()> {
    if sql.trim().is_empty() {
        return Ok(());
    }
    let batches = engine.query(sql).await?;
    println!("{}", pretty_format_batches(&batches)?);
    let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    println!("{rows} row(s)");
    Ok(())
}

async fn run_ask(
    engine: &Engine,
    compiler: &Compiler<OpenAiProvider>,
    request: &str,
    dry_run: bool,
    views_only: bool,
) -> Result<()> {
    let compilation = if views_only {
        compiler.compile_views(engine, request).await?
    } else {
        compiler.compile(engine, request).await?
    };
    match compilation.outcome {
        GroundingOutcome::Grounded { query } => {
            println!(
                "SQL (validated in {} attempt(s)):\n{}",
                compilation.attempts, query.sql
            );
            for evidence in &query.evidence {
                println!(
                    "  {:?} → {}: {}",
                    evidence.phrase, evidence.catalog_reference, evidence.interpretation
                );
            }
            if !dry_run {
                let batches = engine
                    .plan_generated_sql(&query.sql)
                    .await?
                    .collect()
                    .await?;
                println!("{}", pretty_format_batches(&batches)?);
                let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
                println!("{rows} row(s)");
            }
        }
        GroundingOutcome::NeedsClarification { question, .. } => {
            println!("Needs clarification: {question}")
        }
        GroundingOutcome::Unsupported { reason } => println!("Unsupported: {reason}"),
    }
    Ok(())
}
