//! Offline project binding backed by demand-driven live source resolution.

use super::{Options, Project, Registry, Result, SecretResolver, SourceConnection, SourceError};
use arrow_schema::SchemaRef;
use datafusion::{
    datasource::MemTable,
    error::Result as DataFusionResult,
    logical_expr::{Expr, col},
    prelude::SessionContext,
};
use semantic_engine::{
    DeferredBackend, DeferredOptions, DeferredProviderReport, Engine, Relation, RelationBackend,
    TableProvider,
};
use semantic_ossie::{Diagnostic, ImportedCatalog, SourceBindings};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;

pub struct DeferredProject {
    pub engine: Engine,
    pub warnings: Vec<Diagnostic>,
    backend: DeferredBackend<ConfiguredBackend>,
}
impl DeferredProject {
    /// Metadata provider work is separate from row execution statistics.
    pub fn provider_report(&self) -> DeferredProviderReport {
        self.backend.report()
    }
}

struct BindingSpec {
    connection: String,
    options: Options,
    recorded_schema: SchemaRef,
    /// Physical source column and projected model name, in output order.
    projection: Option<Vec<(String, String)>>,
}

struct ConfiguredBackend {
    project: Arc<Project>,
    registry: Arc<Registry>,
    secrets: Arc<SecretResolver<'static>>,
    specs: BTreeMap<String, BindingSpec>,
    connections: Mutex<BTreeMap<String, Arc<dyn SourceConnection>>>,
}
impl RelationBackend for ConfiguredBackend {
    async fn resolve(&self, relation: &Relation) -> DataFusionResult<Arc<dyn TableProvider>> {
        let fail = |message: String| semantic_runtime::failure(&message);
        let spec = self.specs.get(&relation.name).ok_or_else(|| {
            fail(format!(
                "no deferred source binding for {:?}",
                relation.name
            ))
        })?;
        let config = &self.project.config.connections[&spec.connection];
        let factory = self
            .registry
            .factory(&config.connector, "/connections")
            .map_err(|error| fail(error.to_string()))?;
        let mut connections = self.connections.lock().await;
        if !connections.contains_key(&spec.connection) {
            let connection = factory
                .connect(&config.options, self.secrets.as_ref())
                .await
                .map_err(|error| fail(error.to_string()))?;
            connections.insert(spec.connection.clone(), connection);
        }
        let connection = connections[&spec.connection].clone();
        drop(connections);
        let resource = connection
            .resource(&spec.options, &self.project.base_dir)
            .await
            .map_err(|error| fail(error.to_string()))?;
        if resource.read.is_some() || resource.write.is_some() {
            return Err(fail(format!(
                "deferred read-only binding {:?} has a read/write contract",
                relation.name
            )));
        }
        if resource.provider.schema().as_ref() != spec.recorded_schema.as_ref() {
            return Err(fail(format!(
                "deferred physical schema drift for {:?}: rebuild the recorded source schema",
                relation.name
            )));
        }
        match &spec.projection {
            None => Ok(resource.provider),
            Some(columns) => {
                let expressions: Vec<Expr> = columns
                    .iter()
                    .map(|(source, name)| {
                        let expression = col(source.as_str());
                        if source == name {
                            expression
                        } else {
                            expression.alias(name.clone())
                        }
                    })
                    .collect();
                let projected = SessionContext::new()
                    .read_table(resource.provider)?
                    .select(expressions)?
                    .into_view();
                Ok(projected)
            }
        }
    }
}

