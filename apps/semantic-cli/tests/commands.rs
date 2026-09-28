use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sdb"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_describes_explicit_commands_and_their_options() {
    let bare = run(&[]);
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&bare.stdout),
        String::from_utf8_lossy(&bare.stderr)
    );
    for command in ["repl", "server", "init"] {
        assert!(help.contains(command), "{help}");
    }
    for (command, option) in [
        ("repl", "--query"),
        ("server", "--http-port"),
        ("init", "[PATH]"),
    ] {
        let output = run(&[command, "--help"]);
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains(option));
    }
}

#[test]
fn repl_help_puts_project_first_and_interactive_preferences_last() {
    let output = run(&["repl", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    let headings = [
        "Project and validation:",
        "Queries and writes:",
        "Read options:",
        "Execution limits:",
        "Cache maintenance:",
        "Interactive preferences:",
    ];
    for pair in headings.windows(2) {
        assert!(help.find(pair[0]).unwrap() < help.find(pair[1]).unwrap());
    }
    assert!(help.contains("--project-config <PATH>"));
    for removed in [
        "--config",
        "--csv",
        "--source-csv",
        "--ossie",
        "--view",
        "--bypass-cache",
    ] {
        assert!(!help.contains(removed), "{removed} remains in help");
    }

    let server = run(&["server", "--help"]);
    assert!(server.status.success());
    let help = String::from_utf8(server.stdout).unwrap();
    assert!(help.contains("--project-config <PATH>"));
    assert!(!help.contains("--config"));
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
    for command in ["repl", "server", "init"] {
        let output = run(&[command, "--version"]);
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
    }
}

#[test]
fn arguments_are_scoped_to_their_command_and_conflicts_still_apply() {
    for args in [
        vec!["--query", "SELECT 1"],
        vec!["--project-config", "missing.yaml"],
        vec!["server"],
        vec!["server", "--config", "missing.yaml"],
        vec![
            "server",
            "--project-config",
            "missing.yaml",
            "--query",
            "SELECT 1",
        ],
        vec![
            "server",
            "--project-config",
            "missing.yaml",
            "--query-timeout-seconds",
            "0",
        ],
        vec!["repl", "--port", "5544"],
        vec!["repl", "--query", "SELECT 1", "--file", "query.sql"],
        vec!["repl", "--no-history", "--history-file", "history"],
        vec!["repl", "--connect"],
        vec!["repl", "--project-config", "missing.yaml", "--no-project"],
        vec!["repl", "--validate", "--write", "UPDATE items SET id = 2"],
        vec!["repl", "--cache-status", "--query", "SELECT 1"],
        vec!["repl", "--inspect", "--cache-status"],
        vec!["repl", "--cache-refresh", "items", "--cache-status"],
        vec!["repl", "--read-cache", "never", "--query", "SELECT 1"],
        vec!["repl", "--read-cache", "max-age=bad", "--query", "SELECT 1"],
        vec![
            "repl",
            "--read-cache",
            "bypass",
            "--query",
            "SELECT 1",
            "--dry-run",
        ],
        vec![
            "repl",
            "--read-report",
            "--explain-read",
            "--query",
            "SELECT 1",
        ],
        vec!["repl", "--read-report", "--validate"],
        vec![
            "repl",
            "--query-timeout-seconds",
            "0",
            "--query",
            "SELECT 1",
        ],
        vec![
            "repl",
            "--query-timeout-seconds",
            "86401",
            "--query",
            "SELECT 1",
        ],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
}
