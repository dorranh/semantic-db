#[path = "../../../tests/support/file_sources.rs"]
mod fixtures;
use datafusion::arrow::util::pretty::pretty_format_batches;
use fixtures::Files;
use semantic_sources::{Project, Registry};
use serde_json::json;

#[tokio::test]
async fn all_formats_preserve_identifiers_and_support_authored_views() {
    let files = Files::new();
    for name in Files::FORMATS {
        let project = Project::from_path(files.config(json!({"path":name}))).unwrap();
        project.inspect(&Registry::standard()).unwrap();
        let loaded = project
            .load(&Registry::standard(), &|_| {
                panic!("local files need no secrets")
            })
            .await
            .unwrap();
        let rows = loaded
            .engine
            .query("SELECT * FROM selected_products ORDER BY code")
            .await
            .unwrap();
        let output = pretty_format_batches(&rows).unwrap().to_string();
        assert!(
            output.contains("00123") && output.contains("00456"),
            "{name}: {output}"
        );
    }
}

#[tokio::test]
async fn explicit_connectors_read_extensionless_files() {
    let files = Files::new();
    for (connector, source) in [
        ("csv", "items.csv"),
        ("json", "items.jsonl"),
        ("parquet", "items.parquet"),
        ("avro", "items.avro"),
        ("arrow", "items.arrow"),
    ] {
        std::fs::copy(files.0.join(source), files.0.join("records")).unwrap();
        let project =
            Project::from_path(files.config_connector(connector, json!({"path":"records"})))
                .unwrap();
        let loaded = project
            .load(&Registry::standard(), &|_| None)
            .await
            .unwrap();
        let rows = loaded.engine.query("SELECT * FROM products").await.unwrap();
        assert_eq!(
            rows.iter().map(|batch| batch.num_rows()).sum::<usize>(),
            2,
            "{connector}"
        );
    }
}

#[tokio::test]
async fn compatible_directories_and_explicit_format_work() {
    let files = Files::new();
    for (format, filename) in [
        ("csv", "items.csv"),
        ("json", "items.jsonl"),
        ("parquet", "items.parquet"),
        ("avro", "items.avro"),
        ("arrow", "items.arrow"),
    ] {
        let dir = files.0.join(format);
        std::fs::create_dir(&dir).unwrap();
        for prefix in ["a", "b"] {
            std::fs::copy(
                files.0.join(filename),
                dir.join(format!("{prefix}-{filename}")),
            )
            .unwrap();
        }
        let extension = if format == "json" {
            ".jsonl"
        } else {
            &format!(".{format}")
        };
        let project = Project::from_path(
            files
                .config(json!({"path":format!("{format}/"),"format":format,"extension":extension})),
        )
        .unwrap();
        let loaded = project
            .load(&Registry::standard(), &|_| None)
            .await
            .unwrap();
        let rows = loaded.engine.query("SELECT * FROM products").await.unwrap();
        assert_eq!(
            rows.iter().map(|b| b.num_rows()).sum::<usize>(),
            4,
            "{format}"
        );
    }
}

#[tokio::test]
async fn guided_csv_keeps_nulls_and_infers_undeclared_fields() {
    let files = Files::new();
    files.write("items.csv", "CODE;qty\n00123;2\n;3\n");
    let mut datasets = Files::datasets();
    datasets[0]["fields"][1]
        .as_object_mut()
        .unwrap()
        .remove("datatype");
    files.model(datasets);
    let project =
        Project::from_path(files.config(json!({"path":"items.csv","delimiter":";"}))).unwrap();
    let loaded = project
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    let rows = loaded
        .engine
        .query("SELECT code, qty FROM products ORDER BY qty")
        .await
        .unwrap();
    assert_eq!(rows[0].column(0).null_count(), 1);
    assert!(
        pretty_format_batches(&rows)
            .unwrap()
            .to_string()
            .contains("00123")
    );
}

#[test]
fn conflicting_declarations_fail_offline_before_source_io() {
    let files = Files::new();
    let mut datasets = Files::datasets();
    let mut other = datasets[0].clone();
    other["name"] = json!("other_products");
    other["fields"][0]["datatype"] = json!("Integer");
    datasets.as_array_mut().unwrap().push(other);
    files.model(datasets);
    let project = Project::from_path(files.config(json!({"path":"missing.csv"}))).unwrap();
    let error = project
        .inspect(&Registry::standard())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("conflicting Ossie types") && error.contains("CODE"),
        "{error}"
    );
}

#[tokio::test]
async fn incompatible_values_fail_instead_of_becoming_null_or_truncated() {
    let files = Files::new();
    for (name, content) in [
        ("bad.csv", "CODE,qty\n00123,2.5\n"),
        ("fractional.jsonl", "{\"CODE\":\"00123\",\"qty\":2.5}\n"),
        (
            "bad.jsonl",
            "{\"CODE\":\"00123\",\"qty\":\"not-an-integer\"}\n",
        ),
    ] {
        files.write(name, content);
        let project = Project::from_path(files.config(json!({"path":name}))).unwrap();
        let loaded = project
            .load(&Registry::standard(), &|_| None)
            .await
            .unwrap();
        assert!(
            loaded.engine.query("SELECT * FROM products").await.is_err(),
            "{name}"
        );
    }
}

