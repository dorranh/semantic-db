use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "semantic-init-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_semantic-db"))
            .current_dir(&self.0)
            .env_remove("OPENAI_API_KEY")
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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

fn assert_runnable(temp: &Temp, project: &Path) {
    let config = project.join("semantic-db.yaml");
    let config = config.to_str().unwrap();
    let inspected = success(temp.run(&["--config", config, "--inspect"]));
    assert!(inspected.contains("starter") && inspected.contains("View active_items"));
    assert!(
        success(temp.run(&["--config", config, "--validate"]))
            .contains("Offline validation passed")
    );
    assert!(
        success(temp.run(&["--config", config, "--validate", "--connect"]))
            .contains("Connected schema validation passed")
    );
    let rows = success(temp.run(&[
        "--config",
        config,
        "--query",
        "SELECT * FROM active_items ORDER BY id",
    ]));
    assert!(rows.contains("First item") && rows.contains("Third item"));
    assert!(!rows.contains("Second item"));
    assert!(rows.contains("2 row(s)"));
    assert!(project.join(".env.example").is_file());
    assert!(
        fs::read_to_string(project.join(".gitignore"))
            .unwrap()
            .contains("/.env\n")
    );
    assert!(!project.join(".env").exists());
}

#[test]
fn initializes_current_directory_and_nested_destination_with_runnable_views() {
    for destination in [None, Some("nested/my project")] {
        let temp = Temp::new();
        let args = match destination {
            Some(path) => vec!["init", path],
            None => vec!["init"],
        };
        let output = success(temp.run(&args));
        assert!(output.contains("--config semantic-db.yaml --validate --connect"));
        assert_runnable(&temp, &temp.0.join(destination.unwrap_or(".")));
    }
}

#[test]
fn preserves_existing_files_and_extends_gitignore() {
    let temp = Temp::new();
    fs::write(temp.0.join(".gitignore"), "target/").unwrap();
    fs::write(temp.0.join(".env.example"), "# existing template\n").unwrap();
    fs::write(temp.0.join(".env"), "EXISTING=value\n").unwrap();
    fs::write(temp.0.join("README.md"), "Existing project\n").unwrap();
    fs::create_dir(temp.0.join("views")).unwrap();
    fs::write(temp.0.join("views/custom.sql"), "SELECT 1").unwrap();
    success(temp.run(&["init"]));
    for (path, contents) in [
        (".env.example", "# existing template\n"),
        (".env", "EXISTING=value\n"),
        ("README.md", "Existing project\n"),
        ("views/custom.sql", "SELECT 1"),
    ] {
        assert_eq!(fs::read_to_string(temp.0.join(path)).unwrap(), contents);
    }
    let ignore = fs::read_to_string(temp.0.join(".gitignore")).unwrap();
    assert!(ignore.starts_with("target/\n# Semantic DB"));
    assert!(ignore.ends_with("!/.env.example\n"));

    // A second invocation fails before changing the existing project or ignore rules.
    let config = fs::read(temp.0.join("semantic-db.yaml")).unwrap();
    let output = temp.run(&["init"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("refusing to overwrite"));
    assert_eq!(fs::read(temp.0.join("semantic-db.yaml")).unwrap(), config);
    assert_eq!(
        fs::read_to_string(temp.0.join(".gitignore")).unwrap(),
        ignore
    );
}

#[test]
fn conflicts_are_reported_before_any_scaffold_is_written() {
    for conflict in [
        "model.ossie.yaml",
        "views/active_items.sql",
        "data",
        ".gitignore",
    ] {
        let temp = Temp::new();
        let path = temp.0.join(conflict);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if conflict == ".gitignore" {
            fs::create_dir(&path).unwrap();
        } else {
            fs::write(&path, "keep me").unwrap();
        }
        let output = temp.run(&["init"]);
        assert!(!output.status.success(), "{conflict}");
        assert!(String::from_utf8_lossy(&output.stderr).contains(conflict));
        assert!(!temp.0.join("semantic-db.yaml").exists());
        assert!(!temp.0.join(".env.example").exists());
        if conflict != ".gitignore" {
            assert_eq!(fs::read_to_string(path).unwrap(), "keep me");
        }
    }
    let temp = Temp::new();
    fs::write(temp.0.join("project"), "keep me").unwrap();
    assert!(!temp.run(&["init", "project"]).status.success());
    assert_eq!(
        fs::read_to_string(temp.0.join("project")).unwrap(),
        "keep me"
    );
}

#[cfg(unix)]
#[test]
fn refuses_symlink_conflicts_including_dangling_links() {
    use std::os::unix::fs::symlink;
    for conflict in [
        "semantic-db.yaml",
        "data",
        "views",
        ".gitignore",
        ".env.example",
    ] {
        let temp = Temp::new();
        symlink(temp.0.join("missing"), temp.0.join(conflict)).unwrap();
        assert!(!temp.run(&["init"]).status.success(), "{conflict}");
        assert!(!temp.0.join("missing").exists());
        assert!(!temp.0.join("model.ossie.yaml").exists());
    }
}

#[test]
fn init_help_and_argument_conflicts_do_not_create_files() {
    let temp = Temp::new();
    assert!(success(temp.run(&["--help"])).contains("init"));
    assert!(success(temp.run(&["init", "--help"])).contains("[PATH]"));
    for args in [
        vec!["--query", "SELECT 1", "init"],
        vec!["--config", "missing.yaml", "init"],
        vec!["init", "--query", "SELECT 1"],
        vec!["init", "one", "two"],
    ] {
        assert!(!temp.run(&args).status.success(), "{args:?}");
    }
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 0);
}
