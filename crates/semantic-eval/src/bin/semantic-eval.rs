use clap::{Parser, Subcommand};
use semantic_eval::{ContextMode, Dataset, Interface, RunOptions};
use std::path::PathBuf;
#[derive(Parser)]
#[command(about = "Run standalone SQL and typed Ask acceptance datasets")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Validate {
        #[arg(long)]
        dataset: PathBuf,
    },
    Run(Args),
    Release(Args),
}
#[derive(clap::Args)]
struct Args {
    #[arg(long)]
    dataset: PathBuf,
    #[arg(long, value_enum, default_value = "both")]
    interface: Interface,
    #[arg(long, value_enum, default_value = "auto")]
    context: ContextMode,
    #[arg(long = "case")]
    cases: Vec<String>,
    #[arg(long)]
    repetitions: Option<usize>,
    #[arg(long, default_value = ".semantic-eval")]
    artifacts: PathBuf,
    #[arg(long)]
    attach: bool,
    #[arg(long)]
    keep_environment: bool,
    #[arg(long)]
    env_file: Option<PathBuf>,
    #[arg(long, default_value_t = 60)]
    timeout_seconds: u64,
    /// Execution decoding admission budget, separate from collected output.
    #[arg(long)]
    max_decoded_bytes: Option<usize>,
    #[arg(long)]
    max_requests: Option<usize>,
    #[arg(long)]
    max_remote_bytes: Option<usize>,
    #[arg(long, default_value_t = 32 * 1024 * 1024)]
    max_output_bytes: usize,
    #[arg(long, default_value_t = 100_000)]
    max_output_rows: usize,
    /// Minimum delay between harness model call starts; public subprocess calls are unaffected.
    #[arg(long,default_value_t=0,value_parser=clap::value_parser!(u64).range(0..=60000))]
    model_request_interval_ms: u64,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    cli_binary: Option<PathBuf>,
    #[arg(long)]
    public_interfaces: bool,
    /// Retain bounded model transcripts and compiled SQL/parameters in private artifacts.
    #[arg(long)]
    debug_capture: bool,
}
#[tokio::main]
async fn main() {
    if let Err(e) = execute().await {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
async fn execute() -> semantic_eval::Result<()> {
    let cli = Cli::parse();
    let (args, release) = match cli.command {
        Command::Validate { dataset } => {
            let d = Dataset::load(dataset)?;
            d.validate_project()?;
            println!(
                "Validated {} {}: {} paired, {} companion cases; digest {}",
                d.manifest.id,
                d.manifest.version,
                d.manifest.required_paired_cases,
                d.manifest.required_companion_cases,
                d.digest
            );
            return Ok(());
        }
        Command::Run(a) => (a, false),
        Command::Release(a) => (a, true),
    };
    let dataset = Dataset::load(args.dataset)?;
    let options = RunOptions {
        interface: args.interface,
        context: args.context,
        cases: args.cases,
        repetitions: args.repetitions.unwrap_or(if release { 3 } else { 1 }),
        release,
        artifacts: args.artifacts,
        attach: args.attach,
        keep_environment: args.keep_environment,
        timeout_seconds: args.timeout_seconds,
        max_decoded_bytes: args.max_decoded_bytes,
        max_requests: args.max_requests,
        max_remote_bytes: args.max_remote_bytes,
        max_bytes: args.max_output_bytes,
        max_rows: args.max_output_rows,
        model_request_interval_millis: args.model_request_interval_ms,
        model: args.model,
        env_file: args.env_file,
        cli_binary: args.cli_binary,
        public_interfaces: args.public_interfaces,
        debug_capture: args.debug_capture,
    };
    let report = semantic_eval::run(&dataset, options).await?;
    let passed = report.cases.iter().filter(|c| c.passed).count();
    println!(
        "{}: {passed}/{} passed; complete={}; report={}",
        report.dataset_id,
        report.cases.len(),
        report.complete,
        report.report_path.display()
    );
    if !report.success() {
        return Err("acceptance failed or incomplete; inspect report.json".into());
    }
    Ok(())
}
