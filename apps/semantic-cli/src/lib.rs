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
    group(ArgGroup::new("batch").args(["query", "file", "ask", "ask_views", "write", "explain_write"]))
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
    /// Execute one explicitly authored mutation through the write dispatcher.
    #[arg(long, value_name = "SQL")]
    write: Option<String>,
    /// Explain a mutation without executing its source or destination changes.
    #[arg(long, value_name = "SQL")]
    explain_write: Option<String>,
    #[arg(long, default_value = "observed", value_parser = ["observed", "snapshot"])]
    read_consistency: String,
    #[arg(
        long,
        default_value = "configured",
        value_name = "configured|bypass|MAX_AGE_SECONDS"
    )]
    read_cache: String,
    #[arg(long)]
    explain_read: bool,
    #[arg(long)]
    read_report: bool,
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
    /// Bypass configured materializations for this invocation.
    #[arg(long)]
    bypass_cache: bool,
    /// Query-wide deadline, including cache fills and local operators.
    #[arg(long, default_value_t = 30)]
    query_timeout_seconds: u64,
    /// List published materialization generations and exit.
    #[arg(long, requires = "config")]
    cache_status: bool,
    /// Invalidate a materialization key reported by --cache-status.
    #[arg(long, requires = "config")]
    cache_invalidate: Option<String>,
    /// Refresh one configured relation and exit.
    #[arg(long, requires = "config")]
    cache_refresh: Option<String>,
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
    engine.set_query_options(semantic_engine::QueryOptions {
        timeout_seconds: args.query_timeout_seconds,
        bypass_materialization: args.bypass_cache,
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
        return run_ask(&engine, &compiler, &request, args.dry_run, false).await;
    }
    if let Some(request) = args.ask_views {
        let compiler = Compiler::new(config::provider()?);
        return run_ask(&engine, &compiler, &request, args.dry_run, true).await;
    }
    if let Some(query) = args.query {
        if args.read_consistency == "observed"
            && args.read_cache == "configured"
            && !args.read_report
            && !args.explain_read
        {
            return run_sql(&engine, &query, args.dry_run).await;
        }
        if args.dry_run {
            return run_sql(&engine, &query, true).await;
        }
        let options = semantic_engine::ReadOptions {
            query: engine.query_options().clone(),
            consistency: if args.read_consistency == "snapshot" {
                semantic_engine::ReadConsistency::Snapshot
            } else {
                semantic_engine::ReadConsistency::Observed
            },
            cache: match args.read_cache.as_str() {
                "configured" => semantic_engine::ReadCache::Configured,
                "bypass" => semantic_engine::ReadCache::Bypass,
                age => {
                    semantic_engine::ReadCache::MaxAge(std::time::Duration::from_secs(age.parse()?))
                }
            },
            ..Default::default()
        };
        if args.explain_read {
            println!(
                "{}",
                serde_json::to_string_pretty(&engine.explain_read(&query, options).await?)?
            );
            return Ok(());
        }
        let result = engine
            .execute_read(&query, vec![], options)
            .await?
            .collect()
            .await?;
        println!("{}", pretty_format_batches(&result.batches)?);
        println!(
            "{} row(s)",
            result.batches.iter().map(|b| b.num_rows()).sum::<usize>()
        );
        if args.read_report {
            eprintln!("{}", serde_json::to_string_pretty(&result.report)?);
        }
        return Ok(());
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
                engine.plan_generated_sql(&query.sql).await?;
                let batches = engine.query(&query.sql).await?;
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
