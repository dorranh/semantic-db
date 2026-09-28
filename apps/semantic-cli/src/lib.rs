use std::{
    error::Error,
    io::{self, IsTerminal, Read},
};

use clap::{ArgGroup, Parser, Subcommand};
use semantic_compiler::{Compiler, GroundingOutcome, provider::OpenAiProvider};
use semantic_engine::{Engine, pretty_format_batches};
use semantic_ossie::ModelInspection;
use semantic_sources::{Project, Registry};

mod config;
mod init;
mod repl;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(
    name = "sdb",
    version,
    propagate_version = true,
    about = "Query and serve registered data with SQL or natural language",
    arg_required_else_help = true
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Args)]
#[command(
    group(ArgGroup::new("action").args(["validate", "inspect", "query", "file", "ask", "ask_views", "write", "explain_write", "cache_status", "cache_invalidate", "cache_refresh"])),
    group(ArgGroup::new("plannable").args(["query", "file", "ask", "ask_views", "write"]))
)]
struct ReplArgs {
    /// Load a Semantic DB project (YAML or JSON), including its model and sources.
    #[arg(
        long,
        value_name = "PATH",
        help_heading = "Project and validation",
        conflicts_with = "no_project"
    )]
    project_config: Option<std::path::PathBuf>,
    /// Start without a project, even when semantic-db.yaml is in the current directory.
    #[arg(long, help_heading = "Project and validation")]
    no_project: bool,
    /// Check the model, bindings and connector options offline, without credentials.
    #[arg(long, help_heading = "Project and validation")]
    validate: bool,
    /// Also construct providers and validate physical schemas; requires --validate.
    #[arg(long, help_heading = "Project and validation", requires = "validate")]
    connect: bool,
    /// Show model fields, source requirements and configured connector names offline.
    #[arg(long, help_heading = "Project and validation")]
    inspect: bool,
    /// Execute one SQL statement and exit.
    #[arg(short, long, help_heading = "Queries and writes")]
    query: Option<String>,
    /// Read one SQL statement from a file and exit.
    #[arg(short, long, help_heading = "Queries and writes")]
    file: Option<std::path::PathBuf>,
    /// Compile a natural-language request, show SQL/evidence, and execute it.
    #[arg(long, value_name = "REQUEST", help_heading = "Queries and writes")]
    ask: Option<String>,
    /// Select an authored view and compile a bounded query without model-written SQL.
    #[arg(long, value_name = "REQUEST", help_heading = "Queries and writes")]
    ask_views: Option<String>,
    /// Execute one explicitly authored mutation through the write dispatcher.
    #[arg(long, value_name = "SQL", help_heading = "Queries and writes")]
    write: Option<String>,
    /// Explain a mutation without executing its source or destination changes.
    #[arg(long, value_name = "SQL", help_heading = "Queries and writes")]
    explain_write: Option<String>,
    /// Plan SQL or compile natural language without executing query rows.
    #[arg(long, help_heading = "Queries and writes", requires = "plannable")]
    dry_run: bool,
    /// Request an observed read or a single-domain snapshot (which bypasses cache).
    #[arg(long, help_heading = "Read options", value_parser = ["observed", "snapshot"], conflicts_with_all = ["validate", "inspect", "write", "explain_write", "cache_status", "cache_invalidate", "cache_refresh", "dry_run"])]
    read_consistency: Option<String>,
    /// Use configured cache policy, bypass it, or set a maximum age in seconds.
    #[arg(
        long,
        help_heading = "Read options",
        value_name = "configured|bypass|max-age=SECONDS",
        value_parser = parse_read_cache,
        conflicts_with_all = ["validate", "inspect", "write", "explain_write", "cache_status", "cache_invalidate", "cache_refresh", "dry_run"]
    )]
    read_cache: Option<ReadCacheArg>,
    /// Show read dependencies and consistency checks without executing rows.
    #[arg(long, help_heading = "Read options", conflicts_with_all = ["validate", "inspect", "write", "explain_write", "cache_status", "cache_invalidate", "cache_refresh", "dry_run", "read_report"])]
    explain_read: bool,
    /// Print a JSON report after executing a read.
    #[arg(long, help_heading = "Read options", conflicts_with_all = ["validate", "inspect", "write", "explain_write", "cache_status", "cache_invalidate", "cache_refresh", "dry_run", "explain_read"])]
    read_report: bool,
    /// Query-wide deadline, including cache fills and local operators.
    #[arg(long, help_heading = "Execution limits", default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400))]
    query_timeout_seconds: u64,
    /// List published materialization generations and exit.
    #[arg(long, help_heading = "Cache maintenance")]
    cache_status: bool,
    /// Invalidate a materialization key reported by --cache-status.
    #[arg(long, help_heading = "Cache maintenance")]
    cache_invalidate: Option<String>,
    /// Refresh one configured relation and exit.
    #[arg(long, help_heading = "Cache maintenance")]
    cache_refresh: Option<String>,
    /// Disable colors in the interactive REPL.
    #[arg(long, help_heading = "Interactive preferences")]
    no_color: bool,
    /// Keep interactive command history in memory only.
    #[arg(
        long,
        help_heading = "Interactive preferences",
        conflicts_with = "history_file"
    )]
    no_history: bool,
    /// Override the interactive history file (shared across projects by default).
    #[arg(long, value_name = "PATH", help_heading = "Interactive preferences")]
    history_file: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug)]
