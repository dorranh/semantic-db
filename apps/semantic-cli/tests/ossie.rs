use std::process::Command;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

#[test]
fn loads_wells_from_project_and_runs_the_existing_query() {
    let output = Command::new(env!("CARGO_BIN_EXE_sdb"))
        .arg("repl")
        .current_dir(ROOT)
        .args([
            "--project-config",
            "examples/geospatial/semantic-db.yaml",
            "--file",
            "examples/geospatial/query.sql",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("W-001") && text.contains("W-004") && text.contains("2 row(s)"));
    assert!(!text.contains("W-002") && !text.contains("W-003") && !text.contains("W-005"));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unenforced_keys")
    );
}

#[test]
fn removed_direct_configuration_flags_are_rejected() {
    for args in [
        vec!["--ossie", "examples/geospatial/wells.ossie.yaml"],
        vec!["--ossie-model", "geospatial_wells"],
        vec!["--source-csv", "source=examples/geospatial/wells.csv"],
        vec!["--csv", "wells=examples/geospatial/wells.csv"],
        vec!["--view", "wells=SELECT 1"],
        vec!["--config", "examples/geospatial/semantic-db.yaml"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sdb"))
            .arg("repl")
            .current_dir(ROOT)
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
    }
}
