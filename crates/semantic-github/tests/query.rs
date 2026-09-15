use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use datafusion::{catalog::TableProvider, prelude::SessionContext};
use futures::StreamExt;
use semantic_compiler::{
    Compiler, GroundingOutcome,
    provider::{Message, ModelProvider, ProviderError},
};
use semantic_engine::{Engine, pretty_format_batches};
use semantic_github::{GitHub, GitHubConfig};
use semantic_ossie::{OssieDocument, SourceBindings};
use serde_json::{Value, json};

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(respond: impl Fn(&Value, usize) -> (u16, String, Duration) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/graphql", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let seen = requests.clone();
        let stopped = stop.clone();
        let handle = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(socket) => socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                // Accepted sockets can inherit nonblocking mode on macOS.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let request = loop {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        assert!(headers.starts_with("POST /graphql HTTP/1.1"));
                        assert!(
                            headers
                                .to_ascii_lowercase()
                                .contains("authorization: bearer fixture-token")
                        );
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(str::to_owned)
                            })
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break serde_json::from_slice::<Value>(
                                &bytes[end + 4..end + 4 + length],
                            )
                            .unwrap();
                        }
                    }
                };
                let index = {
                    let mut seen = seen.lock().unwrap();
                    seen.push(request.clone());
                    seen.len() - 1
                };
                let (status, body, delay) = respond(&request, index);
                thread::sleep(delay);
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes());
            }
        });
        Self {
            url,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    fn fixture() -> Self {
        Self::new(|request, _| (200, fixture_response(request).to_string(), Duration::ZERO))
    }

    fn config(&self) -> GitHubConfig {
        let mut config = GitHubConfig::new("fixture-token".into(), vec!["acme/widget".into()]);
        config.endpoint = self.url.clone();
        config.page_size = 2;
        config.filter_pushdown = false;
        config
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.handle.take().unwrap().join();
        if !thread::panicking() {
            assert!(result.is_ok(), "fixture server failed");
        }
    }
}

fn issue(number: i64, state: &str, author: Option<&str>) -> Value {
    json!({
        "id": format!("I{number}"), "number": number, "title": format!("Issue {number}"),
        "state": state, "author": author.map(|login| json!({"login": login})),
        "createdAt": "2026-01-01T01:00:00+01:00", "updatedAt": "2026-02-01T00:00:00Z",
        "closedAt": if state == "CLOSED" { Some("2026-02-01T00:00:00Z") } else { None },
        "url": format!("https://github.com/acme/widget/issues/{number}")
    })
}

