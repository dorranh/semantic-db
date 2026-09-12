//! The same configured source loader for command-line and embedded applications.
//!
//! Parse a [`Project`], inspect it offline, then load it using a [`Registry`] and
//! an application-supplied secret resolver. No environment variables are read by
//! this library. Connectors return standard DataFusion providers; the Ossie
//! importer owns semantic projection and schema validation.

mod builtin;
pub mod conformance;

use datafusion::catalog::TableProvider;
use futures::future::BoxFuture;
use semantic_ossie::{ImportedCatalog, ModelInspection, OssieDocument, SourceBindings};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

#[cfg(feature = "clickhouse")]
pub use builtin::ClickHouseConnector;
pub use builtin::CsvConnector;
#[cfg(feature = "github")]
pub use builtin::GitHubConnector;

pub type Options = Map<String, Value>;
pub type Result<T> = std::result::Result<T, SourceError>;
/// Applications can resolve names from environment, a secret store, or a map.
pub type SecretResolver<'a> = dyn Fn(&str) -> Option<String> + Send + Sync + 'a;

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("[{code}] {path}: {message}")]
    Configuration {
        code: &'static str,
        path: String,
        message: String,
    },
    #[error(transparent)]
    Import(#[from] semantic_ossie::ImportError),
    #[error(transparent)]
    Engine(#[from] semantic_engine::EngineError),
    #[error(transparent)]
    DataFusion(#[from] datafusion::error::DataFusionError),
}
impl SourceError {
    pub fn configuration(
        code: &'static str,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::Configuration {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
    fn context(self, path: String) -> Self {
        Self::configuration("connector", path, self.to_string())
    }
}

/// Configuration validation must be offline and must not resolve secrets.
/// `connect` may obtain metadata, but table rows belong in lazy execution streams.
/// Return sanitized errors: the host includes connector errors in diagnostics.
pub trait ConnectorFactory: Send + Sync {
    fn validate_connection(&self, options: &Options) -> Result<()>;
    fn validate_source(&self, options: &Options) -> Result<()>;
    fn connect<'a>(
        &'a self,
        options: &'a Options,
        secrets: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>>;
}

/// A reusable connection/client serving one or more source bindings.
pub trait SourceConnection: Send + Sync {
    fn table<'a>(
        &'a self,
        options: &'a Options,
        base_dir: &'a Path,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>>;
}

#[derive(Default)]
pub struct Registry {
    factories: BTreeMap<String, Arc<dyn ConnectorFactory>>,
}
impl Registry {
    /// Empty registry for applications that want an explicit connector allowlist.
    pub fn new() -> Self {
        Self::default()
    }

    /// CSV plus connectors enabled by the `github` and `clickhouse` features.
    pub fn standard() -> Self {
        let mut registry = Self::new();
        registry
            .register("csv", CsvConnector)
            .expect("unique builtin");
        #[cfg(feature = "github")]
        registry
            .register("github", GitHubConnector)
            .expect("unique builtin");
        #[cfg(feature = "clickhouse")]
        registry
            .register("clickhouse", ClickHouseConnector)
            .expect("unique builtin");
        registry
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        factory: impl ConnectorFactory + 'static,
    ) -> Result<()> {
        let name = name.into();
        if name.trim().is_empty() || self.factories.contains_key(&name) {
            return Err(SourceError::configuration(
                "connector_registration",
                "/connectors",
                "connector name must be nonempty and unique",
            ));
        }
        self.factories.insert(name, Arc::new(factory));
        Ok(())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }

    fn factory(&self, name: &str, path: &str) -> Result<&Arc<dyn ConnectorFactory>> {
        self.factories.get(name).ok_or_else(|| {
            SourceError::configuration(
                "unknown_connector",
                path,
                format!(
                    "connector {name:?} is not compiled/registered; available: {}",
                    self.names().collect::<Vec<_>>().join(", ")
                ),
            )
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub ossie: PathBuf,
    pub model: Option<String>,
    #[serde(default)]
    pub connections: BTreeMap<String, ConnectionConfig>,
    #[serde(default)]
    pub sources: BTreeMap<String, SourceConfig>,
    #[serde(default)]
    pub views: BTreeMap<String, ViewConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewConfig {
    pub sql_file: PathBuf,
    pub description: Option<String>,
}

/// Offline project inspection. View columns/types are checked only on load.
#[derive(Debug)]
pub struct ProjectInspection {
    pub model: ModelInspection,
    /// Definitions in dependency order, following the selected model's datasets.
    pub views: Vec<ViewInspection>,
}

#[derive(Debug, Clone)]
pub struct ViewInspection {
    pub name: String,
    pub sql_file: PathBuf,
    pub description: Option<String>,
    pub sql: String,
    pub dependencies: Vec<String>,
}
#[derive(Deserialize)]
pub struct ConnectionConfig {
    pub connector: String,
    #[serde(flatten)]
    pub options: Options,
}
#[derive(Deserialize)]
pub struct SourceConfig {
    pub connection: String,
    #[serde(flatten)]
    pub options: Options,
}

pub struct Project {
    config: ProjectConfig,
    document: OssieDocument,
    base_dir: PathBuf,
    views: BTreeMap<String, ViewInspection>,
}
impl Project {
    /// Model, view SQL and local source paths are relative to this configuration file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| {
            SourceError::configuration("config_read", path.display().to_string(), e.to_string())
        })?;
        let options = serde_saphyr::options! { strict_booleans: true };
        // Parse via Value so duplicate keys are rejected before flattened maps.
        let value: Value = serde_saphyr::from_str_with_options(&text, options).map_err(|_| {
            SourceError::configuration(
                "config_parse",
                path.display().to_string(),
                "invalid YAML/JSON or duplicate keys",
            )
        })?;
        let config: ProjectConfig = serde_json::from_value(value).map_err(|e| {
            SourceError::configuration("config_schema", path.display().to_string(), e.to_string())
        })?;
        let base_dir = path.parent().unwrap_or(Path::new(".")).to_owned();
        let model_path = base_dir.join(&config.ossie);
        let yaml = std::fs::read_to_string(&model_path).map_err(|e| {
            SourceError::configuration(
                "model_read",
                model_path.display().to_string(),
                e.to_string(),
            )
        })?;
        Self::new(config, OssieDocument::parse(&yaml)?, base_dir)
    }

    /// In-memory equivalent of `from_path`; the caller supplies the document and
    /// base directory explicitly. The `ossie` path is not read by this method.
    /// View SQL files are read once here; recreate the Project to pick up edits.
    pub fn new(config: ProjectConfig, document: OssieDocument, base_dir: PathBuf) -> Result<Self> {
        if config.ossie.as_os_str().is_empty() {
            return Err(SourceError::configuration(
                "config_schema",
                "/ossie",
                "model path must be nonempty",
            ));
        }
        // Path::parent("project.yaml") is the empty path, meaning the current
        // directory. std::path::absolute requires that spelling to be explicit.
        let base_dir = if base_dir.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            base_dir
        };
        let base_dir = std::path::absolute(&base_dir).map_err(|error| {
            SourceError::configuration(
                "project_path",
                base_dir.display().to_string(),
                error.to_string(),
            )
        })?;
        let mut views = BTreeMap::new();
        for (name, view) in &config.views {
            let path = format!("/views/{}/sql_file", pointer(name));
            if view.sql_file.as_os_str().is_empty() {
                return Err(SourceError::configuration(
                    "view_file",
                    path,
                    "SQL file path must be nonempty",
                ));
            }
            let file = base_dir.join(&view.sql_file);
            let sql = std::fs::read_to_string(&file).map_err(|error| {
                SourceError::configuration(
                    "view_read",
                    path,
                    format!("{}: {error}", file.display()),
                )
            })?;
            views.insert(
                name.clone(),
                ViewInspection {
                    name: name.clone(),
                    sql_file: view.sql_file.clone(),
                    description: view.description.clone(),
                    sql,
                    dependencies: Vec::new(),
                },
            );
        }
        Ok(Self {
            config,
            document,
            base_dir,
            views,
        })
    }

    /// No credential resolution, provider construction, or source I/O.
    /// Checks all configured options; only the selected model must be bound.
    /// Also validates configured views; use `inspect_project` to inspect them.
    pub fn inspect(&self, registry: &Registry) -> Result<ModelInspection> {
        Ok(self.inspect_project(registry)?.model)
    }

    /// Offline model inspection plus project views in dependency order.
    pub fn inspect_project(&self, registry: &Registry) -> Result<ProjectInspection> {
        let inspection = self.document.inspect(self.config.model.as_deref())?;
        for (name, connection) in &self.config.connections {
            let path = format!("/connections/{}", pointer(name));
            if name.trim().is_empty() {
                return Err(SourceError::configuration(
                    "connection_name",
                    path,
                    "connection name must be nonempty",
                ));
            }
            registry
                .factory(&connection.connector, &path)?
                .validate_connection(&connection.options)
                .map_err(|e| e.context(path))?;
        }
        for (name, source) in &self.config.sources {
            let path = format!("/sources/{}", pointer(name));
            let connection = self
                .config
                .connections
                .get(&source.connection)
                .ok_or_else(|| {
                    SourceError::configuration(
                        "missing_connection",
                        &path,
                        format!("define connection {:?} in connections", source.connection),
                    )
                })?;
            registry
                .factory(&connection.connector, &path)?
                .validate_source(&source.options)
                .map_err(|e| e.context(path))?;
        }
        for dataset in &inspection.datasets {
            if !self.config.sources.contains_key(&dataset.source) {
                return Err(SourceError::configuration(
                    "missing_binding",
                    format!("{}/source", dataset.path),
                    format!(
                        "add sources[{source:?}] to the project configuration for dataset {name:?}",
                        source = dataset.source,
                        name = dataset.name
                    ),
                ));
            }
        }
        let views = self.inspect_views(&inspection)?;
        Ok(ProjectInspection {
            model: inspection,
            views,
        })
    }

    fn inspect_views(&self, model: &ModelInspection) -> Result<Vec<ViewInspection>> {
        let engine = semantic_engine::Engine::new();
        let mut ready: BTreeSet<_> = model
            .datasets
            .iter()
            .map(|dataset| dataset.name.clone())
            .collect();
        let mut pending = self.views.clone();
        for (name, view) in &mut pending {
            let path = format!("/views/{}", pointer(name));
            if ready.contains(name) {
                return Err(SourceError::configuration(
                    "duplicate_relation",
                    path,
                    "view name conflicts with a dataset in the selected model",
                ));
            }
            view.dependencies =
                engine
                    .validate_view_definition(name, &view.sql)
                    .map_err(|error| {
                        SourceError::configuration(
                            "view_definition",
                            format!("{path}/sql_file"),
                            format!("{}: {error}", view.sql_file.display()),
                        )
                    })?;
            for dependency in &view.dependencies {
                if !ready.contains(dependency) && !self.views.contains_key(dependency) {
                    return Err(SourceError::configuration(
                        "missing_view_dependency",
                        path,
                        format!("unknown relation {dependency:?}"),
                    ));
                }
            }
        }
        let mut ordered = Vec::new();
        while !pending.is_empty() {
            let next = pending
                .iter()
                .find(|(_, view)| view.dependencies.iter().all(|name| ready.contains(name)))
                .map(|(name, _)| name.clone());
            let Some(name) = next else {
                return Err(SourceError::configuration(
                    "cyclic_views",
                    "/views",
                    format!(
                        "cyclic dependencies; blocked views: {}",
                        pending.keys().cloned().collect::<Vec<_>>().join(", ")
                    ),
                ));
            };
            ready.insert(name.clone());
            ordered.push(pending.remove(&name).expect("selected pending view"));
        }
        Ok(ordered)
    }

    pub fn source_connector(&self, source: &str) -> Option<&str> {
        let binding = self.config.sources.get(source)?;
        Some(&self.config.connections.get(&binding.connection)?.connector)
    }

    /// Validate offline first, resolve each required connection/source once, then
    /// check provider schemas through Ossie. No partial engine is returned.
    /// CSV schema inference reads a sample; remote connectors may fetch metadata.
    pub async fn load(
        &self,
        registry: &Registry,
        secrets: &SecretResolver<'_>,
    ) -> Result<ImportedCatalog> {
        let inspection = self.inspect_project(registry)?;
        let required: BTreeSet<_> = inspection
            .model
            .datasets
            .iter()
            .map(|d| d.source.as_str())
            .collect();
        let mut connections: BTreeMap<&str, Arc<dyn SourceConnection>> = BTreeMap::new();
        let mut bindings = SourceBindings::new();
        for source in required {
            let binding = &self.config.sources[source];
            let config = &self.config.connections[&binding.connection];
            let path = format!("/connections/{}", pointer(&binding.connection));
            if !connections.contains_key(binding.connection.as_str()) {
                let connection = registry
                    .factory(&config.connector, &path)?
                    .connect(&config.options, secrets)
                    .await
                    .map_err(|e| e.context(path))?;
                connections.insert(&binding.connection, connection);
            }
            let provider = connections[binding.connection.as_str()]
                .table(&binding.options, &self.base_dir)
                .await
                .map_err(|e| e.context(format!("/sources/{}", pointer(source))))?;
            bindings.bind(source, provider)?;
        }
        let mut imported = self
            .document
            .load(self.config.model.as_deref(), &bindings)?;
        for view in inspection.views {
            imported
                .engine
                .create_view_with_description(&view.name, &view.sql, view.description.as_deref())
                .await
                .map_err(|error| {
                    SourceError::configuration(
                        "view_plan",
                        format!("/views/{}/sql_file", pointer(&view.name)),
                        format!("{}: {error}", view.sql_file.display()),
                    )
                })?;
        }
        Ok(imported)
    }
}

fn pointer(text: &str) -> String {
    text.replace('~', "~0").replace('/', "~1")
}