enum ReadCacheArg {
    Configured,
    Bypass,
    MaxAge(u64),
}

fn parse_read_cache(value: &str) -> std::result::Result<ReadCacheArg, String> {
    match value {
        "configured" => Ok(ReadCacheArg::Configured),
        "bypass" => Ok(ReadCacheArg::Bypass),
        _ => {
            let seconds = value
                .strip_prefix("max-age=")
                .ok_or("expected configured, bypass, or max-age=SECONDS")?;
            let seconds = seconds
                .parse::<u64>()
                .map_err(|_| "max-age must be a nonnegative integer number of seconds")?;
            Ok(ReadCacheArg::MaxAge(seconds))
        }
    }
}

#[derive(Clone)]
struct ReadMode {
    consistency: semantic_engine::ReadConsistency,
    cache: semantic_engine::ReadCache,
    explain: bool,
    report: bool,
    custom: bool,
}

impl ReadMode {
    fn from_args(args: &ReplArgs) -> Self {
        Self {
            consistency: if args.read_consistency.as_deref() == Some("snapshot") {
                semantic_engine::ReadConsistency::Snapshot
            } else {
                semantic_engine::ReadConsistency::Observed
            },
            cache: match args.read_cache.as_ref() {
                None | Some(ReadCacheArg::Configured) => semantic_engine::ReadCache::Configured,
                Some(ReadCacheArg::Bypass) => semantic_engine::ReadCache::Bypass,
                Some(ReadCacheArg::MaxAge(seconds)) => {
                    semantic_engine::ReadCache::MaxAge(std::time::Duration::from_secs(*seconds))
                }
            },
            explain: args.explain_read,
            report: args.read_report,
            custom: args.read_consistency.is_some()
                || args.read_cache.is_some()
                || args.explain_read
                || args.read_report,
        }
    }

    fn options(&self, engine: &Engine) -> semantic_engine::ReadOptions {
        semantic_engine::ReadOptions {
            query: engine.query_options().clone(),
            consistency: self.consistency,
            cache: self.cache.clone(),
            ..Default::default()
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Open the interactive REPL or execute a query, validation, or cache operation.
    Repl(Box<ReplArgs>),
    /// Serve Semantic DB over PostgreSQL and HTTP Ask compilation.
    Server(semantic_server::ServerArgs),
    /// Create a runnable project with a CSV source, semantic model, and SQL view.
    Init {
        /// Project directory; defaults to the current directory.
        #[arg(default_value = ".")]
        path: std::path::PathBuf,
    },
}

/// Run the standard CLI with an application-owned connector registry.
/// Custom binaries can register connectors and reuse all commands and the REPL.
pub async fn run_with_registry(registry: Registry) -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Init { path } => init::run(&path),
        Command::Server(args) => semantic_server::run_with_registry(args, registry).await,
        Command::Repl(args) => run_repl(*args, registry).await,
    }
}