fn page(nodes: Vec<Value>, next: Option<&str>) -> Value {
    json!({"nodes": nodes, "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next}})
}

fn fixture_response(request: &Value) -> Value {
    let variables = &request["variables"];
    assert_eq!(variables["first"], 2);
    if request["query"]
        .as_str()
        .unwrap()
        .contains("SemanticDbIssueLabels")
    {
        let labels = match (
            variables["id"].as_str().unwrap(),
            variables["after"].as_str(),
        ) {
            ("I1", None) => page(vec![], None),
            ("I2", None) => page(vec![json!({"id":"L1", "name":"bug"})], Some("label-next")),
            ("I2", Some("label-next")) => page(vec![json!({"id":"L2", "name":"triage"})], None),
            ("I3", None) => page(vec![json!({"id":"L1", "name":"bug"})], None),
            _ => panic!("unexpected label cursor"),
        };
        json!({"data": {"node": {"labels": labels}}})
    } else {
        assert_eq!(variables["owner"], "acme");
        assert_eq!(variables["name"], "widget");
        let issues = match variables["after"].as_str() {
            None => page(vec![issue(1, "CLOSED", Some("alice"))], Some("issues-next")),
            Some("issues-next") => page(
                vec![issue(2, "OPEN", None), issue(3, "OPEN", Some("alice"))],
                None,
            ),
            _ => panic!("unexpected issue cursor"),
        };
        json!({"data": {"repository": {"nameWithOwner":"acme/widget", "issues": issues}}})
    }
}

async fn view(sql: &str) -> Arc<dyn TableProvider> {
    SessionContext::new().sql(sql).await.unwrap().into_view()
}

async fn engine(issues: Arc<dyn TableProvider>, labels: Arc<dyn TableProvider>) -> Engine {
    let mut sources = SourceBindings::new();
    sources.bind("github.scoped.issues", issues).unwrap();
    sources.bind("github.scoped.issue_labels", labels).unwrap();
    sources
        .bind(
            "local.repository_teams",
            view("SELECT 'acme/widget' AS repository, 'Platform' AS team").await,
        )
        .unwrap();
    let document =
        OssieDocument::parse(include_str!("../../../examples/github/github.ossie.yaml")).unwrap();
    let imported = document.load(Some("github_maintenance"), &sources).unwrap();
    assert_eq!(imported.warnings.len(), 3);
    imported.engine
}

async fn remote(github: &GitHub) -> Engine {
    engine(github.issues().unwrap(), github.issue_labels().unwrap()).await
}

async fn reference() -> Engine {
    let issues = view(
        "SELECT issue_id, 'acme/widget' AS repository, number, title, state, author_login,
        arrow_cast('2026-01-01T00:00:00Z', 'Timestamp(Millisecond, Some(\"UTC\"))') AS created_at,
        arrow_cast('2026-02-01T00:00:00Z', 'Timestamp(Millisecond, Some(\"UTC\"))') AS updated_at,
        arrow_cast(closed_at, 'Timestamp(Millisecond, Some(\"UTC\"))') AS closed_at,
        concat('https://github.com/acme/widget/issues/', number) AS url
        FROM (VALUES
            ('I1', 1, 'Issue 1', 'CLOSED', 'alice', '2026-02-01T00:00:00Z'),
            ('I2', 2, 'Issue 2', 'OPEN', NULL, NULL),
            ('I3', 3, 'Issue 3', 'OPEN', 'alice', NULL)
        ) AS t(issue_id, number, title, state, author_login, closed_at)",
    )
    .await;
    let labels = view("SELECT issue_id, 'acme/widget' AS repository, label_id, label FROM
        (VALUES ('I2', 'L1', 'bug'), ('I2', 'L2', 'triage'), ('I3', 'L1', 'bug')) AS t(issue_id, label_id, label)").await;
    engine(issues, labels).await
}

async fn result(engine: &Engine, sql: &str) -> String {
    pretty_format_batches(&engine.query(sql).await.unwrap())
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn ossie_queries_match_local_reference_across_pages_nulls_timestamps_and_joins() {
    let server = Server::fixture();
    let github = GitHub::new(server.config()).unwrap();
    let engine = remote(&github).await;
    let reference = reference().await;
    engine.plan_sql("SELECT * FROM issues").await.unwrap();
    assert_eq!(
        github.request_count(),
        0,
        "registration and planning must be lazy"
    );
    for (sql, requests) in [
        ("SELECT * FROM issues ORDER BY number", 2),
        ("SELECT COUNT(*) FROM issues", 2),
        (
            "SELECT issue_id FROM issues WHERE state = 'OPEN' AND author_login IS NULL LIMIT 1",
            2,
        ),
        ("SELECT number FROM issues WHERE title LIKE '%3' LIMIT 1", 2),
        ("SELECT number FROM issues ORDER BY number DESC LIMIT 1", 2),
        ("SELECT * FROM issue_labels ORDER BY issue_id, label_id", 6),
        (
            include_str!("../../../examples/github/open_issues_by_team.sql"),
            2,
        ),
        (
            include_str!("../../../examples/github/open_issues_by_label.sql"),
            8,
        ),
    ] {
        let before = github.request_count();
        assert_eq!(
            result(&engine, sql).await,
            result(&reference, sql).await,
            "{sql}"
        );
        assert_eq!(github.request_count() - before, requests, "{sql}");
    }
    assert_eq!(
        github.request_count(),
        server.requests.lock().unwrap().len()
    );
}

#[tokio::test]
async fn limits_and_stream_drop_stop_pagination_but_residual_filters_do_not_truncate() {
    let server = Server::fixture();
    let github = GitHub::new(server.config()).unwrap();
    let engine = remote(&github).await;
    assert!(
        result(&engine, "SELECT issue_id FROM issues LIMIT 1")
            .await
            .contains("I1")
    );
    assert_eq!(github.request_count(), 1);
    assert!(
        result(
            &engine,
            "SELECT issue_id FROM issues WHERE state = 'OPEN' LIMIT 1"
        )
        .await
        .contains("I2")
    );
    assert_eq!(
        github.request_count(),
        3,
        "filter must traverse beyond the first page"
    );
    let mut stream = engine
        .plan_sql("SELECT * FROM issues")
        .await
        .unwrap()
        .execute_stream()
        .await
        .unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().num_rows(), 1);
    drop(stream);
    assert_eq!(
        github.request_count(),
        4,
        "no background prefetch after stream drop"
    );
}

#[tokio::test]
async fn later_page_failure_and_exhausted_budget_never_become_successful_counts() {
    let server = Server::new(|request, index| {
        let body = if index == 0 {
            fixture_response(request)
        } else {
            json!({"data": {"repository": null}, "errors": [{"message":"fixture-token"}]})
        };
        (200, body.to_string(), Duration::ZERO)
    });
    let github = GitHub::new(server.config()).unwrap();
    let error = remote(&github)
        .await
        .query("SELECT COUNT(*) FROM issues")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("partial data rejected"));
    assert!(!format!("{error:?}").contains("fixture-token"));
    assert_eq!(github.request_count(), 2);

    let server = Server::fixture();
    let mut config = server.config();
    config.max_requests_per_scan = 1;
    let github = GitHub::new(config).unwrap();
    let error = remote(&github)
        .await
        .query("SELECT COUNT(*) FROM issues")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("request budget exhausted"));
    assert_eq!(github.request_count(), 1);
}

#[tokio::test]
async fn rejects_http_errors_bad_data_oversize_responses_and_timeouts() {
    for (status, body, delay, expected) in [
        (401, "fixture-token", Duration::ZERO, "HTTP 401"),
        (403, "fixture-token", Duration::ZERO, "HTTP 403"),
        (429, "fixture-token", Duration::ZERO, "HTTP 429"),
        (302, "fixture-token", Duration::ZERO, "HTTP 302"),
        (200, "bad", Duration::ZERO, "invalid JSON"),
        (200, "{}", Duration::ZERO, "invalid GraphQL data"),
        (
            200,
            r#"{"data":{"repository":null}}"#,
            Duration::ZERO,
            "repository unavailable",
        ),
        (200, "{}", Duration::from_millis(500), "timed out"),
    ] {
        let server = Server::new(move |_, _| (status, body.into(), delay));
        let mut config = server.config();
        config.request_timeout = if delay.is_zero() {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(200)
        };
        let github = GitHub::new(config).unwrap();
        let error = remote(&github)
            .await
            .query("SELECT * FROM issues")
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert!(!format!("{error:?}").contains("fixture-token"));
        assert_eq!(github.request_count(), 1);
    }
    let server = Server::fixture();
    let mut config = server.config();
    config.max_response_bytes = 10;
    let error = remote(&GitHub::new(config).unwrap())
        .await
        .query("SELECT * FROM issues")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("byte budget"));
}

