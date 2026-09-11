//! Embed Semantic DB with an application-owned catalog and relation backend.
//!
//! Start with [`Engine::from_catalog`], [`Relation`], and [`RelationBackend`].
//! The `compiler` feature (enabled by default) adds natural-language compilation;
//! SQL-only consumers can disable default features. No environment variables are
//! read by the library: applications configure backends and model providers.
//!
//! DataFusion and Arrow are re-exported to keep connector types on the same
//! dependency version as the engine. They remain part of the public API.

pub use datafusion;
pub use datafusion::arrow;
pub use semantic_catalog as catalog;
#[cfg(feature = "compiler")]
pub use semantic_compiler as compiler;
pub use semantic_engine as engine;
pub use semantic_plan as plan;

pub use semantic_catalog::{Catalog, Relation, RelationKind};
#[cfg(feature = "compiler")]
pub use semantic_compiler::{Compiler, CompilerError, GroundingOutcome};
pub use semantic_engine::{Engine, EngineError, RelationBackend, TableProvider};
