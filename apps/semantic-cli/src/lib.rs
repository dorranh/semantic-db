use std::{
    error::Error,
    io::{self, IsTerminal, Read},
};

use clap::{ArgGroup, Parser};
use rustyline::{DefaultEditor, error::ReadlineError};
use semantic_compiler::{Compiler, GroundingOutcome, provider::OpenAiProvider};
use semantic_engine::{Engine, pretty_format_batches};
use semantic_ossie::{ModelInspection, OssieDocument, SourceBindings};
use semantic_sources::{Project, Registry};

mod config;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(
    name = "semantic-db",
    version,
    about = "Query registered data with SQL or natural language",
    group(ArgGroup::new("model_input").args(["config", "ossie"])),
    group(ArgGroup::new("batch").args(["query", "file", "ask"]))
)]
struct Args {
    /// Load an Ossie model and source connections from a YAML/JSON project file.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["csv", "ossie", "ossie_model", "source_csv"])]
    config: Option<std::path::PathBuf>,
    /// Check the model, bindings and connector options offline, without credentials.
    #[arg(long, requires = "model_input", conflicts_with_all = ["inspect", "query", "file", "ask", "view", "dry_run"])]
    validate: bool,
    /// Also construct providers and validate physical schemas; requires --validate.
    #[arg(long, requires = "validate")]
    connect: bool,
    /// Show model fields, source requirements and configured connector names offline.
    #[arg(long, requires = "model_input", conflicts_with_all = ["validate", "query", "file", "ask", "view", "dry_run"])]
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
    /// Plan SQL or compile natural language without executing query rows.
    #[arg(long, requires = "batch")]
    dry_run: bool,
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
    let mut engine = if let Some(path) = args.config {
        let project = Project::from_path(path)?;
        if args.inspect || args.validate {
            let inspection = project.inspect(&registry)?;
            show_inspection(&inspection, args.inspect, |source| {
                project.source_connector(source).map(str::to_owned)
            });
            if !args.connect {
                println!(
                    "Offline validation passed; physical schemas and access have not been checked."
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
        return run_ask(&engine, &compiler, &request, args.dry_run).await;
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
    repl(&mut engine).await
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
) -> Result<()> {
    let compilation = compiler.compile(engine, request).await?;
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

async fn repl(engine: &mut Engine) -> Result<()> {
    println!("Semantic DB — end SQL with ; or use .help");
    let mut editor = DefaultEditor::new()?;
    let mut pending = String::new();
    let mut compiler = None;
    loop {
        let prompt = if pending.is_empty() {
            "semantic> "
        } else {
            "      ... "
        };
        match editor.readline(prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if pending.is_empty() && trimmed.starts_with('.') {
                    if matches!(trimmed, ".quit" | ".exit") {
                        break;
                    }
                    if let Err(error) = command(engine, &mut compiler, trimmed).await {
                        eprintln!("Error: {error}");
                    }
                    continue;
                }
                if pending.is_empty() && trimmed.is_empty() {
                    continue;
                }
                pending.push_str(&line);
                pending.push('\n');
                // Deliberately simple REPL framing: one statement per submission.
                // DataFusion performs SQL parsing; this is not a script parser.
                if trimmed.ends_with(';') {
                    editor.add_history_entry(pending.trim())?;
                    if let Err(error) = run_query(engine, &pending).await {
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
    compiler: &mut Option<Compiler<OpenAiProvider>>,
    line: &str,
) -> Result<()> {
    let (command, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    match command {
        ".help" => println!(
            ".tables                 List registered relations\n\
             .schema NAME            Show schema and definition\n\
             .view NAME=SELECT ...   Register an in-memory view (one line)\n\
             .ask REQUEST           Compile and execute natural language\n\
             .plan REQUEST          Compile and show SQL without execution\n\
             .quit                   Exit\n\
             SQL spans lines until a line ends with ;. Ctrl-C clears pending SQL."
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
            println!("{}\n{:?}", relation.name, relation.kind);
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
            let (name, sql) = parse_assignment(rest)?;
            engine.create_view(&name, &sql).await?;
            println!("Registered view {name}");
        }
        ".ask" | ".plan" => {
            if rest.is_empty() {
                return Err("provide a natural-language request".into());
            }
            if compiler.is_none() {
                *compiler = Some(Compiler::new(config::provider()?));
            }
            run_ask(
                engine,
                compiler.as_ref().expect("compiler initialized"),
                rest,
                command == ".plan",
            )
            .await?;
        }
        _ => return Err(format!("unknown command: {command}; use .help").into()),
    }
    Ok(())
}