#[tokio::test]
async fn rejects_broken_cursors_and_null_nodes_instead_of_skipping_rows() {
    for mode in [
        "repeated",
        "missing",
        "null_node",
        "timestamp",
        "labels_null",
        "missing_author",
    ] {
        let server = Server::new(move |request, _| {
            let mut body = fixture_response(request);
            match mode {
                "repeated" => {
                    body["data"]["repository"]["issues"]["pageInfo"] =
                        json!({"hasNextPage":true,"endCursor":"issues-next"})
                }
                "missing" => {
                    body["data"]["repository"]["issues"]["pageInfo"] =
                        json!({"hasNextPage":true,"endCursor":null})
                }
                "null_node" => body["data"]["repository"]["issues"]["nodes"] = json!([null]),
                "timestamp" => {
                    body["data"]["repository"]["issues"]["nodes"][0]["createdAt"] = json!("invalid")
                }
                "missing_author" => {
                    body["data"]["repository"]["issues"]["nodes"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("author");
                }
                "labels_null"
                    if request["query"]
                        .as_str()
                        .unwrap()
                        .contains("SemanticDbIssueLabels") =>
                {
                    body["data"]["node"]["labels"] = Value::Null
                }
                _ => {}
            }
            (200, body.to_string(), Duration::ZERO)
        });
        let github = GitHub::new(server.config()).unwrap();
        let table = if mode == "labels_null" {
            "issue_labels"
        } else {
            "issues"
        };
        assert!(
            remote(&github)
                .await
                .query(&format!("SELECT COUNT(*) FROM {table}"))
                .await
                .is_err(),
            "{mode}"
        );
    }
}

#[test]
fn rejects_invalid_scope_and_redacts_debug_output() {
    for repositories in [vec![], vec!["bad".into()], vec!["a/b".into(), "A/B".into()]] {
        assert!(GitHub::new(GitHubConfig::new("fixture-token".into(), repositories)).is_err());
    }
    let config = GitHubConfig::new("fixture-token".into(), vec!["a/b".into()]);
    assert!(!format!("{config:?}").contains("fixture-token"));
    assert!(!format!("{:?}", GitHub::new(config).unwrap()).contains("fixture-token"));
    for endpoint in [
        "http://example.com/graphql",
        "https://fixture-token@example.com/graphql",
        "https://example.com/graphql?key=fixture-token",
    ] {
        let mut config = GitHubConfig::new("fixture-token".into(), vec!["a/b".into()]);
        config.endpoint = endpoint.into();
        let error = GitHub::new(config).unwrap_err();
        assert!(!format!("{error:?}").contains("fixture-token"));
    }
}

#[tokio::test]
async fn repository_scope_resets_cursors_and_includes_empty_repositories() {
    let server = Server::new(|request, _| {
        let name = request["variables"]["name"].as_str().unwrap();
        let after = request["variables"]["after"].as_str();
        let (nodes, next) = match (name, after) {
            ("first", None) => (vec![issue(1, "OPEN", None)], Some("next")),
            ("first", Some("next")) => (vec![issue(2, "OPEN", None)], None),
            ("empty", None) => (vec![], None),
            ("last", None) => (vec![issue(3, "OPEN", None)], None),
            _ => panic!("cursor leaked between repositories"),
        };
        (
            200,
            json!({"data":{"repository":{
                "nameWithOwner":format!("acme/{name}"), "issues":page(nodes, next)
            }}})
            .to_string(),
            Duration::ZERO,
        )
    });
    let mut config = server.config();
    config.repositories = vec!["acme/first".into(), "acme/empty".into(), "acme/last".into()];
    let github = GitHub::new(config).unwrap();
    let rows = result(
        &remote(&github).await,
        "SELECT repository, COUNT(*) AS n FROM issues GROUP BY repository ORDER BY repository",
    )
    .await;
    assert!(rows.contains("acme/first | 2"), "{rows}");
    assert!(rows.contains("acme/last  | 1"), "{rows}");
    assert_eq!(github.request_count(), 4);
}

#[tokio::test]
async fn aliases_and_nested_label_budgets_fail_without_silent_truncation() {
    let server = Server::new(|_, _| {
        (
            200,
            json!({"data":{"repository":{
                "nameWithOwner":"acme/canonical", "issues":page(vec![], None)
            }}})
            .to_string(),
            Duration::ZERO,
        )
    });
    let mut config = server.config();
    config.repositories = vec!["acme/old".into(), "acme/new".into()];
    let github = GitHub::new(config).unwrap();
    let error = remote(&github)
        .await
        .query("SELECT COUNT(*) FROM issues")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("duplicate scope"));
    assert_eq!(github.request_count(), 2);

    let server = Server::fixture();
    let mut config = server.config();
    config.max_requests_per_scan = 3;
    let github = GitHub::new(config).unwrap();
    let error = remote(&github)
        .await
        .query("SELECT COUNT(*) FROM issue_labels")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("request budget exhausted"));
    assert_eq!(github.request_count(), 3);
}

