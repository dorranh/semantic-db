//! Deterministic SQL execution with a descriptive relation catalog.

use std::sync::Arc;

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

    pub async fn register_csv(&mut self, name: &str, path: &str) -> Result<()> {
        self.check_new_name(name)?;
        let frame = self.context.read_csv(path, CsvReadOptions::new()).await?;
        let provider = frame.into_view();
        let relation = Relation {
            name: name.into(),
            schema: provider.schema(),
            kind: RelationKind::Base {
                source: path.into(),
            },
            description: None,
            owner: None,
            grain: None,
        };
        self.context.register_table(name, provider)?;
        self.catalog.register(relation)?;
        Ok(())
    }

    /// Register a composable view without collecting or materializing its rows.
    pub async fn create_view(&mut self, name: &str, sql: &str) -> Result<()> {
        self.check_new_name(name)?;
        let state = self.context.state();
        let statement = state.sql_to_statement(sql, &state.config_options().sql_parser.dialect)?;
        if !matches!(&statement, Statement::Statement(inner) if matches!(inner.as_ref(), SqlStatement::Query(_)))
        {
            return Err(EngineError::InvalidViewDefinition);
        }
        // Collect named references before planning expands views. DataFusion's
        // resolver handles subqueries and excludes locally scoped CTE names.
        let dependencies = state
            .resolve_table_references(&statement)?
            .into_iter()
            .map(|reference| reference.to_string())
            .collect();
        let frame = self.plan_sql(sql).await?;
        let relation = Relation {
            name: name.into(),
            schema: Arc::new(frame.schema().as_arrow().clone()),
            kind: RelationKind::View {
                sql: sql.into(),
                dependencies,
            },
            description: None,
            owner: None,
            grain: None,
        };
        self.context.register_table(name, frame.into_view())?;
        self.catalog.register(relation)?;
        Ok(())
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
