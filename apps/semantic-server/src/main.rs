use clap::Parser;
use semantic_db::{
    compiler::{
        Compiler,
        provider::{OpenAiConfig, OpenAiProvider},
    },
    sources::{Project, Registry},
};
use semantic_server::{
    http::{StateData, router},
    protocol::Handlers,
};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
};
#[derive(Parser)]
#[command(
    version,
    about = "Serve Semantic DB over PostgreSQL and HTTP Ask compilation"
)]
struct Args {
    #[arg(long)]
    config: PathBuf,
    #[arg(long, default_value_t = 5544)]
    port: u16,
    #[arg(long, default_value_t = 5545)]
    http_port: u16,
    /// Deadline for each query, including all remote pages and local processing.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400))]
    query_timeout_seconds: u64,
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let file: HashMap<String, String> = match dotenvy::from_path_iter(".env") {
        Ok(values) => values
            .collect::<Result<_, _>>()
            .map_err(|_| "invalid .env syntax")?,
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(_) => return Err("could not read .env".into()),
    };
    let secret = |name: &str| std::env::var(name).ok().or_else(|| file.get(name).cloned());
    let mut imported = Project::from_path(&args.config)?
        .load(&Registry::standard(), &secret)
        .await?;
    imported
        .engine
        .set_query_options(semantic_db::QueryOptions {
            timeout_seconds: args.query_timeout_seconds,
            ..Default::default()
        })?;
    let engine = Arc::new(imported.engine);
    let compiler = if let Some(key) = secret("OPENAI_API_KEY").filter(|s| !s.is_empty()) {
        let model = secret("OPENAI_MODEL").ok_or("set OPENAI_MODEL when enabling Ask")?;
        let mut config = OpenAiConfig::new(key, model);
        if let Some(url) = secret("OPENAI_BASE_URL") {
            config.base_url = url;
        }
        Some(Compiler::new(OpenAiProvider::new(config)?))
    } else {
        None
    };
    let listener =
        tokio::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), args.http_port)).await?;
    let http = axum::serve(
        listener,
        router(StateData {
            engine: engine.clone(),
            compiler,
        }),
    );
    let opts = datafusion_postgres::ServerOptions::new()
        .with_host("127.0.0.1".into())
        .with_port(args.port)
        .with_max_connections(32);
    eprintln!(
        "Semantic DB: pg 127.0.0.1:{}, compilation 127.0.0.1:{}",
        args.port, args.http_port
    );
    tokio::select! {
        result = datafusion_postgres::serve_with_handlers(Arc::new(Handlers::new(engine)), &opts) => result?,
        result = http => result?,
        _ = tokio::signal::ctrl_c() => {},
    }
    Ok(())
}
