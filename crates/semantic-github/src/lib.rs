//! Experimental, read-only GitHub GraphQL tables over an explicit repository set.
//!
//! Rows are fetched only when polled. Filters, joins, aggregates, and column
//! selection run in DataFusion; the wire queries deliberately have fixed fields.
//! Each scan has its own request budget and cursor state. There is no snapshot
//! isolation, automatic retry, or query-wide budget across multiple scans.

use std::{
    collections::{BTreeSet, VecDeque},
    fmt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray, TimestampMillisecondArray},
        datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit},
        record_batch::RecordBatch,
    },
    catalog::{TableProvider, streaming::StreamingTable},
    error::{DataFusionError, Result},
    execution::TaskContext,
    physical_plan::{
        SendableRecordBatchStream, stream::RecordBatchStreamAdapter, streaming::PartitionStream,
    },
};
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, HeaderValue},
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

const ISSUES_QUERY: &str = r#"
query SemanticDbIssues($owner: String!, $name: String!, $first: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    nameWithOwner
    issues(first: $first, after: $after, orderBy: {field: CREATED_AT, direction: ASC}) {
      nodes { id number title state author { login } createdAt updatedAt closedAt url }
      pageInfo { hasNextPage endCursor }
    }
  }
}"#;

const LABELS_QUERY: &str = r#"
query SemanticDbIssueLabels($id: ID!, $first: Int!, $after: String) {
  node(id: $id) {
    ... on Issue {
      labels(first: $first, after: $after) {
        nodes { id name }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}"#;

/// No environment is read by the connector. Keep tokens out of model documents.
/// Debug output intentionally excludes the token and endpoint.
#[derive(Clone)]
pub struct GitHubConfig {
    pub token: String,
    pub repositories: Vec<String>,
    pub endpoint: String,
    pub page_size: usize,
    /// Includes issue enumeration and label requests, across all scoped repos.
    /// Exceeding this budget fails the scan instead of returning truncated rows.
    pub max_requests_per_scan: usize,
    pub request_timeout: Duration,
    pub max_response_bytes: usize,
}

impl GitHubConfig {
    pub fn new(token: String, repositories: Vec<String>) -> Self {
        Self {
            token,
            repositories,
            endpoint: "https://api.github.com/graphql".into(),
            page_size: 100,
            max_requests_per_scan: 100,
            request_timeout: Duration::from_secs(30),
            max_response_bytes: 4 * 1024 * 1024,
        }
    }
}

impl fmt::Debug for GitHubConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubConfig")
            .field("repository_count", &self.repositories.len())
            .field("page_size", &self.page_size)
            .field("max_requests_per_scan", &self.max_requests_per_scan)
            .finish_non_exhaustive()
    }
}

struct Connection {
    client: Client,
    endpoint: Url,
    authorization: HeaderValue,
    config: GitHubConfig,
    requests: AtomicUsize,
}

/// Share one client between issue and label providers. Re-executing a query
/// performs fresh reads. `request_count` counts attempted HTTP requests over
/// this client's lifetime, including concurrent scans.
#[derive(Clone)]
pub struct GitHub {
    connection: Arc<Connection>,
}

impl fmt::Debug for GitHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHub")
            .field("config", &self.connection.config)
            .finish()
    }
}

impl GitHub {
    pub fn new(mut config: GitHubConfig) -> Result<Self> {
        if config.token.trim().is_empty()
            || config.repositories.is_empty()
            || !(1..=100).contains(&config.page_size)
            || config.max_requests_per_scan == 0
            || config.request_timeout.is_zero()
            || config.max_response_bytes == 0
        {
            return Err(error(
                "invalid configuration: token, repository scope and budgets must be nonempty/positive; page size must be 1–100",
            ));
        }
        let mut seen = BTreeSet::new();
        for repository in &config.repositories {
            let parts: Vec<_> = repository.split('/').collect();
            if parts.len() != 2
                || parts.iter().any(|s| {
                    s.is_empty()
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                })
            {
                return Err(error(
                    "repository scope must contain owner/name identifiers",
                ));
            }
            if !seen.insert(repository.to_ascii_lowercase()) {
                return Err(error("duplicate repository in scope"));
            }
        }
        let endpoint = Url::parse(&config.endpoint).map_err(|_| error("invalid endpoint"))?;
        let loopback = matches!(
            endpoint.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]")
        );
        if (endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback))
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(error(
                "endpoint must be HTTPS (or loopback HTTP), without credentials, query or fragment",
            ));
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", config.token))
            .map_err(|_| error("invalid authorization header"))?;
        authorization.set_sensitive(true);
        config.token.clear();
        let client = Client::builder()
            .user_agent("semantic-db-github-example")
            .timeout(config.request_timeout)
            .connect_timeout(config.request_timeout.min(Duration::from_secs(10)))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(transport_error)?;
        Ok(Self {
            connection: Arc::new(Connection {
                client,
                endpoint,
                authorization,
                config,
                requests: AtomicUsize::new(0),
            }),
        })
    }

    pub fn issues(&self) -> Result<Arc<dyn TableProvider>> {
        self.table(Kind::Issues)
    }

    pub fn issue_labels(&self) -> Result<Arc<dyn TableProvider>> {
        self.table(Kind::Labels)
    }

    pub fn request_count(&self) -> usize {
        self.connection.requests.load(Ordering::Relaxed)
    }

    fn table(&self, kind: Kind) -> Result<Arc<dyn TableProvider>> {
        let schema = kind.schema();
        let partition = Arc::new(GitHubPartition {
            github: self.clone(),
            kind,
            schema: schema.clone(),
        });
        Ok(Arc::new(StreamingTable::try_new(schema, vec![partition])?))
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Issues,
    Labels,
}

