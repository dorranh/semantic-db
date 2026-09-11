use std::process::Command;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

#[test]
fn loads_wells_from_ossie_and_runs_the_existing_query() {
    let output = Command::new(env!("CARGO_BIN_EXE_semantic-db"))
        .current_dir(ROOT)
        .args([
            "--ossie",
            "examples/geospatial/wells.ossie.yaml",
            "--source-csv",
            "fixtures.geospatial.wells=examples/geospatial/wells.csv",
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
fn fails_for_missing_bindings_or_invalid_cli_combinations() {
    for args in [
        vec![
            "--ossie",
            "examples/geospatial/wells.ossie.yaml",
            "--query",
            "SELECT * FROM wells",
        ],
        vec![
            "--source-csv",
            "source=examples/geospatial/wells.csv",
            "--query",
            "SELECT 1",
        ],
        vec!["--ossie-model", "geospatial_wells", "--query", "SELECT 1"],
        vec![
            "--ossie",
            "examples/geospatial/wells.ossie.yaml",
            "--csv",
            "wells=examples/geospatial/wells.csv",
            "--query",
            "SELECT 1",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_semantic-db"))
            .current_dir(ROOT)
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
