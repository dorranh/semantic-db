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
        Command::new(env!("CARGO_BIN_EXE_semantic-db"))
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
fn configured_csv_works_from_another_directory_with_validation_inspection_and_sql_planning() {
    let temp = Temp::new();
    let config = format!("{ROOT}/examples/geospatial/semantic-db.yaml");
    let inspected = success(temp.run(&["--config", &config, "--inspect"]));
    assert!(
        inspected.contains("geospatial_wells")
            && inspected.contains("total_depth_m")
            && inspected.contains("csv")
    );
    assert!(
        success(temp.run(&["--config", &config, "--validate"]))
            .contains("Offline validation passed")
    );
    assert!(
        success(temp.run(&["--config", &config, "--validate", "--connect"]))
            .contains("Connected schema validation passed")
    );
    let rows = success(temp.run(&[
        "--config",
        &config,
        "--file",
        &format!("{ROOT}/examples/geospatial/query.sql"),
    ]));
    assert!(rows.contains("W-001") && rows.contains("W-004") && rows.contains("2 row(s)"));
    let planned = success(temp.run(&[
        "--config",
        &config,
        "--query",
        "SELECT * FROM wells",
        "--dry-run",
    ]));
    assert!(planned.contains("Projection") && !planned.contains("W-001"));
}

#[test]
fn github_offline_commands_do_not_read_credentials_or_dotenv() {
    let temp = Temp::new();
    std::fs::write(temp.0.join(".env"), "not a valid env file\n").unwrap();
    let config = format!("{ROOT}/examples/github/semantic-db.yaml");
    for mode in ["--validate", "--inspect"] {
        let result = success(temp.run(&["--config", &config, mode]));
        assert!(result.contains("github_maintenance") && result.contains("github.scoped.issues"));
    }
    let model = format!("{ROOT}/examples/github/github.ossie.yaml");
    assert!(
        success(temp.run(&["--ossie", &model, "--inspect"]))
            .contains("explicit provider binding required")
    );
}

#[test]
fn invalid_configurations_fail_before_execution_with_actionable_diagnostics() {
    let temp = Temp::new();
    let model = format!("{ROOT}/examples/geospatial/wells.ossie.yaml");
    for (body, expected) in [
        (
            format!("ossie: {model}\nconnections: {{local: {{connector: nonexistent}}}}"),
            "unknown_connector",
        ),
        (
            format!("ossie: {model}\nconnections: {{local: {{connector: csv}}}}"),
            "missing_binding",
        ),
        (
            format!("ossie: {model}\nconnections: {{local: {{connector: csv, typo: true}}}}"),
            "unknown field",
        ),
        (format!("ossie: {model}\nossie: {model}"), "config_parse"),
        (
            format!("ossie: {model}\nconnections: {{local: {{connector: csv, connector: csv}}}}"),
            "config_parse",
        ),
    ] {
        std::fs::write(temp.0.join("project.yaml"), body).unwrap();
        let output = temp.run(&["--config", "project.yaml", "--validate"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
    }
    for args in [
        vec!["--validate"],
        vec!["--connect"],
        vec!["--config", "x", "--ossie", "y"],
        vec!["--config", "x", "--inspect", "--query", "SELECT 1"],
        vec!["--config", "x", "--csv", "x=y"],
        vec!["--config", "x", "--validate", "--ask", "hello"],
        vec!["--config", "x", "--dry-run"],
    ] {
        assert!(!temp.run(&args).status.success(), "{args:?}");
    }
}

#[test]
fn configured_github_and_csv_federate_through_the_standard_cli() {
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
    let output = Command::new(env!("CARGO_BIN_EXE_semantic-db"))
        .current_dir(&temp.0)
        .env("GITHUB_TOKEN", "fixture-token")
        .env_remove("OPENAI_API_KEY")
        .args([
            "--config",
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