impl Kind {
    fn schema(self) -> SchemaRef {
        let text = |name, nullable| Field::new(name, DataType::Utf8, nullable);
        let timestamp = |name, nullable| {
            Field::new(
                name,
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                nullable,
            )
        };
        Arc::new(Schema::new(match self {
            Self::Issues => vec![
                text("issue_id", false),
                text("repository", false),
                Field::new("number", DataType::Int64, false),
                text("title", false),
                text("state", false),
                text("author_login", true),
                timestamp("created_at", false),
                timestamp("updated_at", false),
                timestamp("closed_at", true),
                text("url", false),
            ],
            Self::Labels => vec![
                text("issue_id", false),
                text("repository", false),
                text("label_id", false),
                text("label", false),
            ],
        }))
    }
}

#[derive(Debug)]
struct GitHubPartition {
    github: GitHub,
    kind: Kind,
    schema: SchemaRef,
}

impl PartitionStream for GitHubPartition {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    fn execute(&self, _ctx: Arc<TaskContext>) -> SendableRecordBatchStream {
        // No spawned task or prefetch: dropping the stream drops in-flight I/O.
        let state = Scan {
            github: self.github.clone(),
            kind: self.kind,
            schema: self.schema.clone(),
            repository_index: 0,
            canonical_repositories: BTreeSet::new(),
            issue_cursor: Cursor::default(),
            labels: VecDeque::new(),
            requests: 0,
        };
        Box::pin(RecordBatchStreamAdapter::new(
            self.schema.clone(),
            futures::stream::try_unfold(state, |mut state| async move {
                Ok(state.next_batch().await?.map(|batch| (batch, state)))
            }),
        ))
    }
}

#[derive(Default)]
struct Cursor {
    after: Option<String>,
    seen: BTreeSet<String>,
}

impl Cursor {
    fn advance(&mut self, page: PageInfo) -> Result<bool> {
        if !page.has_next_page {
            return Ok(false);
        }
        let next = page
            .end_cursor
            .filter(|s| !s.is_empty())
            .ok_or_else(|| error("pagination has another page without a cursor"))?;
        if !self.seen.insert(next.clone()) {
            return Err(error(
                "pagination cursor repeated; refusing incomplete results",
            ));
        }
        self.after = Some(next);
        Ok(true)
    }
}

struct PendingLabels {
    issue_id: String,
    repository: String,
    cursor: Cursor,
}

struct Scan {
    github: GitHub,
    kind: Kind,
    schema: SchemaRef,
    repository_index: usize,
    canonical_repositories: BTreeSet<String>,
    issue_cursor: Cursor,
    labels: VecDeque<PendingLabels>,
    requests: usize,
}

