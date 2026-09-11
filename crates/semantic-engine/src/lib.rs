//! Deterministic SQL execution with a descriptive relation catalog.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::Arc,
};

use datafusion::{
    dataframe::DataFrame,
    error::DataFusionError,
    execution::context::SQLOptions,
    prelude::{CsvReadOptions, SessionConfig, SessionContext},
    sql::{parser::Statement, sqlparser::ast::Statement as SqlStatement},
};
use semantic_catalog::{Catalog, CatalogError, Relation, RelationKind};
use thiserror::Error;

pub use datafusion::arrow::record_batch::RecordBatch;
pub use datafusion::arrow::util::pretty::pretty_format_batches;
pub use datafusion::catalog::TableProvider;

/// Resolve a base relation to a standard DataFusion provider. Implementations
/// own routing, credentials, and connection pools. Resolution happens once at
/// engine construction; providers should defer row reads to execution.
///
/// SQL views are planned by the engine and never passed to this backend.
pub trait RelationBackend: Send + Sync {
    fn resolve(
        &self,
        relation: &Relation,
    ) -> impl Future<Output = datafusion::error::Result<Arc<dyn TableProvider>>> + Send;
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    DataFusion(#[from] DataFusionError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("invalid relation name {0:?}; use a lowercase SQL identifier (a-z, 0-9, _)")]
    InvalidName(String),
    #[error("a view definition must be a SELECT, WITH, or VALUES query")]
    InvalidViewDefinition,
    #[error("generated SQL must be a SELECT, WITH, or VALUES query")]
    InvalidGeneratedQuery,
    #[error("generated SQL may only reference unqualified registered catalog relations")]
    UnregisteredQueryRelation,
    #[error("backend failed to resolve relation {relation:?}: {source}")]
    Resolution {
        relation: String,
        #[source]
        source: DataFusionError,
    },
    #[error("schema mismatch for relation {relation:?}: expected {expected}, got {actual}")]
    SchemaMismatch {
        relation: String,
        expected: String,
        actual: String,
    },
    #[error("register_table requires a base relation; use a catalog or create_view for {0:?}")]
    ExpectedBaseRelation(String),
    #[error("view {view:?} references missing relation {dependency:?}")]
    MissingDependency { view: String, dependency: String },
    #[error("views may only reference unqualified catalog relations: {0}")]
    InvalidViewReference(String),
    #[error("cyclic view dependencies; blocked relations: {0:?}")]
    CyclicViews(Vec<String>),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// One in-memory session. Registration requires exclusive access so metadata and
/// executable providers are updated together. No catalog persistence yet.
pub struct Engine {
    context: SessionContext,
    catalog: Catalog,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        let config = SessionConfig::new().with_information_schema(true);
        Self {
            context: SessionContext::new_with_config(config),
            catalog: Catalog::default(),
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Load an application-owned catalog snapshot, resolving base providers and
    /// planning views in dependency order. Accepts a `Catalog`, a collection of
    /// `Relation`s, or an iterator projected from another catalog implementation.
    ///
    /// Names, missing dependencies, and cycles are checked before backend calls.
    /// Schemas must match exactly (including Arrow metadata). Stored view lineage
    /// is recomputed from SQL. Failure never exposes a partially loaded engine;
    /// backend I/O already performed cannot be rolled back. Rebuild to refresh
    /// the catalog; this does not promise a transactional snapshot of source data.
    pub async fn from_catalog(
        relations: impl IntoIterator<Item = Relation>,
        backend: &impl RelationBackend,
    ) -> Result<Self> {
        let mut engine = Self::new();
        let catalog = Catalog::from_relations(relations)?;
        let order = engine.registration_order(&catalog)?;
        let mut relations: BTreeMap<_, _> = catalog
            .into_iter()
            .map(|relation| (relation.name.clone(), relation))
            .collect();
        for name in order {
            let relation = relations
                .remove(&name)
                .expect("validated registration order");
            match &relation.kind {
                RelationKind::Base { .. } => {
                    let provider = backend.resolve(&relation).await.map_err(|source| {
                        EngineError::Resolution {
                            relation: name,
                            source,
                        }
                    })?;
                    engine.register_table(relation, provider)?;
                }
                RelationKind::View { sql, .. } => {
                    let dependencies = engine.view_dependencies(sql)?;
                    let provider = engine.plan_sql(sql).await?.into_view();
                    let mut relation = relation;
                    let RelationKind::View {
                        dependencies: stored,
                        ..
                    } = &mut relation.kind
                    else {
                        unreachable!()
                    };
                    *stored = dependencies;
                    engine.register_provider(relation, provider)?;
                }
            }
        }
        Ok(engine)
    }

    /// Register an existing DataFusion provider with its semantic metadata.
    /// Validation fails before either catalog is changed.
    pub fn register_table(
        &mut self,
        relation: Relation,
        provider: Arc<dyn TableProvider>,
    ) -> Result<()> {
        if !matches!(relation.kind, RelationKind::Base { .. }) {
            return Err(EngineError::ExpectedBaseRelation(relation.name));
        }
        self.register_provider(relation, provider)
    }

    fn register_provider(
        &mut self,
        relation: Relation,
        provider: Arc<dyn TableProvider>,
    ) -> Result<()> {
        self.check_new_name(&relation.name)?;
        let actual = provider.schema();
        if relation.schema != actual {
            return Err(EngineError::SchemaMismatch {
                relation: relation.name,
                expected: relation.schema.to_string(),
                actual: actual.to_string(),
            });
        }
        self.context
            .register_table(relation.name.as_str(), provider)?;
        self.catalog.register(relation)?;
        Ok(())
    }

    pub async fn register_csv(&mut self, name: &str, path: &str) -> Result<()> {
        self.check_new_name(name)?;
        let frame = self.context.read_csv(path, CsvReadOptions::new()).await?;
        let provider = frame.into_view();
        self.register_table(Relation::base(name, provider.schema(), path), provider)
    }

    /// Register a composable view without collecting or materializing its rows.
    pub async fn create_view(&mut self, name: &str, sql: &str) -> Result<()> {
        self.check_new_name(name)?;
        let dependencies = self.view_dependencies(sql)?;
        for dependency in &dependencies {
            if self.catalog.relation(dependency).is_none() {
                return Err(EngineError::MissingDependency {
                    view: name.into(),
                    dependency: dependency.clone(),
                });
            }
        }
        let frame = self.plan_sql(sql).await?;
        let mut relation = Relation::view(name, Arc::new(frame.schema().as_arrow().clone()), sql);
        relation.kind = RelationKind::View {
            sql: sql.into(),
            dependencies,
        };
        self.register_provider(relation, frame.into_view())
    }

    fn view_dependencies(&self, sql: &str) -> Result<Vec<String>> {
        let state = self.context.state();
        let statement = state.sql_to_statement(sql, &state.config_options().sql_parser.dialect)?;
        if !matches!(&statement, Statement::Statement(inner) if matches!(inner.as_ref(), SqlStatement::Query(_)))
        {
            return Err(EngineError::InvalidViewDefinition);
        }
        // Collect named references before planning expands views. DataFusion's
        // resolver handles subqueries and excludes locally scoped CTE names.
        let mut dependencies = state
            .resolve_table_references(&statement)?
            .into_iter()
            .map(|reference| {
                if reference.schema().is_some() || reference.catalog().is_some() {
                    Err(EngineError::InvalidViewReference(reference.to_string()))
                } else {
                    Ok(reference.table().to_owned())
                }
            })
            .collect::<Result<Vec<_>>>()?;
        dependencies.sort();
        dependencies.dedup();
        Ok(dependencies)
    }

    fn registration_order(&self, catalog: &Catalog) -> Result<Vec<String>> {
        let mut pending = BTreeMap::new();
        for relation in catalog.relations() {
            self.check_new_name(&relation.name)?;
            let dependencies = match &relation.kind {
                RelationKind::Base { .. } => Vec::new(),
                RelationKind::View { sql, .. } => self.view_dependencies(sql)?,
            };
            for dependency in &dependencies {
                if catalog.relation(dependency).is_none() {
                    return Err(EngineError::MissingDependency {
                        view: relation.name.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
            pending.insert(relation.name.clone(), dependencies);
        }
        let mut ready = BTreeSet::new();
        let mut order = Vec::new();
        while !pending.is_empty() {
            let next = pending
                .iter()
                .find(|(_, dependencies)| dependencies.iter().all(|name| ready.contains(name)))
                .map(|(name, _)| name.clone());
            let Some(name) = next else {
                return Err(EngineError::CyclicViews(pending.into_keys().collect()));
            };
            pending.remove(&name);
            ready.insert(name.clone());
            order.push(name);
        }
        Ok(order)
    }

    /// Parse and resolve SQL without executing it. DDL/DML go through explicit
    /// engine APIs so they cannot bypass descriptive catalog registration.
    pub async fn plan_sql(&self, sql: &str) -> Result<DataFrame> {
        let options = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false)
            .with_allow_statements(false);
        Ok(self.context.sql_with_options(sql, options).await?)
    }

    /// More restrictive entry point for model output: query statements and
    /// registered relations only, excluding internal schemas and external paths.
    /// Planning validates names/types but cannot establish semantic correctness.
    pub async fn plan_generated_sql(&self, sql: &str) -> Result<DataFrame> {
        let state = self.context.state();
        let statement = state.sql_to_statement(sql, &state.config_options().sql_parser.dialect)?;
        if !matches!(&statement, Statement::Statement(inner) if matches!(inner.as_ref(), SqlStatement::Query(_)))
        {
            return Err(EngineError::InvalidGeneratedQuery);
        }
        for reference in state.resolve_table_references(&statement)? {
            if reference.schema().is_some()
                || reference.catalog().is_some()
                || self.catalog.relation(reference.table()).is_none()
            {
                return Err(EngineError::UnregisteredQueryRelation);
            }
        }
        self.plan_sql(sql).await
    }

    /// Convenience for small interactive results. Large consumers should call
    /// `plan_sql` and use DataFrame::execute_stream instead of collecting.
    pub async fn query(&self, sql: &str) -> Result<Vec<RecordBatch>> {
        Ok(self.plan_sql(sql).await?.collect().await?)
    }

    fn check_new_name(&self, name: &str) -> Result<()> {
        let mut chars = name.chars();
        let valid = chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            return Err(EngineError::InvalidName(name.into()));
        }
        if self.catalog.relation(name).is_some() {
            return Err(CatalogError::DuplicateRelation(name.into()).into());
        }
        Ok(())
    }
}
