use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::{Value, json};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "semantic-cli-prepared-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let init = Command::new(env!("CARGO_BIN_EXE_sdb"))
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(init.status.success(), "{init:?}");
        Self(path)
    }
    fn request(&self, value: &Value) -> PathBuf {
        let path = self.0.join("prepared.json");
        std::fs::write(&path, value.to_string()).unwrap();
        path
    }
    fn run(&self, args: &[&str]) -> Output {
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

fn request() -> Value {
    json!({
        "query": {
            "version":1,
            "input":{"relation":"items","instance":"i"},
            "requirements":[
                {"id":"active","source_text":"status equals parameter","operation":{
                    "kind":"filter","predicate":{
                        "kind":"compare_parameter",
                        "field":{"instance":"i","field":"status"},
                        "operator":"eq","parameter":"status"
                    }
                }},
                {"id":"id","source_text":"item IDs","operation":{
                    "kind":"project","field":{"instance":"i","field":"id"},"alias":"id"
                }}
            ],
            "unresolved":[]
        },
        "declarations":[{"name":"status","value_type":"utf8"}],
        "values":{"status":{"type":"utf8","value":"active"}}
    })
}

#[test]
fn prepared_json_compiles_without_model_and_executes_only_when_requested() {
    let temp = Temp::new();
    let path = temp.request(&request());
    let path = path.to_str().unwrap();
    let compiled = temp.run(&["compile-prepared", "--file", path]);
    assert!(compiled.status.success(), "{compiled:?}");
    let output = String::from_utf8(compiled.stdout).unwrap();
    assert!(output.contains("SQL:") && output.contains("$1"), "{output}");
    assert!(output.contains("Parameter types:"));
    assert!(!output.contains("row(s)"));
    assert!(!output.contains("'active'"));
    let executed = temp.run(&["compile-prepared", "--file", path, "--execute"]);
    assert!(executed.status.success(), "{executed:?}");
    let output = String::from_utf8(executed.stdout).unwrap();
    assert!(output.contains("2 row(s)"), "{output}");
    assert!(output.contains("1") && output.contains("3"));
}

#[test]
fn bad_prepared_values_fail_without_exposing_contents() {
    let temp = Temp::new();
    let mut wrong = request();
    wrong["values"]["status"] = json!({"type":"int64","value":42});
    let path = temp.request(&wrong);
    let result = temp.run(&["compile-prepared", "--file", path.to_str().unwrap()]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("parameter_type"));

    let mut malformed = request();
    malformed["values"]["status"] = json!({"type":"int64","value":"private-secret"});
    let path = temp.request(&malformed);
    let result = temp.run(&["compile-prepared", "--file", path.to_str().unwrap()]);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("invalid prepared row JSON"), "{stderr}");
    assert!(!stderr.contains("private-secret"));
}
