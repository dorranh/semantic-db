//! Deterministic SQL execution with a descriptive relation catalog.

mod allocation;
mod business_calendar;
mod calendar;
mod checked_distinct;
mod checked_mean;
mod checked_snapshot;
mod checked_weighted;
pub use allocation::{
    semantic_allocation_floor_v1, semantic_allocation_remainder_v1, semantic_allocation_share_v1,
    semantic_assert_allocation_v1,
};
pub use business_calendar::semantic_local_date_us_v1;
pub use calendar::semantic_utc_month_us_v1;
mod checked_sum;
mod compiler_functions;
mod conversion;
pub use checked_distinct::{EXACT_DISTINCT_MAX_IDENTITIES_V1, semantic_exact_count_i64_v1};
pub use checked_mean::semantic_mean_i64_v1;
pub use checked_snapshot::semantic_snapshot_balance_i64_v1;
pub use checked_sum::semantic_sum_v1;
pub use checked_weighted::semantic_weighted_mean_i64_v1;
mod contracts;
pub use compiler_functions::{semantic_assert_single_v1, semantic_ratio_i64_v1};
pub use conversion::semantic_scale_i64_v1;
mod deferred;
pub use deferred::{DeferredBackend, DeferredOptions, DeferredProviderReport};
mod execution_profile;
pub use execution_profile::*;
mod reads;
mod writes;
pub use contracts::*;
pub use reads::*;
pub use writes::{PreparedWrite, WriteDescription, is_write_explanation, is_write_statement};
mod federation;
mod materialization;
mod parameters;
mod query;
mod rate_guard;
pub use parameters::ReadDescription;
pub use query::{PreparedQuery, QueryExecution};
pub use rate_guard::semantic_assert_exactly_one_v1;
pub use semantic_materialization::{CacheOptions, MaterializationManager, MaterializationPolicy};
pub use semantic_runtime::SourceDescriptor;
pub use semantic_runtime::staging::{StagedInput, StagingOptions};
pub use semantic_runtime::{QueryContext, QueryOptions};

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::{Arc, Mutex},
};

use datafusion::{
    dataframe::DataFrame,
    error::DataFusionError,
    execution::context::SQLOptions,
    prelude::{CsvReadOptions, SessionConfig, SessionContext},
    sql::{parser::Statement, sqlparser::ast::Statement as SqlStatement},
};
pub use semantic_catalog::Relation;
use semantic_catalog::{Catalog, CatalogError, RelationKind};
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
    CatalogPublication(#[from] semantic_catalog::PublicationError),
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

// The cache is deliberately local to an Engine. A DataFrame contains an
// unexecuted logical plan and its SessionState; a fresh physical plan is made
// when callers collect it. Keep retention small because a plan can reference
// a wide provider schema even when the SQL projects only a few fields.
const GENERATED_PLAN_CACHE_ENTRIES: usize = 8;
const GENERATED_PLAN_CACHE_SQL_BYTES: usize = 256 * 1024;
const GENERATED_PLAN_CACHE_MAX_SQL_BYTES: usize = 64 * 1024;

struct GeneratedPlanEntry {
    generation: u64,
    sql: String,
    frame: DataFrame,
}

#[derive(Default)]
struct GeneratedPlanCache {
    // Oldest entry first. A linear lookup is cheaper than another index at
    // this deliberately small bound.
    entries: Vec<GeneratedPlanEntry>,
    sql_bytes: usize,
}

impl GeneratedPlanCache {
    fn get(&mut self, generation: u64, sql: &str) -> Option<DataFrame> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.generation == generation && entry.sql == sql)?;
        let entry = self.entries.remove(index);
        let frame = entry.frame.clone();
        self.entries.push(entry);
        Some(frame)
    }

    fn insert(&mut self, generation: u64, sql: &str, frame: &DataFrame) {
        if sql.len() > GENERATED_PLAN_CACHE_MAX_SQL_BYTES {
            return;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.generation == generation && entry.sql == sql)
        {
            let entry = self.entries.remove(index);
            self.entries.push(entry);
            return;
        }
        while self.entries.len() >= GENERATED_PLAN_CACHE_ENTRIES
            || self.sql_bytes + sql.len() > GENERATED_PLAN_CACHE_SQL_BYTES
        {
            let oldest = self.entries.remove(0);
            self.sql_bytes -= oldest.sql.len();
        }
        self.sql_bytes += sql.len();
        self.entries.push(GeneratedPlanEntry {
            generation,
            sql: sql.to_owned(),
            frame: frame.clone(),
        });
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.sql_bytes = 0;
    }
}

