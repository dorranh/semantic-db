use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn repl_flags_do_not_change_batch_or_piped_output() {
    let run = |flags: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_semantic-db"))
            .args(flags)
            .args(["--query", "SELECT 42 AS answer"])
            .output()
            .unwrap()
    };
    let plain = run(&[]);
    let flagged = run(&["--no-color", "--no-history"]);
    assert!(plain.status.success() && flagged.status.success());
    assert_eq!(plain.stdout, flagged.stdout);
    assert_eq!(plain.stderr, flagged.stderr);
    assert!(!flagged.stdout.contains(&0x1b));

    let mut child = Command::new(env!("CARGO_BIN_EXE_semantic-db"))
        .args(["--no-history", "--no-color"])
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
    assert!(piped.status.success());
    assert_eq!(piped.stdout, plain.stdout);
    assert!(piped.stderr.is_empty());
}

#[test]
fn history_flags_conflict() {
    let output = Command::new(env!("CARGO_BIN_EXE_semantic-db"))
        .args([
            "--no-history",
            "--history-file",
            "unused-history",
            "--query",
            "SELECT 1",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}

#[cfg(unix)]
#[test]
fn terminal_editor_behaviors() {
    let output = Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/repl_pty.py"))
        .env("SEMANTIC_DB_TEST_BINARY", env!("CARGO_BIN_EXE_semantic-db"))
        .output()
        .expect("the Unix PTY smoke tests require python3 (standard library only)");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
