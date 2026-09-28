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
        Command::new(env!("CARGO_BIN_EXE_sdb"))
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
    let inspected = success(temp.run(&["inspect", "--project-config", &config]));
    assert!(
        inspected.contains("geospatial_wells")
            && inspected.contains("total_depth_m")
            && inspected.contains("csv")
    );
    assert!(
        success(temp.run(&["validate", "--project-config", &config]))
            .contains("Offline validation passed")
    );
    assert!(
        success(temp.run(&["validate", "--project-config", &config, "--connect"]))
            .contains("Connected schema validation passed")
    );
    let rows = success(temp.run(&[
        "sql",
        "--project-config",
        &config,
        "--file",
        &format!("{ROOT}/examples/geospatial/query.sql"),
    ]));
    assert!(rows.contains("W-001") && rows.contains("W-004") && rows.contains("2 row(s)"));
    let reported = temp.run(&[
        "sql",
        "--project-config",
        &config,
        "--file",
        &format!("{ROOT}/examples/geospatial/query.sql"),
        "--read-report",
    ]);
    assert!(reported.status.success());
    assert!(String::from_utf8_lossy(&reported.stdout).contains("2 row(s)"));
    assert!(String::from_utf8_lossy(&reported.stderr).contains("\"requested\""));
    let explanation = success(temp.run(&[
        "sql",
        "--project-config",
        &config,
        "--file",
        &format!("{ROOT}/examples/geospatial/query.sql"),
        "--explain",
    ]));
    assert!(explanation.contains("\"dependencies\""));
    assert!(!explanation.contains("W-001"));
    assert!(
        success(temp.run(&[
            "sql",
            "--project-config",
            &config,
            "SELECT COUNT(*) FROM wells",
            "--read-cache",
            "max-age=300",
        ]))
        .contains("1 row(s)")
    );
    let planned = success(temp.run(&[
        "sql",
        "--project-config",
        &config,
        "SELECT * FROM wells",
        "--plan",
    ]));
    assert!(planned.contains("Projection") && !planned.contains("W-001"));
}

#[test]
fn project_views_inspect_and_reload_from_another_working_directory() {
    let temp = Temp::new();
    let config = format!("{ROOT}/examples/geospatial/semantic-db.views.yaml");
    std::fs::write(temp.0.join(".env"), "not a valid env file\n").unwrap();
    let inspected = success(temp.run(&["inspect", "--project-config", &config]));
    assert!(inspected.contains("View deep_wells") && inspected.contains("views/deep_wells.sql"));
    assert!(inspected.contains("dependencies: deep_wells"));
    assert!(inspected.contains("example project's depth convention"));
    assert!(
        inspected.find("View deep_wells").unwrap()
            < inspected.find("View active_deep_wells").unwrap()
    );
    assert!(
        success(temp.run(&["validate", "--project-config", &config]))
            .contains("view columns/types")
    );
    std::fs::remove_file(temp.0.join(".env")).unwrap();
    assert!(
        success(temp.run(&["validate", "--project-config", &config, "--connect"]))
            .contains("Connected schema validation passed")
    );
    // Separate processes reload the authored definitions on every startup.
    for _ in 0..2 {
        let rows = success(temp.run(&[
            "sql",
            "--project-config",
            &config,
            "SELECT well_id FROM active_deep_wells WHERE basin = 'North Basin' ORDER BY well_id",
        ]));
        assert!(rows.contains("W-001") && rows.contains("W-004") && rows.contains("2 row(s)"));
    }
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
        let output = temp.run(&["validate", "--project-config", "project.yaml"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
    }
    for args in [
        vec!["validate"],
        vec!["validate", "--connect"],
        vec!["inspect", "--project-config", "x", "SELECT 1"],
        vec!["validate", "--project-config", "x", "--ask", "hello"],
        vec!["sql", "--project-config", "x", "--plan"],
    ] {
        assert!(!temp.run(&args).status.success(), "{args:?}");
    }
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
    success(temp.run(&[
        "cache",
        "refresh",
        "--project-config",
        "project.json",
        "wells",
    ]));
    let status = success(temp.run(&["cache", "status", "--project-config", "project.json"]));
    let key = status.split_whitespace().next().unwrap().to_owned();
    assert!(status.contains("generation="));
    std::fs::rename(temp.0.join("wells.csv"), temp.0.join("offline.csv")).unwrap();
    std::fs::write(temp.0.join(".env"), "not valid env text").unwrap();
    assert_eq!(
        success(temp.run(&["cache", "status", "--project-config", "project.json"])),
        status
    );
    success(temp.run(&[
        "cache",
        "invalidate",
        "--project-config",
        "project.json",
        &key,
    ]));
    assert!(
        success(temp.run(&["cache", "status", "--project-config", "project.json"]))
            .trim()
            .is_empty()
    );
    std::fs::rename(temp.0.join("offline.csv"), temp.0.join("wells.csv")).unwrap();
    std::fs::remove_file(temp.0.join(".env")).unwrap();
    success(temp.run(&[
        "sql",
        "--project-config",
        "project.json",
        "--read-cache",
        "bypass",
        "SELECT COUNT(*) FROM wells",
    ]));
    assert!(
        success(temp.run(&["cache", "status", "--project-config", "project.json"]))
            .trim()
            .is_empty()
    );
}