/// One in-memory session. Registration requires exclusive access so metadata and
/// executable providers are updated together. No catalog persistence yet.
pub struct Engine {
    context: SessionContext,
    catalog: Catalog,
    providers: BTreeMap<String, Arc<dyn TableProvider>>,
    descriptors: BTreeMap<String, SourceDescriptor>,
    materialization_policies: BTreeMap<String, MaterializationPolicy>,
    materializations: Option<Arc<MaterializationManager>>,
    identity: String,
    query_options: QueryOptions,
    read_bindings: BTreeMap<String, ReadBinding>,
    write_bindings: BTreeMap<String, WriteBinding>,
    binding_generation: u64,
    generated_plan_cache: Mutex<GeneratedPlanCache>,
    write_domains: BTreeMap<String, String>,
    resource_identities: BTreeMap<String, ResourceIdentity>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::with_memory_limit(512 * 1024 * 1024).expect("default runtime")
    }
    pub fn with_memory_limit(bytes: usize) -> Result<Self> {
        if bytes == 0 {
            return Err(semantic_runtime::failure("memory limit must be positive").into());
        }
        let runtime = datafusion::execution::runtime_env::RuntimeEnvBuilder::new()
            .with_memory_limit(bytes, 1.0)
            .build_arc()?;
        let config = SessionConfig::new().with_information_schema(true);
        let state = datafusion::execution::session_state::SessionStateBuilder::new()
            .with_config(config)
            .with_runtime_env(runtime)
            .with_default_features()
            .with_optimizer_rules(federation::optimizer_rules())
            .with_query_planner(Arc::new(datafusion_federation::FederatedQueryPlanner::new()))
            .build();
        let context = SessionContext::new_with_state(state);
        context.register_udf((*semantic_ratio_i64_v1()).clone());
        context.register_udf((*semantic_assert_single_v1()).clone());
        context.register_udf((*semantic_assert_exactly_one_v1()).clone());
        context.register_udf((*semantic_scale_i64_v1()).clone());
        context.register_udf((*semantic_allocation_floor_v1()).clone());
        context.register_udf((*semantic_allocation_remainder_v1()).clone());
        context.register_udf((*semantic_assert_allocation_v1()).clone());
        context.register_udf((*semantic_allocation_share_v1()).clone());
        context.register_udf((*semantic_utc_month_us_v1()).clone());
        context.register_udf((*semantic_local_date_us_v1()).clone());
        context.register_udaf((*semantic_sum_v1()).clone());
        context.register_udaf((*semantic_mean_i64_v1()).clone());
        context.register_udaf((*semantic_weighted_mean_i64_v1()).clone());
        context.register_udaf((*semantic_exact_count_i64_v1()).clone());
        context.register_udaf((*semantic_snapshot_balance_i64_v1()).clone());
        Ok(Self {
            context,
            catalog: Catalog::default(),
            providers: BTreeMap::new(),
            descriptors: BTreeMap::new(),
            materialization_policies: BTreeMap::new(),
            materializations: None,
            identity: semantic_runtime::unique_id(),
            query_options: QueryOptions::default(),
            read_bindings: BTreeMap::new(),
            write_bindings: BTreeMap::new(),
            binding_generation: 0,
            generated_plan_cache: Mutex::new(GeneratedPlanCache::default()),
            write_domains: BTreeMap::new(),
            resource_identities: BTreeMap::new(),
        })
    }

    pub fn has_write_bindings(&self) -> bool {
        !self.write_bindings.is_empty()
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
        let order = engine.registration_order(&catalog, true)?;
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
        engine
            .catalog
            .validate(&semantic_catalog::PublicationLimits::default())?;
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

    /// Register a preplanned canonical direct-projection view. The caller
    /// supplies the plan; the engine still checks its SQL dependencies and
    /// Arrow output contract before making it visible.
    pub fn register_view_projection(
        &mut self,
        relation: Relation,
        provider: Arc<dyn TableProvider>,
    ) -> Result<()> {
        let RelationKind::View { sql, dependencies } = &relation.kind else {
            return Err(EngineError::InvalidViewDefinition);
        };
        let lineage = relation
            .semantics
            .as_ref()
            .and_then(|semantics| semantics.view_lineage.as_ref())
            .ok_or(EngineError::InvalidViewDefinition)?;
        if dependencies.len() != 1
            || dependencies[0] != lineage.source.id
            || lineage.canonical_sql(&relation.schema).as_deref() != Some(sql.as_str())
            || self
                .catalog
                .snapshot()
                .relation(&lineage.source.id)
                .is_none_or(|source| source.reference() != &lineage.source)
        {
            return Err(EngineError::InvalidViewDefinition);
        }
        if self.validate_view_definition(&relation.name, sql)? != *dependencies {
            return Err(EngineError::InvalidViewDefinition);
        }
        for dependency in dependencies {
            if self.catalog.relation(dependency).is_none() {
                return Err(EngineError::MissingDependency {
                    view: relation.name.clone(),
                    dependency: dependency.clone(),
                });
            }
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
            .register_table(relation.name.as_str(), provider.clone())?;
        self.providers.insert(relation.name.clone(), provider);
        self.catalog.register(relation)?;
        self.binding_generation += 1;
        self.generated_plan_cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clear();
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
        self.create_view_with_description(name, sql, None).await
    }

    /// Register an authored description alongside the inferred view schema.
    pub async fn create_view_with_description(
        &mut self,
        name: &str,
        sql: &str,
        description: Option<&str>,
    ) -> Result<()> {
        let dependencies = self.validate_view_definition(name, sql)?;
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
        relation.description = description.map(str::to_owned);
        relation.kind = RelationKind::View {
            sql: sql.into(),
            dependencies,
        };
        self.register_provider(relation, frame.into_view())
    }

    /// Check a new view's name and query syntax and return direct dependencies.
    /// Does not resolve dependencies or columns, construct providers, or read rows.
    /// Project loaders can use this before connecting to sources.
    pub fn validate_view_definition(&self, name: &str, sql: &str) -> Result<Vec<String>> {
        self.check_new_name(name)?;
        self.view_dependencies(sql)
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

    fn registration_order(&self, catalog: &Catalog, validate_new: bool) -> Result<Vec<String>> {
        let mut pending = BTreeMap::new();
        for relation in catalog.relations() {
            if validate_new {
                self.check_new_name(&relation.name)?;
            }
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
        let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut indegree = BTreeMap::new();
        for (name, dependencies) in &pending {
            indegree.insert(name.clone(), dependencies.len());
            for dependency in dependencies {
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(name.clone());
            }
        }
        let mut ready: BTreeSet<_> = indegree
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(name, _)| name.clone())
            .collect();
        let mut order = Vec::new();
        while let Some(name) = ready.pop_first() {
            pending.remove(&name);
            if let Some(children) = dependents.get(&name) {
                for child in children {
                    let count = indegree.get_mut(child).expect("known dependent");
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(child.clone());
                    }
                }
            }
            order.push(name);
        }
        if !pending.is_empty() {
            return Err(EngineError::CyclicViews(pending.into_keys().collect()));
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
        // Parameter values and their inferred types are supplied later. Until
        // they are part of a cache key, never retain plans containing a `$`.
        // This also conservatively bypasses strings/identifiers containing `$`.
        let cacheable = !sql.contains('$') && sql.len() <= GENERATED_PLAN_CACHE_MAX_SQL_BYTES;
        if cacheable
            && let Some(frame) = self
                .generated_plan_cache
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .get(self.binding_generation, sql)
        {
            return Ok(frame);
        }
        let frame = self.plan_sql(sql).await?;
        if cacheable {
            self.generated_plan_cache
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .insert(self.binding_generation, sql, &frame);
        }
        Ok(frame)
    }

    /// Convenience for small interactive results. Large consumers should call
    /// `execute` and consume QueryExecution::stream instead of collecting.
    pub async fn query(&self, sql: &str) -> Result<Vec<RecordBatch>> {
        self.execute(sql, self.query_options.clone())
            .await?
            .collect()
            .await
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
