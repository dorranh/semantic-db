//! Connector-issued evidence and normalized operations. Descriptive catalog keys
//! never grant write capabilities.
use crate::{Result, TableProvider};
use datafusion::{
    arrow::datatypes::{DataType, SchemaRef},
    common::ScalarValue,
    sql::sqlparser::ast,
};
use futures::future::BoxFuture;
use semantic_runtime::{
    QueryContext, QueryOptions,
    staging::{StagedInput, StagingOptions},
};
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitReceipt {
    pub domain: String,
    pub evidence: String,
    pub resources: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WriteOutcome {
    Rejected,
    Aborted,
    Committed,
    OutcomeUnknown,
    PartialFailure,
    AppliedInTransaction,
}
#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub outcome: WriteOutcome,
    pub operation_id: String,
    pub boundary: String,
    pub atomic: bool,
    pub idempotent: bool,
    pub receipt: Option<CommitReceipt>,
    pub affected_rows: Option<u64>,
    pub code: Option<String>,
    pub message: Option<String>,
    pub external_observations: Vec<String>,
}
impl WriteResult {
    pub fn success(&self) -> bool {
        matches!(
            self.outcome,
            WriteOutcome::Committed | WriteOutcome::AppliedInTransaction
        )
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Atomicity {
    Required,
    BestEffort,
}
#[derive(Debug, Clone)]
pub struct WriteOptions {
    pub atomicity: Atomicity,
    pub query: QueryOptions,
    pub staging: StagingOptions,
}
impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            atomicity: Atomicity::Required,
            query: QueryOptions::default(),
            staging: StagingOptions::default(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Isolation {
    RepeatableRead,
    Serializable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalReads {
    Reject,
    AllowObserved,
}
#[derive(Debug, Clone)]
pub struct TransactionOptions {
    pub isolation: Isolation,
    pub external_reads: ExternalReads,
    pub lifetime: Duration,
}
impl Default for TransactionOptions {
    fn default() -> Self {
        Self {
            isolation: Isolation::Serializable,
            external_reads: ExternalReads::Reject,
            lifetime: Duration::from_secs(300),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ReadConsistency {
    Observed,
    Snapshot,
}
#[derive(Debug, Clone)]
pub enum ReadCache {
    Configured,
    Bypass,
    MaxAge(Duration),
}
#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub query: QueryOptions,
    pub consistency: ReadConsistency,
    pub cache: ReadCache,
    pub after_commits: Vec<CommitReceipt>,
}
impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            query: QueryOptions::default(),
            consistency: ReadConsistency::Observed,
            cache: ReadCache::Configured,
            after_commits: vec![],
        }
    }
}
#[derive(Debug, Clone)]
pub struct ReadSessionOptions {
    pub lifetime: Duration,
    pub external_reads: ExternalReads,
    pub after_commits: Vec<CommitReceipt>,
}
impl Default for ReadSessionOptions {
    fn default() -> Self {
        Self {
            lifetime: Duration::from_secs(300),
            external_reads: ExternalReads::Reject,
            after_commits: vec![],
        }
    }
}
#[derive(Debug, Clone)]
pub struct TargetInspection {
    pub physical_namespace: String,
    pub schema: SchemaRef,
    pub revision: String,
    pub resource: String,
    pub domain: String,
    pub unique_keys: Vec<Vec<String>>,
    pub checked_eligible: bool,
    pub supported_operations: Vec<String>,
    pub atomic_writes: bool,
}
/// Connector-issued physical identity. Different namespaces identify disjoint
/// storage kinds; matching display names or connection URLs are not evidence.
#[derive(Debug, Clone)]
pub struct ResourceIdentity {
    pub namespace: String,
    pub domain: Option<String>,
    pub resource: Option<String>,
}
#[derive(Clone)]
pub struct WriteBinding {
    pub connection: Arc<dyn WriteConnection>,
    pub target: String,
    pub columns: BTreeMap<String, String>,
}
#[derive(Clone)]
pub struct ReadBinding {
    pub connection: Arc<dyn ReadConnection>,
    pub resource: String,
    pub columns: BTreeMap<String, String>,
}
#[derive(Debug, Clone)]
pub enum ValueExpr {
    Column(String),
    Parameter(usize),
    Literal(ScalarValue),
    Binary(Box<Self>, ast::BinaryOperator, Box<Self>),
    Unary(ast::UnaryOperator, Box<Self>),
    Cast(Box<Self>, DataType),
    IsNull(Box<Self>, bool),
}
#[derive(Debug, Clone)]
pub enum Mutation {
    Insert {
        columns: Vec<String>,
    },
    Update {
        assignments: Vec<(String, ValueExpr)>,
        predicate: Option<ValueExpr>,
    },
    Delete {
        predicate: Option<ValueExpr>,
    },
    Merge {
        keys: Vec<(String, String)>,
        updates: Vec<(String, String)>,
        inserts: Vec<(String, String)>,
    },
}
impl Mutation {
    pub fn command(&self) -> &'static str {
        match self {
            Self::Insert { .. } => "INSERT",
            Self::Update { .. } => "UPDATE",
            Self::Delete { .. } => "DELETE",
            Self::Merge { .. } => "MERGE",
        }
    }
}
#[derive(Debug, Clone)]
pub struct MutationPlan {
    pub target: String,
    pub revision: String,
    pub mutation: Mutation,
    pub checked: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct WriteExplanation {
    pub operation: String,
    pub target: String,
    pub boundary: String,
    pub require_idempotent: bool,
    pub mapped_keys: Vec<(String, String)>,
    pub atomic_supported: bool,
    pub source_observations: Vec<String>,
    pub static_checks: Vec<String>,
    pub runtime_checks: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct TableDefinition {
    pub name: String,
    pub target: BTreeMap<String, String>,
    pub schema: SchemaRef,
    pub primary_key: Vec<String>,
}
pub struct CreatedTable {
    pub provider: Arc<dyn TableProvider>,
    pub read: ReadBinding,
    pub write: WriteBinding,
}

pub trait WriteConnection: Send + Sync {
    fn inspect_target<'a>(&'a self, target: &'a str) -> BoxFuture<'a, Result<TargetInspection>>;
    fn validate_operation<'a>(&'a self, plan: &'a MutationPlan) -> BoxFuture<'a, Result<()>>;
    fn apply<'a>(
        &'a self,
        plan: &'a MutationPlan,
        input: &'a StagedInput,
        parameters: &'a [ScalarValue],
        context: Arc<QueryContext>,
    ) -> BoxFuture<'a, Result<WriteResult>>;
    fn begin<'a>(
        &'a self,
        options: TransactionOptions,
    ) -> BoxFuture<'a, Result<Arc<dyn ConnectorSession>>>;
    fn create_table<'a>(
        &'a self,
        _definition: &'a TableDefinition,
    ) -> BoxFuture<'a, Result<CreatedTable>> {
        Box::pin(async { Err(semantic_runtime::failure("table creation unsupported").into()) })
    }
}
pub trait ReadConnection: Send + Sync {
    fn physical_namespace(&self) -> Option<&'static str> {
        None
    }
    fn domain(&self) -> String;
    fn read_provider<'a>(
        &'a self,
        resource: &'a str,
        receipts: &'a [CommitReceipt],
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>>;
    fn open_read_session<'a>(
        &'a self,
        options: ReadSessionOptions,
    ) -> BoxFuture<'a, Result<Arc<dyn ConnectorSession>>>;
}
/// A pinned native session. Providers must serialize native access themselves.
/// Dropping the last owner must dispose of any unclean connection.
pub trait ConnectorSession: Send + Sync {
    fn read_your_writes(&self) -> bool {
        false
    }
    fn inspect_target<'a>(&'a self, _target: &'a str) -> BoxFuture<'a, Result<TargetInspection>> {
        Box::pin(async {
            Err(semantic_runtime::failure("session target inspection unsupported").into())
        })
    }
    fn validate_operation<'a>(&'a self, _plan: &'a MutationPlan) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Err(semantic_runtime::failure("session writes unsupported").into()) })
    }

    fn domain(&self) -> String;
    fn snapshot(&self) -> String;
    fn read_provider<'a>(
        &'a self,
        resource: &'a str,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>>;
    fn apply<'a>(
        &'a self,
        plan: &'a MutationPlan,
        input: &'a StagedInput,
        parameters: &'a [ScalarValue],
        context: Arc<QueryContext>,
    ) -> BoxFuture<'a, Result<WriteResult>>;
    fn finish<'a>(&'a self, commit: bool) -> BoxFuture<'a, Result<WriteResult>>;
}
