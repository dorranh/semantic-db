use datafusion::arrow::datatypes::{DataType, Field, Schema};
use serde_json::json;
use std::{
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "semantic-cli-deferred-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_sdb"))
            .current_dir(&self.0)
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

#[test]
fn configured_deferred_project_plans_before_opening_missing_source() {
    let temp = Temp::new();
    let schema = Schema::new(vec![Field::new("id", DataType::Int64, true)]);
    let config = temp.0.join("semantic-db.json");
    std::fs::write(
        &config,
        json!({
            "deferred_read_only": true,
            "connections": {"local": {"connector": "csv"}},
            "app_tables": {"items": {
                "connection": "local",
                "path": "missing.csv",
                "recorded_schema": schema
            }}
        })
        .to_string(),
    )
    .unwrap();
    let path = config.to_str().unwrap();
    let planned = temp.run(&[
        "sql",
        "--project-config",
        path,
        "SELECT id FROM items",
        "--plan",
    ]);
    assert!(
        planned.status.success(),
        "{}",
        String::from_utf8_lossy(&planned.stderr)
    );
    assert!(String::from_utf8_lossy(&planned.stdout).contains("Projection"));
    let executed = temp.run(&["sql", "--project-config", path, "SELECT id FROM items"]);
    assert!(!executed.status.success());
    assert!(
        String::from_utf8_lossy(&executed.stderr).contains("missing.csv"),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}
