use datafusion::error::Result;
use semantic_runtime::failure;
use serde::{Deserialize, Serialize};

/// Limits are per remote execution; engine query budgets remain cumulative.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PostgresOptions {
    pub filter_pushdown: bool,
    pub federation: bool,
    /// Explicit exception to the verify-full production transport policy.
    pub allow_insecure_transport: bool,
    pub connect_timeout_ms: u64,
    pub acquire_timeout_ms: u64,
    pub statement_timeout_ms: u64,
    pub idle_transaction_timeout_ms: u64,
    pub query_timeout_ms: u64,
    pub max_batch_bytes: usize,
    pub max_session_bytes: usize,
    pub max_in_list: usize,
}
impl Default for PostgresOptions {
    fn default() -> Self {
        Self {
            filter_pushdown: true,
            federation: true,
            allow_insecure_transport: false,
            connect_timeout_ms: 10_000,
            acquire_timeout_ms: 10_000,
            statement_timeout_ms: 30_000,
            idle_transaction_timeout_ms: 30_000,
            query_timeout_ms: 30_000,
            max_batch_bytes: 8 * 1024 * 1024,
            max_session_bytes: 64 * 1024 * 1024,
            max_in_list: 256,
        }
    }
}
impl PostgresOptions {
    pub fn validate(&self) -> Result<()> {
        if [
            self.connect_timeout_ms,
            self.acquire_timeout_ms,
            self.statement_timeout_ms,
            self.idle_transaction_timeout_ms,
            self.query_timeout_ms,
        ]
        .iter()
        .any(|n| *n == 0 || *n > 86_400_000)
            || self.max_batch_bytes < 1024
            || self.max_session_bytes < self.max_batch_bytes
            || self.max_in_list == 0
            || self.max_in_list > 4096
        {
            return Err(failure("invalid Postgres execution limits"));
        }
        Ok(())
    }
}
