use crate::{
    http::{StateData, router},
    protocol::Handlers,
};
use clap::Args;
use semantic_db::{
    interpreter::{
        Interpreter,
        provider::{OpenAiConfig, OpenAiProvider},
    },
    sources::{Project, Registry},
};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    sync::Arc,
};
/// Options for the local PostgreSQL and HTTP server.
#[derive(Args)]
pub struct ServerArgs {
    /// Load a Semantic DB project (YAML or JSON), including its model and sources.
    #[arg(long = "project-config", value_name = "PATH")]
    pub config: Option<PathBuf>,
    #[arg(long, default_value_t = 5544)]
    pub port: u16,
    #[arg(long, default_value_t = 5545)]
    pub http_port: u16,
    /// Deadline for each query, including all remote pages and local processing.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86400))]
    pub query_timeout_seconds: u64,
    /// Maximum source requests, including each remote cursor fetch.
    #[arg(long, value_parser = positive_query_limit)]
    pub query_max_requests: Option<usize>,
    /// Maximum admitted decoding bytes, including source scratch estimates.
    #[arg(long, value_parser = positive_query_limit)]
    pub query_max_decoded_bytes: Option<usize>,
    /// Maximum remote bytes admitted by the query.
    #[arg(long, value_parser = positive_query_limit)]
    pub query_max_remote_bytes: Option<usize>,
}
fn positive_query_limit(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "query limit must be a positive integer".into())
}
fn query_options(
    current: &semantic_db::QueryOptions,
    seconds: u64,
    requests: Option<usize>,
    decoded: Option<usize>,
    remote: Option<usize>,
) -> Result<semantic_db::QueryOptions, Box<dyn std::error::Error>> {
    let mut options = current.clone();
    options.timeout_seconds = seconds;
    if let Some(value) = requests {
        options.max_remote_requests = value;
    }
    if let Some(value) = decoded {
        options.max_decoded_bytes = value;
    }
    if let Some(value) = remote {
        options.max_remote_bytes = value;
    }
    options.validate()?;
    Ok(options)
}
/// Serve a project with an application-owned connector registry.
pub async fn run_with_registry(
    args: ServerArgs,
    registry: Registry,
) -> Result<(), Box<dyn std::error::Error>> {
    query_options(
        &semantic_db::QueryOptions::default(),
        args.query_timeout_seconds,
        args.query_max_requests,
        args.query_max_decoded_bytes,
        args.query_max_remote_bytes,
    )?;
    let project_path = match args.config {
        Some(path) => path,
        None => {
            let path = PathBuf::from("semantic-db.yaml");
            match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    eprintln!("Using project: {}", path.display());
                    path
                }
                Ok(_) => return Err("semantic-db.yaml exists but is not a regular file".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err("sdb server requires a project; pass --project-config PATH or run from a directory with semantic-db.yaml".into());
                }
                Err(error) => return Err(error.into()),
            }
        }
    };
    let file: HashMap<String, String> = match dotenvy::from_path_iter(".env") {
        Ok(values) => values
            .collect::<Result<_, _>>()
            .map_err(|_| "invalid .env syntax")?,
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(_) => return Err("could not read .env".into()),
    };
    let secret: Arc<semantic_db::sources::SecretResolver<'static>> =
        Arc::new(move |name: &str| std::env::var(name).ok().or_else(|| file.get(name).cloned()));
    let (mut engine, warnings) =
        load_project_engine(&project_path, registry, secret.clone()).await?;
    for warning in warnings {
        eprintln!("Warning: {warning}");
    }
    engine.set_query_options(query_options(
        engine.query_options(),
        args.query_timeout_seconds,
        args.query_max_requests,
        args.query_max_decoded_bytes,
        args.query_max_remote_bytes,
    )?)?;
    let engine = Arc::new(engine);
    let compiler = if let Some(key) = secret("OPENAI_API_KEY").filter(|s| !s.is_empty()) {
        let model = secret("OPENAI_MODEL").ok_or("set OPENAI_MODEL when enabling Ask")?;
        let mut config = OpenAiConfig::new(key, model);
        if let Some(url) = secret("OPENAI_BASE_URL") {
            config.base_url = url;
        }
        Some(Interpreter::new(OpenAiProvider::new(config)?))
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

async fn load_project_engine(
    path: &Path,
    registry: Registry,
    secret: Arc<semantic_db::sources::SecretResolver<'static>>,
) -> Result<(semantic_db::Engine, Vec<String>), Box<dyn std::error::Error>> {
    let project = Project::from_path(path)?;
    if project.deferred_read_only() {
        let loaded = project
            .load_deferred_read_only(
                Arc::new(registry),
                secret,
                semantic_db::engine::DeferredOptions::default(),
            )
            .await?;
        Ok((
            loaded.engine,
            loaded
                .warnings
                .into_iter()
                .map(|warning| warning.to_string())
                .collect(),
        ))
    } else {
        let loaded = project.load(&registry, secret.as_ref()).await?;
        Ok((
            loaded.engine,
            loaded
                .warnings
                .into_iter()
                .map(|warning| warning.to_string())
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_db::arrow::datatypes::{DataType, Field, Schema};
    use serde_json::json;

    #[tokio::test]
    async fn deferred_server_project_binds_recorded_schema_without_opening_source() {
        let directory =
            std::env::temp_dir().join(format!("semantic-server-deferred-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("semantic-db.json");
        std::fs::write(
            &path,
            json!({
                "deferred_read_only":true,
                "connections":{"local":{"connector":"csv"}},
                "app_tables":{"items":{
                    "connection":"local",
                    "path":"missing.csv",
                    "recorded_schema":Schema::new(vec![Field::new("id",DataType::Int64,true)])
                }}
            })
            .to_string(),
        )
        .unwrap();
        let (engine, _) = load_project_engine(&path, Registry::standard(), Arc::new(|_| None))
            .await
            .unwrap();
        assert!(engine.catalog().relation("items").is_some());
        let error = engine.query("SELECT id FROM items").await.err().unwrap();
        assert!(error.to_string().contains("missing.csv"));
        let _ = std::fs::remove_dir_all(directory);
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn controls_preserve_cache_policy_and_unmodified_admission_limits() {
        let current = semantic_db::QueryOptions {
            max_remote_requests: 1024,
            bypass_materialization: true,
            max_cache_age_ms: Some(456),
            refresh_materializations: vec!["facts".into()],
            ..Default::default()
        };
        let resolved = query_options(&current, 120, None, Some(999999), None).unwrap();
        assert_eq!(resolved.max_remote_requests, 1024);
        assert_eq!(resolved.max_decoded_bytes, 999999);
        assert_eq!(resolved.max_remote_bytes, current.max_remote_bytes);
        assert_eq!(resolved.timeout_seconds, 120);
        assert!(resolved.bypass_materialization);
        assert_eq!(resolved.max_cache_age_ms, current.max_cache_age_ms);
        assert_eq!(
            resolved.refresh_materializations,
            current.refresh_materializations
        );
    }
    #[tokio::test]
    async fn invalid_capacity_is_rejected_before_project_or_provider_loading() {
        let args = ServerArgs {
            config: Some("project-must-not-be-opened.yaml".into()),
            port: 0,
            http_port: 0,
            query_timeout_seconds: 30,
            query_max_requests: Some(0),
            query_max_decoded_bytes: None,
            query_max_remote_bytes: None,
        };
        let error = run_with_registry(args, Registry::standard())
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("invalid query budgets"),
            "{error}"
        );
    }
}
