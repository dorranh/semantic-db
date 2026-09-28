use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sdb"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_is_scoped_to_each_command() {
    let bare = run(&[]);
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&bare.stdout),
        String::from_utf8_lossy(&bare.stderr)
    );
    for command in [
        "repl", "sql", "ask", "validate", "inspect", "cache", "server", "init",
    ] {
        assert!(help.contains(command), "{help}");
        let output = run(&[command, "--help"]);
        assert!(output.status.success(), "{command}: {output:?}");
    }
    for (command, option) in [
        ("sql", "--plan"),
        ("sql", "--explain"),
        ("ask", "--compile-only"),
        ("validate", "--connect"),
        ("repl", "--no-history"),
        ("server", "--http-port"),
        ("init", "[PATH]"),
    ] {
        let output = run(&[command, "--help"]);
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(option),
            "{command}: {output:?}"
        );
    }
    let repl = String::from_utf8(run(&["repl", "--help"]).stdout).unwrap();
    for removed in [
        "--query <",
        "--write <",
        "--ask <",
        "--validate",
        "--inspect",
        "--cache-status",
    ] {
        assert!(!repl.contains(removed), "{removed} remains on repl");
    }
    let ask = String::from_utf8(run(&["ask", "--help"]).stdout).unwrap();
    assert!(!ask.contains("views-only"));
    for command in ["status", "refresh", "invalidate"] {
        assert!(run(&["cache", command, "--help"]).status.success());
    }
}

#[test]
fn version_identifies_the_build_without_loading_a_project() {
    let expected = env!("CARGO_PKG_VERSION");
    let output = run(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("sdb {expected}")
    );
    for command in [
        "repl", "sql", "ask", "validate", "inspect", "cache", "server", "init",
    ] {
        let output = run(&[command, "--version"]);
        assert!(output.status.success(), "{command}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
    }
}

#[test]
fn rejects_usage_errors_before_loading_a_project() {
    for args in [
        vec!["--query", "SELECT 1"],
        vec!["repl", "--query", "SELECT 1"],
        vec!["repl", "--no-history", "--history-file", "history"],
        vec!["repl", "--project-config", "missing.yaml", "--no-project"],
        vec!["repl", "--validate"],
        vec!["sql"],
        vec!["sql", "SELECT 1", "--file", "query.sql"],
        vec!["sql", "--read-cache", "never", "SELECT 1"],
        vec!["sql", "--read-cache", "max-age=bad", "SELECT 1"],
        vec!["sql", "--query-timeout-seconds", "0", "SELECT 1"],
        vec!["sql", "--query-timeout-seconds", "86401", "SELECT 1"],
        vec!["sql", "--plan", "--explain", "SELECT 1"],
        vec!["sql", "--plan", "--read-cache", "bypass", "SELECT 1"],
        vec!["sql", "--read-report", "--explain", "SELECT 1"],
        vec!["sql", "--plan", "UPDATE items SET id = 2"],
        vec!["sql", "--read-report", "UPDATE items SET id = 2"],
        vec!["sql", "--read-cache", "bypass", "UPDATE items SET id = 2"],
        vec![
            "ask",
            "--compile-only",
            "--read-cache",
            "bypass",
            "show items",
        ],
        vec!["ask", "--views-only", "show items"],
        vec!["validate", "--query", "SELECT 1"],
        vec!["inspect", "--connect"],
        vec!["cache"],
        vec!["server", "--query", "SELECT 1"],
        vec!["server", "--query-timeout-seconds", "0"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
}

#[test]
fn double_dash_ends_options_and_server_discovers_the_project() {
    let sql = run(&["sql", "--", "SELECT 1 AS value"]);
    assert!(sql.status.success(), "{sql:?}");
    assert!(String::from_utf8_lossy(&sql.stdout).contains("1 row(s)"));

    let dir = std::env::temp_dir().join(format!("semantic-server-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("semantic-db.yaml"), "invalid: true\n").unwrap();
    let server = Command::new(env!("CARGO_BIN_EXE_sdb"))
        .current_dir(&dir)
        .arg("server")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!server.status.success());
    assert!(String::from_utf8_lossy(&server.stderr).contains("Using project: semantic-db.yaml"));
}

#[test]
fn sql_dispatch_rejects_mutations_without_a_write_binding_and_multiple_statements() {
    let dir = std::env::temp_dir().join(format!("semantic-sql-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let init = Command::new(env!("CARGO_BIN_EXE_sdb"))
        .current_dir(&dir)
        .arg("init")
        .output()
        .unwrap();
    assert!(init.status.success(), "{init:?}");
    let before = std::fs::read(dir.join("data/items.csv")).unwrap();
    for (sql, expected_error) in [
        (
            "/* leading comment */ UPDATE items SET label = 'changed'",
            "target is read-only",
        ),
        (
            "EXPLAIN UPDATE items SET label = 'changed'",
            "target is read-only",
        ),
        (
            "UPDATE items SET label = 'changed'; SELECT 1",
            "exactly one statement",
        ),
        (
            "SELECT 1; UPDATE items SET label = 'changed'",
            "single SQL statement",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sdb"))
            .current_dir(&dir)
            .args(["sql", sql])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{sql}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected_error),
            "{sql}: {output:?}"
        );
    }
    assert_eq!(std::fs::read(dir.join("data/items.csv")).unwrap(), before);
    let _ = std::fs::remove_dir_all(dir);
}
