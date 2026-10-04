//! Standalone dataset validation and acceptance execution. Dataset semantics live in artifacts.
mod compare;
mod contract;
mod lifecycle;
mod public;
mod runner;
pub use compare::{compare, result_from_batches};
pub use contract::*;
pub use public::PublicCheck;
pub use runner::*;
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