#[tokio::test]
#[ignore = "requires GITHUB_TOKEN; performs one bounded live GitHub request"]
async fn live_github_smoke() {
    let mut config = GitHubConfig::new(
        std::env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN"),
        vec!["apache/ossie".into()],
    );
    config.page_size = 1;
    config.max_requests_per_scan = 1;
    let github = GitHub::new(config).unwrap();
    let engine = remote(&github).await;
    let batches = engine
        .query("SELECT issue_id, repository, number FROM issues LIMIT 1")
        .await
        .unwrap();
    assert!(batches.iter().map(|b| b.num_rows()).sum::<usize>() <= 1);
    assert_eq!(github.request_count(), 1);
}

#[tokio::test]
async fn compiler_receives_semantics_and_clarification_does_not_execute_github() {
    struct Model {
        clarify: bool,
    }
    impl ModelProvider for Model {
        async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
            let context: Value = serde_json::from_str(&messages[1].content).unwrap();
            let catalog = context["catalog"].as_array().unwrap();
            assert_eq!(catalog.len(), 3);
            let issues = catalog.iter().find(|r| r["name"] == "issues").unwrap();
            assert!(
                issues["semantics"]
                    .to_string()
                    .contains("stale issue requires an explicit age cutoff")
            );
            assert!(
                issues["columns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["name"] == "author_login" && c["nullable"] == true)
            );
            assert!(!messages[1].content.contains("fixture-token"));
            assert!(!messages[1].content.contains("github.scoped.issues"));
            Ok(if self.clarify {
                json!({"status":"needs_clarification", "phrases":["stale issues"],
                    "question":"Which time field, age cutoff, and reference time define stale?"})
            } else {
                json!({"status":"grounded", "query":{
                    "sql":include_str!("../../../examples/github/open_issues_by_team.sql"),
                    "evidence":[
                        {"phrase":"open issues", "catalog_reference":"issues.state", "interpretation":"OPEN state"},
                        {"phrase":"teams", "catalog_reference":"repository_teams", "interpretation":"Authored repository ownership"}
                    ]
                }})
            }.to_string())
        }
    }
    let server = Server::fixture();
    let github = GitHub::new(server.config()).unwrap();
    let engine = remote(&github).await;
    let clarification = Compiler::new(Model { clarify: true })
        .compile(&engine, "Which teams have stale issues?")
        .await
        .unwrap();
    assert!(matches!(
        clarification.outcome,
        GroundingOutcome::NeedsClarification { .. }
    ));
    let grounded = Compiler::new(Model { clarify: false })
        .compile(&engine, "Count open issues by team and repository")
        .await
        .unwrap();
    assert_eq!(github.request_count(), 0);
    let GroundingOutcome::Grounded { query } = grounded.outcome else {
        panic!("expected grounded query")
    };
    assert_eq!(
        result(&engine, &query.sql).await,
        result(&reference().await, &query.sql).await
    );
    assert_eq!(github.request_count(), 2);
}

