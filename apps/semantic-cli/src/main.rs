use std::{
    error::Error,
    io::{self, IsTerminal, Read},
};

use clap::Parser;
use rustyline::{DefaultEditor, error::ReadlineError};
use semantic_engine::{Engine, pretty_format_batches};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(
    name = "semantic-db",
    version,
    about = "Query registered data with DataFusion SQL"
)]
struct Args {
    /// Register a CSV with a header row; repeat for multiple sources.
    #[arg(long, value_name = "NAME=PATH", value_parser = parse_assignment)]
    csv: Vec<(String, String)>,
    /// Register a view before querying; repeat in dependency order.
    #[arg(long, value_name = "NAME=SQL", value_parser = parse_assignment)]
    view: Vec<(String, String)>,
    /// Execute one SQL statement and exit.
    #[arg(short, long, conflicts_with = "file")]
    query: Option<String>,
    /// Read one SQL statement from a file and exit.
    #[arg(short, long)]
    file: Option<std::path::PathBuf>,
}

fn parse_assignment(value: &str) -> std::result::Result<(String, String), String> {
    let (name, value) = value.split_once('=').ok_or("expected NAME=VALUE")?;
    if name.is_empty() || value.is_empty() {
        return Err("name and value must be nonempty".into());
    }
    Ok((name.into(), value.into()))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mut engine = Engine::new();
    for (name, path) in args.csv {
        engine.register_csv(&name, &path).await?;
    }
    for (name, sql) in args.view {
        engine.create_view(&name, &sql).await?;
    }
    if let Some(query) = args.query {
        return run_query(&engine, &query).await;
    }
    if let Some(path) = args.file {
        return run_query(&engine, &std::fs::read_to_string(path)?).await;
    }
    if !io::stdin().is_terminal() {
        let mut query = String::new();
        io::stdin().read_to_string(&mut query)?;
        return run_query(&engine, &query).await;
    }
    repl(&mut engine).await
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

async fn repl(engine: &mut Engine) -> Result<()> {
    println!("Semantic DB — end SQL with ; or use .help");
    let mut editor = DefaultEditor::new()?;
    let mut pending = String::new();
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
                    if let Err(error) = command(engine, trimmed).await {
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

async fn command(engine: &mut Engine, line: &str) -> Result<()> {
    let (command, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    match command {
        ".help" => println!(
            ".tables                 List registered relations\n\
             .schema NAME            Show schema and definition\n\
             .view NAME=SELECT ...   Register an in-memory view (one line)\n\
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
        _ => return Err(format!("unknown command: {command}; use .help").into()),
    }
    Ok(())
}
