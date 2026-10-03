use std::{
    error::Error,
    io::{self, IsTerminal, Read},
    path::PathBuf,
};

use clap::{ArgGroup, Args as ClapArgs, Parser, Subcommand};
use semantic_engine::{Engine, pretty_format_batches};
use semantic_interpreter::{GroundingOutcome, Interpreter, provider::OpenAiProvider};
use semantic_ossie::ModelInspection;
use semantic_sources::{Project, Registry};
use serde::Deserialize;

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

#[derive(ClapArgs)]
struct ProjectOptions {
    /// Load a Semantic DB project (YAML or JSON).
    #[arg(long, value_name = "PATH", conflicts_with = "no_project")]
    project_config: Option<PathBuf>,
    /// Start without a project, even if semantic-db.yaml is present.
    #[arg(long)]
    no_project: bool,
}

#[derive(ClapArgs)]
struct RequiredProject {
    /// Load a Semantic DB project (YAML or JSON).
    #[arg(long, value_name = "PATH")]
    project_config: Option<PathBuf>,
}

#[derive(ClapArgs, Default)]
struct ReadPolicyArgs {
    /// Request an observed read or a single-domain snapshot (which bypasses cache).
    #[arg(long, value_parser = ["observed", "snapshot"])]
    read_consistency: Option<String>,
    /// Use configured cache policy, bypass it, or set a maximum age in seconds.
    #[arg(
        long,
        value_name = "configured|bypass|max-age=SECONDS",
        value_parser = parse_read_cache
    )]
    read_cache: Option<ReadCacheArg>,
}

#[derive(ClapArgs)]
struct ReplArgs {
    #[command(flatten)]
    project: ProjectOptions,
    #[command(flatten)]
    read: ReadPolicyArgs,
    /// Use the new typed compiler for Ask mode, .ask, and .plan.
    #[arg(long)]
    experimental_compiler: bool,
    /// Query-wide deadline, including cache fills and local operators.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400))]
    query_timeout_seconds: u64,
    /// Disable colors in the interactive REPL.
    #[arg(long)]
    no_color: bool,
    /// Keep interactive command history in memory only.
    #[arg(long, conflicts_with = "history_file")]
    no_history: bool,
    /// Override the interactive history file.
    #[arg(long, value_name = "PATH")]
    history_file: Option<PathBuf>,
}