#[tokio::test]
async fn exact_state_pushdown_matches_local_results_and_reduces_pages_through_ossie() {
    let server = Server::new(|request, _| {
        let vars = &request["variables"];
        assert!(
            request["query"]
                .as_str()
                .unwrap()
                .contains("states: $states")
        );
        let mut nodes = vec![
            issue(1, "CLOSED", Some("alice")),
            issue(2, "OPEN", None),
            issue(3, "OPEN", Some("alice")),
        ];
        if let Some(states) = vars["states"].as_array() {
            assert_eq!(states.len(), 1);
            nodes.retain(|node| states.contains(&node["state"]));
        }
        let offset = vars["after"]
            .as_str()
            .map(|v| v.parse::<usize>().unwrap())
            .unwrap_or(0);
        let end = (offset + vars["first"].as_u64().unwrap() as usize).min(nodes.len());
        let next = (end < nodes.len()).then(|| end.to_string());
        (200, json!({"data":{"repository":{"nameWithOwner":"acme/widget", "issues":page(nodes[offset..end].to_vec(), next.as_deref())}}}).to_string(), Duration::ZERO)
    });
    let mut config = server.config();
    config.page_size = 1;
    config.filter_pushdown = true;
    let github = GitHub::new(config.clone()).unwrap();
    let optimized = remote(&github).await;
    config.filter_pushdown = false;
    let baseline_client = GitHub::new(config).unwrap();
    let baseline = remote(&baseline_client).await;
    let reference = reference().await;
    let queries = [
        "SELECT number FROM issues WHERE state = 'OPEN' ORDER BY number",
        "SELECT number FROM issues WHERE 'CLOSED' = state ORDER BY number",
        "SELECT number FROM issues WHERE state = 'OPEN' AND author_login IS NULL ORDER BY number",
        "SELECT number FROM issues WHERE state = 'OPEN' AND title LIKE '%3' LIMIT 1",
        "SELECT number FROM issues WHERE state = 'OPEN' ORDER BY number DESC LIMIT 1",
        "SELECT COUNT(*) FROM issues WHERE state = 'OPEN'",
        "SELECT number FROM issues WHERE state = 'open' ORDER BY number",
        "SELECT number FROM issues WHERE lower(state) = 'open' ORDER BY number",
        "SELECT number FROM issues WHERE state = 'OPEN' OR number = 1 ORDER BY number",
        "SELECT number FROM issues WHERE state = 'OPEN' AND state = 'CLOSED'",
        "SELECT number FROM issues WHERE state IS NULL",
        "SELECT number FROM issues WHERE state = CAST(NULL AS VARCHAR)",
        "SELECT number FROM issues WHERE state = 'OPEN' LIMIT 0",
        include_str!("../../../examples/github/open_issues_by_team.sql"),
    ];
    optimized.plan_sql(queries[0]).await.unwrap();
    assert_eq!(github.request_count(), 0);
    for sql in queries {
        let expected = result(&reference, sql).await;
        assert_eq!(result(&optimized, sql).await, expected, "optimized: {sql}");
        assert_eq!(result(&baseline, sql).await, expected, "baseline: {sql}");
    }
    let before = github.request_count();
    result(&optimized, queries[0]).await;
    assert_eq!(github.request_count() - before, 2);
    let before = baseline_client.request_count();
    result(&baseline, queries[0]).await;
    assert_eq!(baseline_client.request_count() - before, 3);

    // Alias projection must preserve pushdown all the way to the remote source.
    let mut model: Value = github_model();
    let fields = model["semantic_model"][0]["datasets"][0]["fields"]
        .as_array_mut()
        .unwrap();
    fields
        .iter_mut()
        .find(|field| field["name"] == "state")
        .unwrap()["name"] = json!("issue_state");
    let mut sources = SourceBindings::new();
    sources
        .bind("github.scoped.issues", github.issues().unwrap())
        .unwrap();
    sources
        .bind("github.scoped.issue_labels", github.issue_labels().unwrap())
        .unwrap();
    sources
        .bind(
            "local.repository_teams",
            view("SELECT 'acme/widget' AS repository, 'Platform' AS team").await,
        )
        .unwrap();
    let aliased = OssieDocument::parse(&model.to_string())
        .unwrap()
        .load(None, &sources)
        .unwrap();
    let before = github.request_count();
    assert_eq!(
        result(
            &aliased.engine,
            "SELECT number FROM issues WHERE issue_state = 'OPEN' ORDER BY number"
        )
        .await,
        result(&reference, queries[0]).await
    );
    assert_eq!(github.request_count() - before, 2);
}