impl Scan {
    async fn request<T: DeserializeOwned>(&mut self, query: &str, variables: Value) -> Result<T> {
        let connection = &self.github.connection;
        if self.requests >= connection.config.max_requests_per_scan {
            return Err(error(
                "request budget exhausted; query results are incomplete",
            ));
        }
        self.requests += 1;
        connection.requests.fetch_add(1, Ordering::Relaxed);
        let mut response = connection
            .client
            .post(connection.endpoint.clone())
            .header(AUTHORIZATION, connection.authorization.clone())
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            return Err(error(&format!(
                "HTTP {}; no automatic retry",
                response.status().as_u16()
            )));
        }
        let max_bytes = connection.config.max_response_bytes;
        if response
            .content_length()
            .is_some_and(|n| n > max_bytes as u64)
        {
            return Err(error("response exceeds byte budget"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if chunk.len() > max_bytes.saturating_sub(bytes.len()) {
                return Err(error("response exceeds byte budget"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let envelope: Value =
            serde_json::from_slice(&bytes).map_err(|_| error("invalid JSON response"))?;
        if envelope
            .get("errors")
            .is_some_and(|v| !matches!(v, Value::Array(a) if a.is_empty()))
        {
            // GraphQL can return HTTP 200 and partial data. Never consume it.
            // Do not echo server messages, which may contain private data.
            return Err(error("GraphQL errors; partial data rejected"));
        }
        serde_json::from_value(envelope.get("data").cloned().unwrap_or(Value::Null))
            .map_err(|_| error("missing or invalid GraphQL data"))
    }

    async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        loop {
            let first = self.github.connection.config.page_size;
            if let Some(mut pending) = self.labels.pop_front() {
                let response: LabelData = self
                    .request(
                        LABELS_QUERY,
                        json!({
                            "id": pending.issue_id, "first": first, "after": pending.cursor.after,
                        }),
                    )
                    .await?;
                let labels = response
                    .node
                    .ok_or_else(|| error("issue unavailable while fetching labels"))?
                    .labels;
                let has_more = pending.cursor.advance(labels.page_info)?;
                let batch = label_batch(&self.schema, &pending, labels.nodes)?;
                if has_more {
                    self.labels.push_front(pending);
                }
                if batch.num_rows() > 0 {
                    return Ok(Some(batch));
                }
                continue;
            }
            let Some(repository) = self
                .github
                .connection
                .config
                .repositories
                .get(self.repository_index)
                .cloned()
            else {
                return Ok(None);
            };
            let (owner, name) = repository.split_once('/').expect("validated scope");
            let first_page = self.issue_cursor.after.is_none();
            let response: IssueData = self.request(ISSUES_QUERY, json!({
                "owner": owner, "name": name, "first": first, "after": self.issue_cursor.after,
            })).await?;
            let repository = response
                .repository
                .ok_or_else(|| error("repository unavailable; check scope and permissions"))?;
            if first_page
                && !self
                    .canonical_repositories
                    .insert(repository.name_with_owner.to_ascii_lowercase())
            {
                return Err(error("repository aliases resolve to duplicate scope"));
            }
            if !self.issue_cursor.advance(repository.issues.page_info)? {
                self.repository_index += 1;
                self.issue_cursor = Cursor::default();
            }
            match self.kind {
                Kind::Issues => {
                    let batch = issue_batch(
                        &self.schema,
                        &repository.name_with_owner,
                        repository.issues.nodes,
                    )?;
                    if batch.num_rows() > 0 {
                        return Ok(Some(batch));
                    }
                }
                Kind::Labels => {
                    self.labels
                        .extend(
                            repository
                                .issues
                                .nodes
                                .into_iter()
                                .map(|issue| PendingLabels {
                                    issue_id: issue.id,
                                    repository: repository.name_with_owner.clone(),
                                    cursor: Cursor::default(),
                                }),
                        );
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    #[serde(deserialize_with = "required_nullable")]
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    nodes: Vec<T>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
struct IssueData {
    #[serde(deserialize_with = "required_nullable")]
    repository: Option<Repository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    name_with_owner: String,
    issues: Page<Issue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Issue {
    id: String,
    number: i64,
    title: String,
    state: String,
    #[serde(deserialize_with = "required_nullable")]
    author: Option<Author>,
    created_at: String,
    updated_at: String,
    #[serde(deserialize_with = "required_nullable")]
    closed_at: Option<String>,
    url: String,
}

#[derive(Deserialize)]
struct Author {
    login: String,
}
#[derive(Deserialize)]
struct LabelData {
    #[serde(deserialize_with = "required_nullable")]
    node: Option<LabelNode>,
}

// Selected nullable fields must be present. Missing fields indicate an invalid
// response, not evidence that the source value was NULL.
fn required_nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}
#[derive(Deserialize)]
struct LabelNode {
    labels: Page<Label>,
}
#[derive(Deserialize)]
struct Label {
    id: String,
    name: String,
}

fn issue_batch(schema: &SchemaRef, repository: &str, issues: Vec<Issue>) -> Result<RecordBatch> {
    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut closed = Vec::new();
    for issue in &issues {
        created.push(timestamp(&issue.created_at)?);
        updated.push(timestamp(&issue.updated_at)?);
        closed.push(issue.closed_at.as_deref().map(timestamp).transpose()?);
    }
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(
            issues.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(vec![repository; issues.len()])),
        Arc::new(Int64Array::from(
            issues.iter().map(|i| i.number).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(
            issues.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(
            issues.iter().map(|i| i.state.as_str()).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(
            issues
                .iter()
                .map(|i| i.author.as_ref().map(|a| a.login.as_str()))
                .collect::<Vec<_>>(),
        )),
        Arc::new(TimestampMillisecondArray::from(created).with_timezone("UTC")),
        Arc::new(TimestampMillisecondArray::from(updated).with_timezone("UTC")),
        Arc::new(TimestampMillisecondArray::from(closed).with_timezone("UTC")),
        Arc::new(StringArray::from(
            issues.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(),
        )),
    ];
    Ok(RecordBatch::try_new(schema.clone(), columns)?)
}

fn label_batch(
    schema: &SchemaRef,
    issue: &PendingLabels,
    labels: Vec<Label>,
) -> Result<RecordBatch> {
    Ok(RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec![
                issue.issue_id.as_str();
                labels.len()
            ])),
            Arc::new(StringArray::from(vec![
                issue.repository.as_str();
                labels.len()
            ])),
            Arc::new(StringArray::from(
                labels.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            )),
        ],
    )?)
}

fn timestamp(value: &str) -> Result<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.timestamp_millis())
        .map_err(|_| error("invalid timestamp in issue data"))
}

fn error(message: &str) -> DataFusionError {
    DataFusionError::Execution(format!("GitHub: {message}"))
}

fn transport_error(cause: reqwest::Error) -> DataFusionError {
    error(if cause.is_timeout() {
        "request timed out"
    } else {
        "HTTP transport failed"
    })
}
