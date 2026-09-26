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
        vec!["--config", "missing.yaml"],
        vec!["server"],
        vec!["server", "--config", "missing.yaml", "--query", "SELECT 1"],
        vec![
            "server",
            "--config",
            "missing.yaml",
            "--query-timeout-seconds",
            "0",
        ],
        vec!["repl", "--port", "5544"],
        vec!["repl", "--query", "SELECT 1", "--file", "query.sql"],
        vec!["repl", "--no-history", "--history-file", "history"],
        vec!["repl", "--connect"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
}
