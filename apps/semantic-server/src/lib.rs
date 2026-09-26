//! Local application frontend. Only the Semantic DB engine plans/executes SQL.
pub mod http;
pub mod protocol;

mod server;
pub use server::{ServerArgs, run_with_registry};