pub(super) async fn load(
    project: Project,
    registry: Arc<Registry>,
    secrets: Arc<SecretResolver<'static>>,
    options: DeferredOptions,
) -> Result<DeferredProject> {
    let inspection = project.inspect_project(&registry)?;
    for (name, binding) in project
        .config
        .sources
        .iter()
        .chain(project.config.app_tables.iter())
    {
        if binding.materialization.is_some() {
            return Err(SourceError::configuration(
                "deferred_read_only",
                format!("/sources/{}/materialization", super::pointer(name)),
                "materialization is outside the deferred read-only profile",
            ));
        }
    }
    for (name, view) in &project.config.views {
        if view.materialization.is_some() {
            return Err(SourceError::configuration(
                "deferred_read_only",
                format!("/views/{}/materialization", super::pointer(name)),
                "materialization is outside the deferred read-only profile",
            ));
        }
    }

    let mut bindings = SourceBindings::new();
    let mut source_schemas = BTreeMap::new();
    for dataset in &inspection.model.datasets {
        if source_schemas.contains_key(&dataset.source) {
            continue;
        }
        let binding = &project.config.sources[&dataset.source];
        let schema = recorded_schema(binding.recorded_schema.as_ref(), "sources", &dataset.source)?;
        bindings.bind(dataset.source.clone(), schema_only(schema.clone())?)?;
        source_schemas.insert(dataset.source.clone(), schema);
    }
    let mut imported = match &project.document {
        Some(document) => document.load(project.config.model.as_deref(), &bindings)?,
        None => ImportedCatalog {
            engine: Engine::new(),
            warnings: vec![],
        },
    };
    let mut specs = BTreeMap::new();
    for dataset in &inspection.model.datasets {
        let binding = &project.config.sources[&dataset.source];
        let connection = &project.config.connections[&binding.connection];
        let options = registry
            .factory(&connection.connector, "/connections")?
            .prepare_source(&binding.options, &inspection.model, &dataset.source)?;
        specs.insert(
            dataset.name.clone(),
            BindingSpec {
                connection: binding.connection.clone(),
                options,
                recorded_schema: source_schemas[&dataset.source].clone(),
                projection: Some(
                    dataset
                        .fields
                        .iter()
                        .map(|field| (field.source_column.clone(), field.name.clone()))
                        .collect(),
                ),
            },
        );
    }
    for (name, binding) in &project.config.app_tables {
        let schema = recorded_schema(binding.recorded_schema.as_ref(), "app_tables", name)?;
        imported.engine.register_table(
            Relation::base(name, schema.clone(), "application"),
            schema_only(schema.clone())?,
        )?;
        specs.insert(
            name.clone(),
            BindingSpec {
                connection: binding.connection.clone(),
                options: binding.options.clone(),
                recorded_schema: schema,
                projection: None,
            },
        );
    }
    for view in inspection.views {
        imported
            .engine
            .create_view_with_description(&view.name, &view.sql, view.description.as_deref())
            .await
            .map_err(|error| {
                SourceError::configuration(
                    "view_plan",
                    format!("/views/{}/sql_file", super::pointer(&view.name)),
                    error.to_string(),
                )
            })?;
    }
    let relations: Vec<_> = imported.engine.catalog().relations().cloned().collect();
    let backend = DeferredBackend::new(
        Arc::new(ConfiguredBackend {
            project: Arc::new(project),
            registry,
            secrets,
            specs,
            connections: Mutex::new(BTreeMap::new()),
        }),
        options,
    )?;
    let engine = Engine::from_catalog(relations, &backend).await?;
    Ok(DeferredProject {
        engine,
        warnings: imported.warnings,
        backend,
    })
}

fn recorded_schema(
    schema: Option<&arrow_schema::Schema>,
    kind: &str,
    name: &str,
) -> Result<SchemaRef> {
    schema.cloned().map(Arc::new).ok_or_else(|| {
        SourceError::configuration(
            "missing_recorded_schema",
            format!("/{kind}/{}/recorded_schema", super::pointer(name)),
            "deferred loading requires a complete recorded physical Arrow schema",
        )
    })
}

fn schema_only(schema: SchemaRef) -> Result<Arc<dyn TableProvider>> {
    Ok(Arc::new(MemTable::try_new(schema, vec![vec![]])?))
}
