use crate::{Engine, EngineError, RelationKind, Result};
use datafusion::{
    catalog::TableProvider,
    execution::context::SQLOptions,
    prelude::{SessionConfig, SessionContext},
};
use semantic_materialization::{
    CacheOptions, Manifest, MaterializationManager, MaterializationPolicy,
};
use semantic_runtime::{QueryContext, SourceDescriptor, fingerprint};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

impl Engine {
    pub async fn refresh_materialization(&self, name: &str) -> Result<()> {
        if !self.materialization_policies.contains_key(name) {
            return Err(semantic_runtime::failure("relation has no materialization policy").into());
        }
        let mut options = self.query_options.clone();
        options.bypass_materialization = false;
        options.refresh_materializations = vec![name.to_owned()];
        self.execute(&format!("SELECT * FROM \"{name}\" LIMIT 0"), options)
            .await?
            .collect()
            .await?;
        Ok(())
    }
    pub fn configure_cache(&mut self, options: CacheOptions) -> Result<()> {
        self.materializations = Some(MaterializationManager::new(options)?);
        Ok(())
    }
    pub fn materialization_manager(&self) -> Option<&Arc<MaterializationManager>> {
        self.materializations.as_ref()
    }
    /// Callers must change this descriptor when scope, credentials, schema, or source semantics change.
    pub fn set_source_descriptor(
        &mut self,
        name: &str,
        descriptor: SourceDescriptor,
    ) -> Result<()> {
        if self.catalog.relation(name).is_none() {
            return Err(EngineError::InvalidName(name.to_owned()));
        }
        self.descriptors.insert(name.to_owned(), descriptor);
        Ok(())
    }
    pub fn materialize(&mut self, name: &str, policy: MaterializationPolicy) -> Result<()> {
        policy.validate()?;
        if self.materializations.is_none() {
            return Err(semantic_runtime::failure(
                "configure cache storage before materialization",
            )
            .into());
        }
        if self.catalog.relation(name).is_none() {
            return Err(EngineError::InvalidName(name.to_owned()));
        }
        self.materialization_policies
            .insert(name.to_owned(), policy);
        Ok(())
    }
    pub(crate) async fn execution_frame(
        &self,
        sql: &str,
        query: &Arc<QueryContext>,
    ) -> Result<datafusion::dataframe::DataFrame> {
        let Some(manager) = self
            .materializations
            .as_ref()
            .filter(|_| !query.options.bypass_materialization)
        else {
            return self.plan_sql(sql).await;
        };
        // Validate first. EXPLAIN must neither fetch rows nor populate the cache.
        let validated = self.plan_sql(sql).await?;
        if matches!(
            validated.logical_plan(),
            datafusion::logical_expr::LogicalPlan::Explain(_)
        ) {
            return Ok(validated);
        }
        let state = self.context.state();
        let statement = state.sql_to_statement(sql, &state.config_options().sql_parser.dialect)?;
        let mut needed: BTreeSet<String> = state
            .resolve_table_references(&statement)?
            .into_iter()
            .map(|r| r.table().to_owned())
            .collect();
        loop {
            let before = needed.len();
            for name in needed.clone() {
                if let Some(relation) = self.catalog.relation(&name)
                    && let RelationKind::View { dependencies, .. } = &relation.kind
                {
                    needed.extend(dependencies.iter().cloned());
                }
            }
            if before == needed.len() {
                break;
            }
        }
        // A materialized view's freshness requirement also applies to cached ancestors.
        let mut allowed_ages: BTreeMap<String, u64> = needed
            .iter()
            .filter_map(|name| {
                self.materialization_policies
                    .get(name)
                    .map(|policy| (name.clone(), policy.max_age_seconds))
            })
            .collect();
        loop {
            let previous = allowed_ages.clone();
            for (name, age) in &previous {
                if let Some(relation) = self.catalog.relation(name)
                    && let RelationKind::View { dependencies, .. } = &relation.kind
                {
                    for dependency in dependencies {
                        allowed_ages
                            .entry(dependency.clone())
                            .and_modify(|value| *value = (*value).min(*age))
                            .or_insert(*age);
                    }
                }
            }
            if previous == allowed_ages {
                break;
            }
        }
        let session = SessionContext::new_with_state(
            datafusion::execution::session_state::SessionStateBuilder::new()
                .with_config(
                    SessionConfig::new()
                        .with_information_schema(true)
                        .with_extension(query.clone()),
                )
                .with_runtime_env(self.context.runtime_env())
                .with_default_features()
                .with_optimizer_rules(crate::federation::optimizer_rules())
                .with_query_planner(Arc::new(datafusion_federation::FederatedQueryPlanner::new()))
                .build(),
        );
        let mut generations: BTreeMap<String, Manifest> = BTreeMap::new();
        let mut revisions: BTreeMap<String, String> = BTreeMap::new();
        let mut acquired: BTreeMap<String, u64> = BTreeMap::new();
        for name in self.registration_order(&self.catalog, false)? {
            let relation = self.catalog.relation(&name).unwrap();
            let (provider, dependency_names): (Arc<dyn TableProvider>, Vec<String>) =
                match &relation.kind {
                    RelationKind::Base { .. } => (self.providers[&name].clone(), vec![]),
                    RelationKind::View { sql, dependencies } => {
                        (session.sql(sql).await?.into_view(), dependencies.clone())
                    }
                };
            session.register_table(name.as_str(), provider)?;
            let dependency_revision = dependency_names
                .iter()
                .map(|name| {
                    generations
                        .get(name)
                        .map(|m| m.generation.clone())
                        .unwrap_or_else(|| revisions.get(name).cloned().unwrap_or_default())
                })
                .collect::<Vec<_>>()
                .join(":");
            let mut descriptor =
                self.descriptors
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| SourceDescriptor {
                        scope: if dependency_names.is_empty() {
                            format!("{}:{name}", self.identity)
                        } else {
                            name.clone()
                        },
                        schema_revision: fingerprint(
                            &[format!("{:?}", relation.schema).as_bytes()],
                        ),
                        authorization_scope: if dependency_names.is_empty() {
                            self.identity.clone()
                        } else {
                            fingerprint(&[dependency_revision.as_bytes()])
                        },
                        revision: fingerprint(&[format!("{:?}", relation.kind).as_bytes()]),
                    });
            descriptor.revision = fingerprint(&[
                descriptor.revision.as_bytes(),
                dependency_revision.as_bytes(),
            ]);
            revisions.insert(name.clone(), descriptor.cache_key());
            let oldest = dependency_names
                .iter()
                .filter_map(|name| acquired.get(name).copied())
                .min();
            acquired.insert(
                name.clone(),
                oldest.unwrap_or_else(semantic_materialization::now_ms),
            );
            if let Some(policy) = self
                .materialization_policies
                .get(&name)
                .filter(|_| needed.contains(&name) && !self.cache_excluded(&name))
            {
                let mut policy = policy.clone();
                if let Some(age) = query.options.max_cache_age_ms {
                    policy.max_age_seconds = policy.max_age_seconds.min(age.div_ceil(1000).max(1));
                }
                if let Some(age) = allowed_ages.get(&name) {
                    policy.max_age_seconds = policy.max_age_seconds.min(*age);
                }
                if query.options.refresh_materializations.contains(&name) {
                    manager.invalidate(&descriptor.cache_key(), query).await?;
                }
                let frame = session.sql(&format!("SELECT * FROM \"{name}\"")).await?;
                let context = query.clone();
                let materialized = manager
                    .resolve(
                        &descriptor,
                        &policy,
                        relation.schema.clone(),
                        query,
                        oldest,
                        move || {
                            Box::pin(async move {
                                let physical = frame.create_physical_plan().await?;
                                let task = frame.task_ctx();
                                let config = task.session_config().clone().with_extension(context);
                                datafusion::physical_plan::execute_stream(
                                    physical,
                                    Arc::new(task.with_session_config(config)),
                                )
                            })
                        },
                    )
                    .await?;
                query.record_cache(semantic_runtime::CacheObservation {
                    relation: name.clone(),
                    generation: materialized.manifest.generation.clone(),
                    published_at_ms: materialized.manifest.published_at_ms,
                    age_ms: semantic_materialization::now_ms()
                        .saturating_sub(materialized.manifest.published_at_ms),
                });
                session.deregister_table(name.as_str())?;
                session.register_table(name.as_str(), materialized.provider)?;
                acquired.insert(name.clone(), materialized.manifest.acquired_at_ms);
                generations.insert(name, materialized.manifest);
            }
        }
        Ok(session
            .sql_with_options(
                sql,
                SQLOptions::new()
                    .with_allow_ddl(false)
                    .with_allow_dml(false)
                    .with_allow_statements(false),
            )
            .await?)
    }
}