#[tokio::test]
async fn embedded_schema_is_validated_without_reinterpretation() {
    let files = Files::new();
    let mut datasets = Files::datasets();
    datasets[0]["fields"][0]["datatype"] = json!("Integer");
    files.model(datasets);
    for name in ["items.parquet", "items.avro", "items.arrow"] {
        let project = Project::from_path(files.config(json!({"path":name}))).unwrap();
        let error = project
            .load(&Registry::standard(), &|_| None)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("type_mismatch"), "{error}");
    }
}

#[test]
fn ambiguous_locations_and_inapplicable_options_are_actionable() {
    let files = Files::new();
    for (options, message) in [
        (json!({"path":"data/"}), "cannot infer file format"),
        (json!({"path":"s3://bucket/items.csv"}), "only local files"),
        (
            json!({"path":"items.parquet","delimiter":";"}),
            "CSV parsing options",
        ),
        (
            json!({"path":"items.csv","schema_infer_max_records":0}),
            "must be positive",
        ),
        (
            json!({"path":"items.csv","physical_types":{"CODE":"Int64"}}),
            "conflicts with Ossie",
        ),
    ] {
        let project = Project::from_path(files.config(options)).unwrap();
        let error = project
            .inspect(&Registry::standard())
            .unwrap_err()
            .to_string();
        assert!(error.contains(message), "{error}");
    }
}

#[tokio::test]
async fn cross_format_join_uses_the_same_semantic_identifiers() {
    let files = Files::new();
    let mut datasets = Files::datasets();
    let mut other = datasets[0].clone();
    other["name"] = json!("archive");
    other["source"] = json!("local.archive");
    datasets.as_array_mut().unwrap().push(other);
    files.model(datasets);
    let config = files.config(json!({"path":"items.csv"}));
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    value["sources"]["local.archive"] = json!({"connection":"local","path":"items.parquet"});
    std::fs::write(&config, value.to_string()).unwrap();
    let loaded = Project::from_path(config)
        .unwrap()
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    let rows = loaded
        .engine
        .query("SELECT p.code FROM products p JOIN archive a ON p.code = a.code ORDER BY p.code")
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|b| b.num_rows()).sum::<usize>(), 2);
}

#[tokio::test]
async fn json_rejects_fractional_values_beyond_the_inference_sample() {
    let files = Files::new();
    let mut data = "{\"CODE\":\"00123\",\"qty\":2}\n".repeat(1025);
    data.push_str("{\"CODE\":\"00456\",\"qty\":2.5}\n");
    files.write("late.jsonl", &data);
    let project = Project::from_path(
        files.config(json!({"path":"late.jsonl", "schema_infer_max_records":1})),
    )
    .unwrap();
    let loaded = project
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    let error = loaded
        .engine
        .query("SELECT * FROM products")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("refusing lossy conversion"), "{error}");
}

#[tokio::test]
async fn explicit_decimal_scale_and_shared_compatible_declarations() {
    let files = Files::new();
    files.write("items.csv", "CODE,qty\n00123,2.50\n00456,3.25\n");
    let mut datasets = Files::datasets();
    datasets[0]["fields"][1]["datatype"] = json!("Decimal");
    let mut other = datasets[0].clone();
    other["name"] = json!("other_products");
    datasets.as_array_mut().unwrap().push(other);
    files.model(datasets);
    let project = Project::from_path(files.config(json!({"path":"items.csv"}))).unwrap();
    assert!(
        project
            .inspect(&Registry::standard())
            .unwrap_err()
            .to_string()
            .contains("precision/scale")
    );
    let project = Project::from_path(
        files.config(json!({"path":"items.csv","physical_types":{"qty":{"Decimal128":[18,2]}}})),
    )
    .unwrap();
    let loaded = project
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    let rows = loaded
        .engine
        .query("SELECT code, qty FROM other_products ORDER BY code")
        .await
        .unwrap();
    let output = pretty_format_batches(&rows).unwrap().to_string();
    assert!(
        output.contains("00123") && output.contains("2.50") && output.contains("3.25"),
        "{output}"
    );
}

#[tokio::test]
async fn timestamp_guidance_preserves_nanosecond_fraction() {
    let files = Files::new();
    files.write(
        "timestamp.csv",
        "CODE,qty\n2026-09-15T12:30:45.123456789,2\n",
    );
    let mut datasets = Files::datasets();
    datasets[0]["fields"][0]["datatype"] = json!("DateTime");
    files.model(datasets);
    let project = Project::from_path(files.config(json!({"path":"timestamp.csv"}))).unwrap();
    let loaded = project
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    let rows = loaded
        .engine
        .query("SELECT code FROM products")
        .await
        .unwrap();
    let values = rows[0]
        .column(0)
        .as_any()
        .downcast_ref::<datafusion::arrow::array::TimestampNanosecondArray>()
        .unwrap();
    assert_eq!(values.value(0) % 1_000_000_000, 123_456_789);
}
