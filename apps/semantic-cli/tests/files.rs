#[path = "../../../tests/support/file_sources.rs"]
mod fixtures;
use fixtures::Files;
use serde_json::json;
#[test]
fn release_cli_loads_every_file_format_without_custom_code() {
    let files = Files::new();
    for name in Files::FORMATS {
        let config = files.config(json!({"path":name}));
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_semantic-db"))
            .current_dir(&files.0)
            .args([
                "--config",
                config.to_str().unwrap(),
                "--query",
                "SELECT * FROM selected_products ORDER BY code",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("00123") && text.contains("00456"),
            "{name}: {text}"
        );
    }
}

#[test]
#[cfg(not(feature = "github"))]
fn default_cli_excludes_the_example_github_connector() {
    let files = Files::new();
    let config = files.config_connector("github", json!({"path":"items.csv"}));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_semantic-db"))
        .current_dir(&files.0)
        .args(["--config", config.to_str().unwrap(), "--validate"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("unknown_connector") && error.contains("github"),
        "{error}"
    );
}
