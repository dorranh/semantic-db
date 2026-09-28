use std::{
    io::Write,
    process::{Command, Stdio},
};

fn run_sql(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sdb"))
        .arg("sql")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn positional_file_and_stdin_sql_have_the_same_read_output() {
    let plain = run_sql(&["SELECT 42 AS answer"]);
    assert!(plain.status.success(), "{plain:?}");

    let mut child = Command::new(env!("CARGO_BIN_EXE_sdb"))
        .args(["sql", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"SELECT 42 AS answer")
        .unwrap();
    let piped = child.wait_with_output().unwrap();
    assert!(piped.status.success(), "{piped:?}");
    assert_eq!(piped.stdout, plain.stdout);

    let mut child = Command::new(env!("CARGO_BIN_EXE_sdb"))
        .args(["sql", "--read-report", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"SELECT 42 AS answer")
        .unwrap();
    let reported = child.wait_with_output().unwrap();
    assert!(reported.status.success(), "{reported:?}");
    assert_eq!(reported.stdout, plain.stdout);
    assert!(String::from_utf8_lossy(&reported.stderr).contains("\"requested\""));
}

#[test]
fn repl_is_reserved_for_terminal_sessions() {
    let output = Command::new(env!("CARGO_BIN_EXE_sdb"))
        .arg("repl")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a terminal"));
}

#[test]
fn history_flags_conflict() {
    let output = Command::new(env!("CARGO_BIN_EXE_sdb"))
        .args(["repl", "--no-history", "--history-file", "unused-history"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[cfg(unix)]
#[test]
fn terminal_editor_behaviors() {
    let output = Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/repl_pty.py"))
        .env("SEMANTIC_DB_TEST_BINARY", env!("CARGO_BIN_EXE_sdb"))
        .output()
        .expect("the Unix PTY smoke tests require python3 (standard library only)");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
