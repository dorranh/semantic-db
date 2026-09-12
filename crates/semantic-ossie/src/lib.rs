//! Import a pinned subset of Ossie into an executable Semantic DB catalog.
//!
//! Parse first, bind source identifiers to providers owned by the application,
//! then load a selected model. No source string is executed or fetched implicitly.

mod document;

pub use document::{
    DatasetRequirement, Diagnostic, FieldRequirement, ImportError, ModelInspection, OssieDocument,
    SCHEMA_COMMIT, SPEC_VERSION,
};

use datafusion::prelude::{CsvReadOptions, SessionContext};
use semantic_engine::{Engine, TableProvider};
use std::{collections::BTreeMap, sync::Arc};

/// Explicit bindings between Ossie source identifiers and executable tables.
#[derive(Default)]
pub struct SourceBindings {
    pub(crate) providers: BTreeMap<String, Arc<dyn TableProvider>>,
}

impl SourceBindings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reject duplicate bindings; a document must never silently change source.
    pub fn bind(
        &mut self,
        source: impl Into<String>,
        provider: Arc<dyn TableProvider>,
    ) -> Result<(), ImportError> {
        let source = source.into();
        if self.providers.contains_key(&source) {
            return Err(ImportError::diagnostic(
                "duplicate_binding",
                "/bindings",
                format!("duplicate source binding {source:?}"),
            ));
        }
        self.providers.insert(source, provider);
        Ok(())
    }

    /// Convenience binding with the same CSV defaults as Engine::register_csv.
    /// Infers the schema; rows remain lazy until execution.
    pub async fn bind_csv(
        &mut self,
        source: impl Into<String>,
        path: &str,
    ) -> Result<(), ImportError> {
        let source = source.into();
        if self.providers.contains_key(&source) {
            return Err(ImportError::diagnostic(
                "duplicate_binding",
                "/bindings",
                format!("duplicate source binding {source:?}"),
            ));
        }
        let frame = SessionContext::new()
            .read_csv(path, CsvReadOptions::new())
            .await?;
        self.bind(source, frame.into_view())
    }
}

pub struct ImportedCatalog {
    pub engine: Engine,
    /// Non-blocking limitations, such as keys that are declared but not enforced.
    pub warnings: Vec<Diagnostic>,
}