async fn run_repl(args: ReplArgs, registry: Registry) -> Result<()> {
    let project_path = project_path(&args)?;
    if project_path.is_none()
        && (args.validate
            || args.inspect
            || args.cache_status
            || args.cache_invalidate.is_some()
            || args.cache_refresh.is_some())
    {
        return Err("this action requires a project; pass --project-config PATH or run from a directory with semantic-db.yaml".into());
    }
    let read_mode = ReadMode::from_args(&args);
    let mut engine = if let Some(path) = project_path {
        let project = Project::from_path(path)?;
        if (args.cache_status || args.cache_invalidate.is_some()) && args.cache_refresh.is_none() {
            let manager = semantic_engine::MaterializationManager::new(
                project
                    .cache_options()
                    .ok_or("project has no cache configuration")?,
            )?;
            if let Some(key) = &args.cache_invalidate {
                manager
                    .invalidate(
                        key,
                        &semantic_engine::QueryContext::new(semantic_engine::QueryOptions {
                            timeout_seconds: args.query_timeout_seconds,
                            ..Default::default()
                        })?,
                    )
                    .await?;
            }
            if args.cache_status {
                for manifest in manager.status()? {
                    println!(
                        "{} generation={} rows={} disk_bytes={} acquired_at_ms={}",
                        manifest.key,
                        manifest.generation,
                        manifest.rows,
                        manifest.disk_bytes,
                        manifest.acquired_at_ms
                    );
                }
            }
            return Ok(());
        }
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
    engine.set_query_options(semantic_engine::QueryOptions {
        timeout_seconds: args.query_timeout_seconds,
        ..Default::default()
    })?;
    if args.cache_status || args.cache_invalidate.is_some() || args.cache_refresh.is_some() {
        let manager = engine
            .materialization_manager()
            .ok_or("project has no cache configuration")?;
        if let Some(key) = args.cache_invalidate {
            manager
                .invalidate(
                    &key,
                    &semantic_engine::QueryContext::new(engine.query_options().clone())?,
                )
                .await?;
        }
        if let Some(name) = args.cache_refresh {
            engine.refresh_materialization(&name).await?;
        }
        if args.cache_status {
            for manifest in manager.status()? {
                println!(
                    "{} generation={} rows={} disk_bytes={} acquired_at_ms={}",
                    manifest.key,
                    manifest.generation,
                    manifest.rows,
                    manifest.disk_bytes,
                    manifest.acquired_at_ms
                );
            }
        }
        return Ok(());
    }
    if let Some(sql) = args.explain_write {
        return run_write(&engine, &sql, true).await;
    }
    if let Some(sql) = args.write {
        return run_write(&engine, &sql, args.dry_run).await;
    }
    if let Some(request) = args.ask {
        let compiler = Compiler::new(config::provider()?);
        return run_ask(
            &engine,
            &compiler,
            &request,
            args.dry_run,
            false,
            &read_mode,
        )
        .await;
    }
    if let Some(request) = args.ask_views {
        let compiler = Compiler::new(config::provider()?);
        return run_ask(&engine, &compiler, &request, args.dry_run, true, &read_mode).await;
    }
    if let Some(query) = args.query {
        return run_sql(&engine, &query, args.dry_run, &read_mode).await;
    }
    if let Some(path) = args.file {
        return run_sql(
            &engine,
            &std::fs::read_to_string(path)?,
            args.dry_run,
            &read_mode,
        )
        .await;
    }
    if !io::stdin().is_terminal() {
        let mut query = String::new();
        io::stdin().read_to_string(&mut query)?;
        return run_query(&engine, &query, &read_mode).await;
    }
    repl::run(
        &mut engine,
        args.no_color,
        args.no_history,
        args.history_file,
        read_mode,
    )
    .await
}

fn project_path(args: &ReplArgs) -> Result<Option<std::path::PathBuf>> {
    if args.no_project {
        return Ok(None);
    }
    if let Some(path) = &args.project_config {
        return Ok(Some(path.clone()));
    }
    let path = std::path::PathBuf::from("semantic-db.yaml");
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {
            eprintln!("Using project: {}", path.display());
            Ok(Some(path))
        }
        Ok(_) => Err("semantic-db.yaml exists but is not a regular file".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
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

async fn run_sql(engine: &Engine, sql: &str, dry_run: bool, read_mode: &ReadMode) -> Result<()> {
    if dry_run {
        let frame = engine.plan_sql(sql).await?;
        println!("{}", frame.logical_plan().display_indent());
        Ok(())
    } else {
        run_query(engine, sql, read_mode).await
    }
}

async fn run_query(engine: &Engine, sql: &str, read_mode: &ReadMode) -> Result<()> {
    if sql.trim().is_empty() {
        return Ok(());
    }
    if read_mode.explain {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &engine.explain_read(sql, read_mode.options(engine)).await?
            )?
        );
        return Ok(());
    }
    let (batches, report) = if read_mode.custom {
        let result = engine
            .execute_read(sql, vec![], read_mode.options(engine))
            .await?
            .collect()
            .await?;
        (result.batches, read_mode.report.then_some(result.report))
    } else {
        (engine.query(sql).await?, None)
    };
    println!("{}", pretty_format_batches(&batches)?);
    let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    println!("{rows} row(s)");
    if let Some(report) = report {
        eprintln!("{}", serde_json::to_string_pretty(&report)?);
    }
    Ok(())
}

async fn run_ask(
    engine: &Engine,
    compiler: &Compiler<OpenAiProvider>,
    request: &str,
    dry_run: bool,
    views_only: bool,
    read_mode: &ReadMode,
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
                engine.plan_generated_sql(&query.sql).await?;
                run_query(engine, &query.sql, read_mode).await?;
            }
        }
        GroundingOutcome::NeedsClarification { question, .. } => {
            println!("Needs clarification: {question}")
        }
        GroundingOutcome::Unsupported { reason } => println!("Unsupported: {reason}"),
    }
    Ok(())
}

async fn run_write(engine: &Engine, sql: &str, explain: bool) -> Result<()> {
    let prepared = engine.prepare_write(sql).await?;
    if explain {
        println!(
            "{}",
            serde_json::to_string_pretty(&prepared.explain().await?)?
        );
        return Ok(());
    }
    let result = prepared
        .execute(
            vec![],
            semantic_engine::WriteOptions {
                query: engine.query_options().clone(),
                ..Default::default()
            },
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    if !result.success() {
        return Err(format!("write outcome: {:?}", result.outcome).into());
    }
    Ok(())
}
