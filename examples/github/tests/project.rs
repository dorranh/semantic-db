use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "semantic-project-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sdb-github"))
            .current_dir(&self.0)
            .env_remove("GITHUB_TOKEN")
            .env_remove("OPENAI_API_KEY")
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn github_offline_commands_do_not_read_credentials_or_dotenv() {
    let temp = Temp::new();
    std::fs::write(temp.0.join(".env"), "not a valid env file\n").unwrap();
    let config = format!("{ROOT}/examples/github/semantic-db.yaml");
    for mode in ["validate", "inspect"] {
        let result = success(temp.run(&[mode, "--project-config", &config]));
        assert!(result.contains("github_maintenance") && result.contains("github.scoped.issues"));
    }
}

#[test]
fn configured_github_and_csv_federate_through_the_example_cli() {
    use serde_json::{Value, json};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };
    let temp = Temp::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => panic!("CLI did not request the fixture: {error}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let count = socket.read(&mut buffer).unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() < end + 4 + length {
                    continue;
                }
                assert!(headers.contains("authorization: bearer fixture-token"));
                let request: Value =
                    serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                assert_eq!(request["variables"]["states"], json!(["OPEN"]));
                assert_eq!(request["variables"]["owner"], "acme");
                break;
            }
        }
        let body = json!({"data":{"repository":{"nameWithOwner":"acme/widget","issues":{
            "nodes":[{"id":"I1","number":1,"title":"Fixture","state":"OPEN","author":null,
                "createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z","closedAt":null,
                "url":"https://github.com/acme/widget/issues/1"}],
            "pageInfo":{"hasNextPage":false,"endCursor":null}
        }}}}).to_string();
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    std::fs::write(
        temp.0.join("teams.csv"),
        "repository,team\nacme/widget,Platform\n",
    )
    .unwrap();
    std::fs::write(temp.0.join("project.json"), json!({
        "ossie":format!("{ROOT}/examples/github/github.ossie.yaml"),
        "connections":{"github":{"connector":"github","token_env":"GITHUB_TOKEN","repositories":["acme/widget"],"endpoint":endpoint},
            "local":{"connector":"csv"}},
        "sources":{"github.scoped.issues":{"connection":"github","collection":"issues"},
            "github.scoped.issue_labels":{"connection":"github","collection":"issue_labels"},
            "local.repository_teams":{"connection":"local","path":"teams.csv"}}
    }).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sdb-github"))
        .arg("sql")
        .current_dir(&temp.0)
        .env("GITHUB_TOKEN", "fixture-token")
        .env_remove("OPENAI_API_KEY")
        .args([
            "--project-config",
            "project.json",
            "--file",
            &format!("{ROOT}/examples/github/open_issues_by_team.sql"),
        ])
        .output()
        .unwrap();
    let text = success(output);
    server.join().unwrap();
    assert!(text.contains("Platform") && text.contains("acme/widget") && text.contains("1 row(s)"));
}

use semantic_sources::Project;

#[tokio::test]
async fn github_project_inspects_without_secrets_and_schema_load_needs_no_http() {
    let project =
        Project::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/semantic-db.yaml")).unwrap();
    assert_eq!(
        project
            .inspect(&example_github::registry().unwrap())
            .unwrap()
            .datasets
            .len(),
        3
    );
    let error = project
        .load(&example_github::registry().unwrap(), &|_| None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("GITHUB_TOKEN") && error.contains("/connections/github"));
    // Construction uses fixed GitHub schemas, so this token is never sent.
    let imported = project
        .load(&example_github::registry().unwrap(), &|_| {
            Some("not-a-real-token".into())
        })
        .await
        .unwrap();
    imported
        .engine
        .plan_sql("SELECT * FROM issues WHERE state = 'OPEN'")
        .await
        .unwrap();
}