fn github_model() -> Value {
    OssieDocument::parse(include_str!("../../../examples/github/github.ossie.yaml"))
        .unwrap()
        .json()
        .clone()
}

#[tokio::test]
async fn rejects_remote_rows_that_violate_an_exact_pushed_filter() {
    let server = Server::fixture();
    let mut config = server.config();
    config.filter_pushdown = true;
    let engine = remote(&GitHub::new(config).unwrap()).await;
    let error = engine
        .query("SELECT number FROM issues WHERE state = 'OPEN'")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("violates pushed issue state filter"));
}

fn history_response(request: &Value) -> Value {
    let v = &request["variables"];
    let (canonical, count) = match v["name"].as_str().unwrap() {
        "old-name" => ("Acme/Widget", 30),
        "other" => ("acme/other", 40),
        "empty" => ("acme/empty", 0),
        _ => panic!("unexpected repository"),
    };
    let nodes = (1..=count)
        .map(|n| issue(n, if n % 2 == 0 { "OPEN" } else { "CLOSED" }, None))
        .filter(|node| {
            v["states"]
                .as_array()
                .is_none_or(|states| states.contains(&node["state"]))
        })
        .collect::<Vec<_>>();
    let offset = v["after"]
        .as_str()
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(0);
    let end = (offset + v["first"].as_u64().unwrap() as usize).min(nodes.len());
    let cursor = (end < nodes.len()).then(|| end.to_string());
    json!({"data":{"repository":{"nameWithOwner":canonical,"issues":page(nodes[offset..end].to_vec(),cursor.as_deref())}}})
}
fn history_config(server: &Server) -> GitHubConfig {
    let mut c = server.config();
    c.filter_pushdown = true;
    c.repositories = vec![
        "acme/old-name".into(),
        "acme/other".into(),
        "acme/empty".into(),
    ];
    c.page_size = 1;
    c.max_requests_per_scan = 128;
    c
}
#[tokio::test]
async fn repository_pruning_preserves_canonical_aliases_case_and_residual_semantics() {
    let server = Server::new(|r, _| (200, history_response(r).to_string(), Duration::ZERO));
    let c = history_config(&server);
    let github = GitHub::new(c.clone()).unwrap();
    let optimized = remote(&github).await;
    let mut baseline_config = c;
    baseline_config.filter_pushdown = false;
    let baseline = remote(&GitHub::new(baseline_config).unwrap()).await;
    for sql in [
        "SELECT repository,number,state FROM issues WHERE lower(repository)='acme/widget' ORDER BY number",
        "SELECT repository,number,state FROM issues WHERE 'Acme/Widget'=repository ORDER BY number",
        "SELECT number FROM issues WHERE repository='acme/widget'",
        "SELECT number FROM issues WHERE lower(repository)='ACME/WIDGET'",
        "SELECT repository,number FROM issues WHERE repository='Acme/Widget' OR number=1 ORDER BY repository,number",
        "SELECT number FROM issues WHERE lower(repository)='acme/widget' AND state='CLOSED' ORDER BY number",
    ] {
        assert_eq!(
            result(&optimized, sql).await,
            result(&baseline, sql).await,
            "{sql}"
        );
    }
    let start = server.requests.lock().unwrap().len();
    result(
        &optimized,
        "SELECT number FROM issues WHERE lower(repository)='acme/widget' ORDER BY number",
    )
    .await;
    let requests = server.requests.lock().unwrap();
    let requests = &requests[start..];
    assert_eq!(requests.len(), 32);
    assert!(
        requests
            .iter()
            .filter(|r| r["variables"]["name"] != "old-name")
            .all(|r| r["variables"]["after"].is_null())
    );
}
#[tokio::test]
async fn bound_repository_filter_reads_beyond_24_pages_through_views_and_keeps_closed_issues() {
    let server = Server::new(|r, _| (200, history_response(r).to_string(), Duration::ZERO));
    let mut c = history_config(&server);
    c.max_requests_per_scan = 32;
    let github = GitHub::new(c).unwrap();
    let mut engine = remote(&github).await;
    engine
        .create_view(
            "selected_issues",
            "SELECT repository AS repo, number, state FROM issues",
        )
        .await
        .unwrap();
    engine
        .create_view(
            "packages",
            "SELECT 'requests' AS name, 'acme/widget' AS repository",
        )
        .await
        .unwrap();
    let sql = "SELECT i.number,i.state FROM selected_issues i JOIN packages p ON lower(i.repo)=lower(p.repository) WHERE p.name=$1 AND lower(i.repo)=lower($2::text) ORDER BY i.number";
    engine.describe_read(sql, &[]).await.unwrap();
    assert_eq!(github.request_count(), 0);
    let batches = engine
        .execute_parameters(
            sql,
            vec![
                datafusion::common::ScalarValue::Utf8(Some("requests".into())),
                datafusion::common::ScalarValue::Utf8(Some("ACME/WIDGET".into())),
            ],
            semantic_engine::QueryOptions::default(),
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(batches.iter().map(|b| b.num_rows()).sum::<usize>(), 30);
    let output = pretty_format_batches(&batches).unwrap().to_string();
    assert!(output.contains("CLOSED"));
    assert!(output.contains("30"));
    assert_eq!(github.request_count(), 32);
}
#[tokio::test]
async fn pruned_history_still_rejects_late_errors_and_repository_identity_changes() {
    for changed_identity in [false, true] {
        let server = Server::new(move |r, _| {
            let mut response = history_response(r);
            if r["variables"]["after"] == "27" {
                if changed_identity {
                    response["data"]["repository"]["nameWithOwner"] = json!("acme/renamed");
                } else {
                    response = json!({"errors":[{"message":"injected late failure"}],"data":null});
                }
            }
            (200, response.to_string(), Duration::ZERO)
        });
        let github = GitHub::new(history_config(&server)).unwrap();
        let e = remote(&github).await;
        let failure = e
            .query(
                "SELECT number FROM issues WHERE lower(repository)='acme/widget' ORDER BY number",
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(
            failure.contains(if changed_identity {
                "identity changed"
            } else {
                "partial data rejected"
            }),
            "{failure}"
        );
        assert_eq!(github.request_count(), 28);
    }
}
