//! Embed Semantic DB with an application-owned catalog and relation backend.
//!
//! With `sources`, load the same configured Ossie project as the CLI. For an
//! application-owned catalog, use [`Engine::from_catalog`] and [`RelationBackend`].
//! The `compiler` feature (enabled by default) adds natural-language compilation;
//! SQL-only consumers can disable default features. No environment variables are
//! read by the library: applications configure backends and model providers.
//! The opt-in `ossie` feature imports pinned Ossie models with explicit source
//! bindings; see the `ossie_wells` example.
//!
//! DataFusion and Arrow are re-exported to keep connector types on the same
//! dependency version as the engine. They remain part of the public API.

pub use datafusion;
pub use datafusion::arrow;
pub use semantic_catalog as catalog;
#[cfg(feature = "clickhouse")]
pub use semantic_clickhouse as clickhouse;
#[cfg(feature = "compiler")]
pub use semantic_compiler as compiler;
pub use semantic_engine as engine;
#[cfg(feature = "github")]
pub use semantic_github as github;
#[cfg(feature = "ossie")]
pub use semantic_ossie as ossie;
pub use semantic_plan as plan;
#[cfg(feature = "sources")]
pub use semantic_sources as sources;

pub use semantic_catalog::{Catalog, Relation, RelationKind};
#[cfg(feature = "compiler")]
pub use semantic_compiler::{Compiler, CompilerError, GroundingOutcome};
pub use semantic_engine::{Engine, EngineError, RelationBackend, TableProvider};

pub use semantic_engine::{
    CacheOptions, MaterializationManager, MaterializationPolicy, PreparedQuery, QueryContext,
    QueryExecution, QueryOptions, SourceDescriptor,
};