#[derive(ClapArgs)]
#[command(group(ArgGroup::new("input").args(["sql", "file"]).required(true)))]
struct SqlArgs {
    #[command(flatten)]
    project: ProjectOptions,
    #[command(flatten)]
    read: ReadPolicyArgs,
    /// One SQL statement, or '-' to read it from stdin.
    #[arg(value_name = "SQL", conflicts_with = "file")]
    sql: Option<String>,
    /// Read one SQL statement from a file.
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,
    /// Print a read's raw logical plan without executing rows.
    #[arg(long, conflicts_with = "explain")]
    plan: bool,
    /// Explain a read or write without executing rows or mutations.
    #[arg(long)]
    explain: bool,
    /// Print a JSON report after executing a read.
    #[arg(long)]
    read_report: bool,
    /// Query-wide deadline, including cache fills and local operators.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400))]
    query_timeout_seconds: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CompilerMode {
    SqlCompatibility,
    TypedFull,
    TypedRetrieved,
    TypedAuto,
}

#[derive(Clone, Copy)]
enum TypedPresentation {
    Cli,
    Repl,
}

#[derive(ClapArgs)]
struct AskArgs {
    /// Choose SQL compatibility or the deterministic typed compiler/context mode.
    #[arg(long, value_enum, default_value_t = CompilerMode::SqlCompatibility)]
    compiler_mode: CompilerMode,
    #[command(flatten)]
    project: RequiredProject,
    /// Natural-language request to compile and execute.
    request: String,
    #[command(flatten)]
    read: ReadPolicyArgs,
    /// Compile and show SQL/evidence without executing query rows; contacts the model.
    #[arg(long)]
    compile_only: bool,
    /// Print a JSON report after executing the read.
    #[arg(long)]
    read_report: bool,
    /// Query-wide deadline, including cache fills and local operators.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400))]
    query_timeout_seconds: u64,
}

#[derive(ClapArgs)]
struct PreparedArgs {
    #[command(flatten)]
    project: RequiredProject,
    /// JSON row template, declarations and values; '-' reads standard input.
    #[arg(long, value_name = "PATH")]
    file: PathBuf,
    /// Execute rows only after successful binding and current-scope validation.
    #[arg(long)]
    execute: bool,
    /// Query-wide deadline, including provider reads when --execute is set.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400))]
    query_timeout_seconds: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PreparedTypeWire {
    Boolean,
    Int64,
    Utf8,
    Date32,
    TimestampMicrosUtc,
}
impl From<PreparedTypeWire> for semantic_compiler::typed::PreparedType {
    fn from(value: PreparedTypeWire) -> Self {
        use semantic_compiler::typed::PreparedType;
        match value {
            PreparedTypeWire::Boolean => PreparedType::Boolean,
            PreparedTypeWire::Int64 => PreparedType::Int64,
            PreparedTypeWire::Utf8 => PreparedType::Utf8,
            PreparedTypeWire::Date32 => PreparedType::Date32,
            PreparedTypeWire::TimestampMicrosUtc => PreparedType::TimestampMicrosUtc,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedDeclarationWire {
    name: String,
    value_type: PreparedTypeWire,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedReferenceWire {
    instant_parameter: String,
    timezone: String,
    calendar: semantic_compiler::typed::Calendar,
    origin: semantic_compiler::typed::ContextOrigin,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedRequest {
    query: semantic_plan::typed::RowQuery,
    declarations: Vec<PreparedDeclarationWire>,
    values: std::collections::BTreeMap<String, semantic_plan::typed::Literal>,
    #[serde(default)]
    reference: Option<PreparedReferenceWire>,
}

#[derive(ClapArgs)]
struct ValidateArgs {
    #[command(flatten)]
    project: RequiredProject,
    /// Also construct providers and validate physical schemas.
    #[arg(long)]
    connect: bool,
}

#[derive(ClapArgs)]
struct CacheArgs {
    #[command(subcommand)]
    command: CacheCommand,
}

#[derive(Subcommand)]
enum CacheCommand {
    /// List published materialization generations.
    Status(RequiredProject),
    /// Refresh one configured relation.
    Refresh {
        #[command(flatten)]
        project: RequiredProject,
        name: String,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400))]
        query_timeout_seconds: u64,
    },
    /// Invalidate a materialization key reported by status.
    Invalidate {
        #[command(flatten)]
        project: RequiredProject,
        key: String,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400))]
        query_timeout_seconds: u64,
    },
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
    fn new(args: &ReadPolicyArgs, explain: bool, report: bool) -> Self {
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
            explain,
            report,
            custom: args.read_consistency.is_some()
                || args.read_cache.is_some()
                || explain
                || report,
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
    /// Open the interactive SQL and dot-command editor.
    Repl(ReplArgs),
    /// Execute one authored SQL statement.
    Sql(SqlArgs),
    /// Compile and execute a natural-language read.
    Ask(AskArgs),
    /// Bind a typed row template from JSON without a model; optionally execute it.
    CompilePrepared(PreparedArgs),
    /// Check a project offline or validate physical schemas.
    Validate(ValidateArgs),
    /// Inspect a project's model, sources, and views offline.
    Inspect(RequiredProject),
    /// Manage materialization cache generations.
    Cache(CacheArgs),
    /// Serve Semantic DB over PostgreSQL and HTTP Ask compilation.
    Server(semantic_server::ServerArgs),
    /// Create a runnable project with a CSV source, semantic model, and SQL view.
    Init {
        /// Project directory; defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

/// Run the standard CLI with an application-owned connector registry.
/// Custom binaries can register connectors and reuse all commands and the REPL.
pub async fn run_with_registry(registry: Registry) -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Init { path } => init::run(&path),
        Command::Server(args) => semantic_server::run_with_registry(args, registry).await,
        Command::Repl(args) => run_repl(args, registry).await,
        Command::Sql(args) => run_sql_command(args, registry).await,
        Command::Ask(args) => run_ask_command(args, registry).await,
        Command::CompilePrepared(args) => run_prepared_command(args, registry).await,
        Command::Validate(args) => run_validation(args, registry).await,
        Command::Inspect(args) => run_inspection(args, registry),
        Command::Cache(args) => run_cache(args, registry).await,
    }
}

fn usage_error(message: &str) -> ! {
    clap::Error::raw(clap::error::ErrorKind::ArgumentConflict, message).exit()
}

fn discovered_project(explicit: Option<PathBuf>) -> Result<Option<PathBuf>> {
    if explicit.is_some() {
        return Ok(explicit);
    }
    let path = PathBuf::from("semantic-db.yaml");
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

fn project_path(args: ProjectOptions) -> Result<Option<PathBuf>> {
    if args.no_project {
        Ok(None)
    } else {
        discovered_project(args.project_config)
    }
}

fn required_project_path(args: RequiredProject) -> Result<PathBuf> {
    discovered_project(args.project_config)?.ok_or_else(|| {
        "this action requires a project; pass --project-config PATH or run from a directory with semantic-db.yaml".into()
    })
}

async fn load_engine(path: Option<PathBuf>, registry: Registry) -> Result<Engine> {
    if let Some(path) = path {
        let project = Project::from_path(path)?;
        let env = std::sync::Arc::new(config::environment()?);
        if project.deferred_read_only() {
            let loaded = project
                .load_deferred_read_only(
                    std::sync::Arc::new(registry),
                    env,
                    semantic_engine::DeferredOptions::default(),
                )
                .await?;
            for warning in loaded.warnings {
                eprintln!("Warning: {warning}");
            }
            Ok(loaded.engine)
        } else {
            let imported = project.load(&registry, env.as_ref()).await?;
            for warning in imported.warnings {
                eprintln!("Warning: {warning}");
            }
            Ok(imported.engine)
        }
    } else {
        Ok(Engine::new())
    }
}

fn set_timeout(engine: &mut Engine, seconds: u64) -> Result<()> {
    engine.set_query_options(semantic_engine::QueryOptions {
        timeout_seconds: seconds,
        ..Default::default()
    })?;
    Ok(())
}

async fn run_repl(args: ReplArgs, registry: Registry) -> Result<()> {
    if !io::stdin().is_terminal() {
        return Err("sdb repl requires a terminal; use sdb sql - for stdin".into());
    }
    let path = project_path(args.project)?;
    let mut engine = load_engine(path.clone(), registry).await?;
    set_timeout(&mut engine, args.query_timeout_seconds)?;
    repl::run(
        &mut engine,
        path,
        args.experimental_compiler,
        args.no_color,
        args.no_history,
        args.history_file,
        ReadMode::new(&args.read, false, false),
    )
    .await
}

async fn run_sql_command(args: SqlArgs, registry: Registry) -> Result<()> {
    let sql = match (args.sql, args.file) {
        (Some(sql), None) if sql == "-" => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input)?;
            input
        }
        (Some(sql), None) => sql,
        (None, Some(path)) if path == std::path::Path::new("-") => {
            usage_error("use the positional '-' operand to read SQL from stdin")
        }
        (None, Some(path)) => std::fs::read_to_string(path)?,
        _ => usage_error("provide exactly one SQL string, file, or '-' for stdin"),
    };
    if sql.trim().is_empty() {
        usage_error("provide one nonempty SQL statement");
    }
    let write = semantic_engine::is_write_statement(&sql);
    if write && args.plan {
        usage_error("--plan accepts read SQL only");
    }
    if write
        && (args.read.read_consistency.is_some()
            || args.read.read_cache.is_some()
            || args.read_report)
    {
        usage_error("read policy and report options cannot be used with mutation SQL");
    }
    if args.plan
        && (args.read.read_consistency.is_some()
            || args.read.read_cache.is_some()
            || args.read_report)
    {
        usage_error("--plan cannot be combined with read policy or report options");
    }
    if args.explain && args.read_report {
        usage_error("--explain cannot be combined with --read-report");
    }
    let path = project_path(args.project)?;
    if write && path.is_none() {
        return Err("mutation SQL requires a project with an explicit write binding".into());
    }
    let mut engine = load_engine(path, registry).await?;
    set_timeout(&mut engine, args.query_timeout_seconds)?;
    if write {
        run_write(
            &engine,
            &sql,
            args.explain || semantic_engine::is_write_explanation(&sql),
        )
        .await
    } else {
        let mode = ReadMode::new(&args.read, args.explain, args.read_report);
        run_sql(&engine, &sql, args.plan, &mode).await
    }
}

async fn run_ask_command(args: AskArgs, registry: Registry) -> Result<()> {
    if args.compile_only
        && (args.read.read_consistency.is_some()
            || args.read.read_cache.is_some()
            || args.read_report)
    {
        usage_error("--compile-only cannot be combined with read policy or report options");
    }
    let path = required_project_path(args.project)?;
    let mut engine = load_engine(Some(path), registry).await?;
    set_timeout(&mut engine, args.query_timeout_seconds)?;
    let compiler = Interpreter::new(config::provider()?);
    if args.compiler_mode != CompilerMode::SqlCompatibility {
        return run_typed_ask(
            &engine,
            &compiler,
            &args.request,
            args.compile_only,
            &ReadMode::new(&args.read, false, args.read_report),
            args.compiler_mode,
            TypedPresentation::Cli,
        )
        .await;
    }
    run_ask(
        &engine,
        &compiler,
        &args.request,
        args.compile_only,
        &ReadMode::new(&args.read, false, args.read_report),
        None,
    )
    .await
}

async fn run_prepared_command(args: PreparedArgs, registry: Registry) -> Result<()> {
    use semantic_compiler::typed::{
        CompileOptions, ParameterDeclaration, PreparedReferenceContext, PreparedRows, TypedOutcome,
    };
    const MAX_INPUT_BYTES: u64 = 16 * 1024;
    let mut input = Vec::new();
    if args.file == std::path::Path::new("-") {
        io::stdin()
            .take(MAX_INPUT_BYTES + 1)
            .read_to_end(&mut input)?;
    } else {
        std::fs::File::open(&args.file)?
            .take(MAX_INPUT_BYTES + 1)
            .read_to_end(&mut input)?;
    }
    if input.is_empty() || input.len() as u64 > MAX_INPUT_BYTES {
        return Err("prepared JSON must contain 1 to 16384 bytes".into());
    }
    let request: PreparedRequest =
        serde_json::from_slice(&input).map_err(|_| "invalid prepared row JSON")?;
    let path = required_project_path(args.project)?;
    let mut engine = load_engine(Some(path), registry).await?;
    set_timeout(&mut engine, args.query_timeout_seconds)?;
    let current_scope = engine
        .catalog()
        .snapshot()
        .relations()
        .map(|relation| relation.definition().name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut options = CompileOptions::default();
    options.timeout = std::time::Duration::from_secs(args.query_timeout_seconds);
    options.allowed_relations = Some(current_scope.clone());
    let declarations = request
        .declarations
        .into_iter()
        .map(|declaration| ParameterDeclaration {
            name: declaration.name,
            value_type: declaration.value_type.into(),
        })
        .collect();
    let reference = request.reference.map(|reference| PreparedReferenceContext {
        instant_parameter: reference.instant_parameter,
        timezone: reference.timezone,
        calendar: reference.calendar,
        origin: reference.origin,
    });
    let prepared = PreparedRows::prepare(&engine, request.query, declarations, reference, options)
        .map_err(|diagnostic| format!("prepared row rejected: {}", diagnostic.code))?;
    let compilation = prepared
        .bind_values(&engine, request.values, Some(&current_scope))
        .await
        .map_err(|diagnostic| format!("prepared row rejected: {}", diagnostic.code))?;
    match compilation.outcome {
        TypedOutcome::Compiled { query } => {
            println!("SQL:\n{}", query.sql().statement());
            println!(
                "Parameter types: {}",
                serde_json::to_string(query.sql().parameter_types())?
            );
            if args.execute {
                let batches = query
                    .execute_authorized(&engine, &current_scope, engine.query_options().clone())
                    .await
                    .map_err(|diagnostic| {
                        format!("prepared row execution rejected: {}", diagnostic.code)
                    })?
                    .collect()
                    .await?;
                println!("{}", pretty_format_batches(&batches)?);
                println!(
                    "{} row(s)",
                    batches.iter().map(|batch| batch.num_rows()).sum::<usize>()
                );
            }
            Ok(())
        }
        TypedOutcome::Rejected { diagnostic }
        | TypedOutcome::Unresolved { diagnostic }
        | TypedOutcome::ProviderFailure { diagnostic } => {
            Err(format!("prepared row rejected: {}", diagnostic.code).into())
        }
        TypedOutcome::NeedsClarification { .. } => Err("prepared row needs clarification".into()),
        TypedOutcome::Unsupported { .. } => Err("prepared row unsupported".into()),
        TypedOutcome::CompiledGraph { .. } => Err("prepared row returned a graph".into()),
    }
}

fn show_project_inspection(
    project: &Project,
    inspection: &semantic_sources::ProjectInspection,
    fields: bool,
) {
    show_inspection(&inspection.model, fields, |source| {
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
}

fn run_inspection(args: RequiredProject, registry: Registry) -> Result<()> {
    let project = Project::from_path(required_project_path(args)?)?;
    let inspection = project.inspect_project(&registry)?;
    show_project_inspection(&project, &inspection, true);
    println!(
        "Offline validation passed; view syntax and dependencies checked. Physical schemas, view columns/types and access have not been checked."
    );
    Ok(())
}

async fn run_validation(args: ValidateArgs, registry: Registry) -> Result<()> {
    let project = Project::from_path(required_project_path(args.project)?)?;
    let inspection = project.inspect_project(&registry)?;
    show_project_inspection(&project, &inspection, false);
    if !args.connect {
        println!(
            "Offline validation passed; view syntax and dependencies checked. Physical schemas, view columns/types and access have not been checked."
        );
        return Ok(());
    }
    let env = config::environment()?;
    let imported = project.load(&registry, &env).await?;
    for warning in imported.warnings {
        eprintln!("Warning: {warning}");
    }
    println!(
        "Connected schema validation passed for {} relation(s); no query rows executed. This does not prove remote row access.",
        imported.engine.catalog().relations().count()
    );
    Ok(())
}

async fn run_cache(args: CacheArgs, registry: Registry) -> Result<()> {
    match args.command {
        CacheCommand::Status(project_args) => {
            let project = Project::from_path(required_project_path(project_args)?)?;
            let manager = semantic_engine::MaterializationManager::new(
                project
                    .cache_options()
                    .ok_or("project has no cache configuration")?,
            )?;
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
        CacheCommand::Invalidate {
            project,
            key,
            query_timeout_seconds,
        } => {
            let project = Project::from_path(required_project_path(project)?)?;
            let manager = semantic_engine::MaterializationManager::new(
                project
                    .cache_options()
                    .ok_or("project has no cache configuration")?,
            )?;
            manager
                .invalidate(
                    &key,
                    &semantic_engine::QueryContext::new(semantic_engine::QueryOptions {
                        timeout_seconds: query_timeout_seconds,
                        ..Default::default()
                    })?,
                )
                .await?;
        }
        CacheCommand::Refresh {
            project,
            name,
            query_timeout_seconds,
        } => {
            let path = required_project_path(project)?;
            let mut engine = load_engine(Some(path), registry).await?;
            set_timeout(&mut engine, query_timeout_seconds)?;
            engine.refresh_materialization(&name).await?;
        }
    }
    Ok(())
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

async fn run_sql(engine: &Engine, sql: &str, plan: bool, read_mode: &ReadMode) -> Result<()> {
    if plan {
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
    compiler: &Interpreter<OpenAiProvider>,
    request: &str,
    compile_only: bool,
    read_mode: &ReadMode,
    progress: Option<&repl::ProgressReporter>,
) -> Result<()> {
    let indicator = progress.map(|reporter| reporter.start("Compiling request"));
    let compilation = compiler.compile(engine, request).await;
    if let Some(indicator) = indicator {
        indicator.finish();
    }
    let compilation = compilation?;
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
            if !compile_only {
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

async fn run_typed_ask(
    engine: &Engine,
    compiler: &Interpreter<OpenAiProvider>,
    request: &str,
    compile_only: bool,
    read_mode: &ReadMode,
    mode: CompilerMode,
    presentation: TypedPresentation,
) -> Result<()> {
    use semantic_compiler::typed::TypedOutcome;
    use semantic_interpreter::typed::{InterpretOptions, SelectionMode};
    let mut options = InterpretOptions::default();
    options.selection_mode = match mode {
        CompilerMode::TypedFull => SelectionMode::Full,
        CompilerMode::TypedRetrieved => SelectionMode::Retrieved,
        CompilerMode::TypedAuto => SelectionMode::Auto,
        CompilerMode::SqlCompatibility => unreachable!(),
    };
    let compilation = compiler.compile_typed(engine, request, options).await;
    if compile_only && matches!(presentation, TypedPresentation::Cli) {
        println!("{}", serde_json::to_string_pretty(&compilation)?);
        return Ok(());
    }
    if matches!(presentation, TypedPresentation::Repl)
        && let Some(explanation) = repl::explain_compilation(&compilation)
    {
        println!("{explanation}");
    }
    match compilation.outcome {
        TypedOutcome::Compiled { query } => {
            println!("SQL:\n{}", query.sql().statement());
            println!(
                "Parameters: {}",
                serde_json::to_string(query.sql().parameters())?
            );
            if compile_only {
                return Ok(());
            }
            let (batches, report) = if read_mode.custom {
                let result = query
                    .execute_read(engine, read_mode.options(engine))
                    .await?
                    .collect()
                    .await?;
                (result.batches, read_mode.report.then_some(result.report))
            } else {
                (
                    query
                        .execute(engine, engine.query_options().clone())
                        .await?
                        .collect()
                        .await?,
                    None,
                )
            };
            println!("{}", pretty_format_batches(&batches)?);
            println!(
                "{} row(s)",
                batches.iter().map(|batch| batch.num_rows()).sum::<usize>()
            );
            if let Some(report) = report {
                eprintln!("{}", serde_json::to_string_pretty(&report)?);
            }
        }
        TypedOutcome::CompiledGraph { query } => {
            println!("SQL:\n{}", query.sql().statement());
            println!(
                "Parameters: {}",
                serde_json::to_string(query.sql().parameters())?
            );
            if compile_only {
                return Ok(());
            }
            let (batches, report) = if read_mode.custom {
                let result = query
                    .execute_read(engine, read_mode.options(engine))
                    .await?
                    .collect()
                    .await?;
                (result.batches, read_mode.report.then_some(result.report))
            } else {
                (
                    query
                        .execute(engine, engine.query_options().clone())
                        .await?
                        .collect()
                        .await?,
                    None,
                )
            };
            println!("{}", pretty_format_batches(&batches)?);
            println!(
                "{} row(s)",
                batches.iter().map(|batch| batch.num_rows()).sum::<usize>()
            );
            if let Some(report) = report {
                eprintln!("{}", serde_json::to_string_pretty(&report)?);
            }
        }
        TypedOutcome::NeedsClarification { question, .. } => {
            println!("Needs clarification: {question}")
        }
        TypedOutcome::Unsupported { reason } => println!("Unsupported: {reason}"),
        TypedOutcome::Unresolved { diagnostic } => println!("Unresolved: {diagnostic}"),
        TypedOutcome::Rejected { diagnostic } | TypedOutcome::ProviderFailure { diagnostic } => {
            return Err(diagnostic.into());
        }
    }
    Ok(())
}
