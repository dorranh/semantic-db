use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_semantic-db"))
}

fn fixture() -> String {
    format!(
        "wells={}/../../examples/geospatial/wells.csv",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

struct TempDirectory(PathBuf);
impl TempDirectory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("semantic-cli-{}-{name}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sql_needs_no_key_and_llm_reports_missing_configuration() {
    let dir = TempDirectory::new("no-key");
    let output = cli()
        .current_dir(&dir.0)
        .env_remove("OPENAI_API_KEY")
        .args(["--csv", &fixture(), "--query", "SELECT count(*) FROM wells"])
        .output()
        .unwrap();
    assert!(stdout(output).contains("1 row(s)"));
    let output = cli()
        .current_dir(&dir.0)
        .env_remove("OPENAI_API_KEY")
        .args(["--csv", &fixture(), "--ask", "List wells"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("OPENAI_API_KEY"));
}

#[test]
fn rejects_conflicting_modes() {
    for args in [
        vec!["--ask", "wells", "--query", "SELECT 1"],
        vec!["--ask", "wells", "--file", "x.sql"],
        vec!["--ask-views", "wells", "--ask", "wells"],
        vec!["--ask-views", "wells", "--query", "SELECT 1"],
        vec!["--ask-views", "wells", "--file", "x.sql"],
        vec!["--ask-views", "wells", "--inspect", "--config", "x.yaml"],
        vec!["--dry-run"],
    ] {
        assert!(!cli().args(args).output().unwrap().status.success());
    }
}

#[test]
fn dotenv_to_http_to_validated_execution_and_dry_run() {
    let dir = TempDirectory::new("mock");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::write(dir.0.join(".env"), format!("OPENAI_API_KEY=local-test-key\nOPENAI_MODEL=dotenv-model\nOPENAI_BASE_URL=http://{}/v1\n", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || {
        for index in 0..4 {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("mock accept failed: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut data = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let read = socket.read(&mut buffer).unwrap();
                assert!(read > 0);
                data.extend_from_slice(&buffer[..read]);
                if let Some(end) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..end]).to_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if data.len() < end + 4 + length {
                        continue;
                    }
                    assert!(headers.contains("authorization: bearer local-test-key"));
                    let body: serde_json::Value = serde_json::from_slice(&data[end + 4..]).unwrap();
                    // Process environment must override the model in .env.
                    assert_eq!(body["model"], "environment-model");
                    if index >= 2 {
                        let context: serde_json::Value =
                            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap())
                                .unwrap();
                        assert_eq!(context["catalog"][0]["name"], "active_deep_wells");
                        assert_eq!(
                            context["catalog"][0]["description"],
                            "Active wells meeting this example project's depth convention."
                        );
                    }
                    break;
                }
            }
            let proposal = if index < 2 {
                json!({"status":"grounded","query":{
                    "sql":"SELECT well_id FROM wells WHERE basin = 'North Basin' AND total_depth_m >= 2500 ORDER BY well_id",
                    "evidence":[{"phrase":"wells","catalog_reference":"wells","interpretation":"Registered wells"}]
                }})
            } else {
                json!({"status":"selected","selection":{
                    "view":"active_deep_wells","phrase":"active deep wells","columns":["well_id"],
                    "filters":[{"kind":"compare","column":"basin","op":"eq","value":{"kind":"text","text":"North Basin"}}],
                    "order_by":[{"column":"well_id","direction":"asc"}]
                }})
            };
            let body = json!({"choices":[{"message":{"content":proposal.to_string()},"finish_reason":"stop"}]}).to_string();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    for (views_only, dry_run) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut command = cli();
        command
            .current_dir(&dir.0)
            .env_remove("OPENAI_API_KEY")
            .env_remove("OPENAI_BASE_URL")
            .env_remove("OPENAI_TIMEOUT_SECONDS")
            .env_remove("OPENAI_JSON_MODE")
            .env("OPENAI_MODEL", "environment-model");
        if views_only {
            command.args([
                "--config",
                &format!(
                    "{}/../../examples/geospatial/semantic-db.views.yaml",
                    env!("CARGO_MANIFEST_DIR")
                ),
                "--ask-views",
                "List well IDs for active deep wells in North Basin, ordered by well_id",
            ]);
        } else {
            command.args([
                "--csv",
                &fixture(),
                "--ask",
                "List well IDs in North Basin with total_depth_m >= 2500",
            ]);
        }
        if dry_run {
            command.arg("--dry-run");
        }
        let text = stdout(command.output().unwrap());
        assert!(text.contains("SQL (validated"));
        assert_eq!(text.contains("W-001"), !dry_run);
        assert_eq!(text.contains("W-004"), !dry_run);
        assert_eq!(text.contains("2 row(s)"), !dry_run);
        assert_eq!(
            text.contains("Applied authored view definition unchanged:"),
            views_only
        );
    }
    server.join().unwrap();
}

/// Explicit opt-in only: sends the synthetic catalog and requests to the configured
/// provider, using the repository-root .env. Makes 3–6 calls with default repairs.
#[test]
#[ignore = "requires an API key; invokes the configured model and incurs API usage"]
fn live_semantic_evaluation() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cases = [
        (
            "Return only well_id for wells where status is 'active', basin is 'North Basin', and total_depth_m >= 2500, ordered by well_id.",
            "2 row(s)",
        ),
        ("Find deep wells.", "Needs clarification:"),
        (
            "List wells excluding records marked as having uncertain locations.",
            "Unsupported:",
        ),
    ];
    for (request, expected) in cases {
        let text = stdout(
            cli()
                .current_dir(&root)
                .args(["--csv", &fixture(), "--ask", request])
                .output()
                .unwrap(),
        );
        println!("Request: {request}\n{text}");
        assert!(text.contains(expected), "expected {expected}");
        if expected == "2 row(s)" {
            assert!(text.contains("| W-001"));
            assert!(text.contains("| W-004"));
            for id in ["W-002", "W-003", "W-005"] {
                assert!(!text.contains(&format!("| {id}")));
            }
        } else {
            assert!(!text.contains("SQL (validated"));
        }
    }
}
