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
fn project_views_inspect_and_reload_from_another_working_directory() {
    let temp = Temp::new();
    let config = format!("{ROOT}/examples/geospatial/semantic-db.views.yaml");
    std::fs::write(temp.0.join(".env"), "not a valid env file\n").unwrap();
    let inspected = success(temp.run(&["--config", &config, "--inspect"]));
    assert!(inspected.contains("View deep_wells") && inspected.contains("views/deep_wells.sql"));
    assert!(inspected.contains("dependencies: deep_wells"));
    assert!(inspected.contains("example project's depth convention"));
    assert!(
        inspected.find("View deep_wells").unwrap()
            < inspected.find("View active_deep_wells").unwrap()
    );
    assert!(success(temp.run(&["--config", &config, "--validate"])).contains("view columns/types"));
    std::fs::remove_file(temp.0.join(".env")).unwrap();
    assert!(
        success(temp.run(&["--config", &config, "--validate", "--connect"]))
            .contains("Connected schema validation passed")
    );
    // Separate processes reload the authored definitions on every startup.
    for _ in 0..2 {
        let rows = success(temp.run(&[
            "--config",
            &config,
            "--query",
            "SELECT well_id FROM active_deep_wells WHERE basin = 'North Basin' ORDER BY well_id",
        ]));
        assert!(rows.contains("W-001") && rows.contains("W-004") && rows.contains("2 row(s)"));
    }
    let duplicate = temp.run(&[
        "--config",
        &config,
        "--view",
        "deep_wells=SELECT * FROM wells",
        "--query",
        "SELECT 1",
    ]);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("DuplicateRelation"));
}

#[test]
#[cfg(feature = "github")]
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
        (
            format!(
                "ossie: {model}\nviews:\n  duplicate: {{sql_file: x.sql}}\n  duplicate: {{sql_file: y.sql}}"
            ),
            "config_parse",
        ),
        (
            format!("ossie: {model}\nviews: {{bad: {{sql_file: x.sql, typo: true}}}}"),
            "unknown field",
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
#[cfg(feature = "github")]
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

#[test]
fn cache_status_invalidation_refresh_and_bypass_work_across_processes() {
    let temp = Temp::new();
    std::fs::copy(
        format!("{ROOT}/examples/geospatial/wells.csv"),
        temp.0.join("wells.csv"),
    )
    .unwrap();
    let config = serde_json::json!({
        "ossie":format!("{ROOT}/examples/geospatial/wells.ossie.yaml"),
        "connections":{"local":{"connector":"csv"}},
        "sources":{"fixtures.geospatial.wells":{"connection":"local","path":"wells.csv","materialization":{"max_age_seconds":60,"max_fill_bytes":65536}}},
        "cache":{"directory":"cache","max_memory_bytes":65536,"max_disk_bytes":1048576}
    });
    std::fs::write(temp.0.join("project.json"), config.to_string()).unwrap();
    success(temp.run(&["--config", "project.json", "--cache-refresh", "wells"]));
    let status = success(temp.run(&["--config", "project.json", "--cache-status"]));
    let key = status.split_whitespace().next().unwrap().to_owned();
    assert!(status.contains("generation="));
    std::fs::rename(temp.0.join("wells.csv"), temp.0.join("offline.csv")).unwrap();
    std::fs::write(temp.0.join(".env"), "not valid env text").unwrap();
    assert_eq!(
        success(temp.run(&["--config", "project.json", "--cache-status"])),
        status
    );
    success(temp.run(&["--config", "project.json", "--cache-invalidate", &key]));
    assert!(
        success(temp.run(&["--config", "project.json", "--cache-status"]))
            .trim()
            .is_empty()
    );
    std::fs::rename(temp.0.join("offline.csv"), temp.0.join("wells.csv")).unwrap();
    std::fs::remove_file(temp.0.join(".env")).unwrap();
    success(temp.run(&[
        "--config",
        "project.json",
        "--bypass-cache",
        "--query",
        "SELECT COUNT(*) FROM wells",
    ]));
    assert!(
        success(temp.run(&["--config", "project.json", "--cache-status"]))
            .trim()
            .is_empty()
    );
}
