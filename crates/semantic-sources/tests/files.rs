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

#[tokio::test]
async fn app_only_files_load_without_ossie_and_remain_read_only() {
    let files = Files::new();
    for name in Files::FORMATS {
        files.write(
            "app.json",
            &json!({
                "connections": {"local": {"connector": "file"}},
                "app_tables": {"items": {"connection": "local", "path": name}},
                "views": {"totals": {"sql_file": "totals.sql"}}
            })
            .to_string(),
        );
        files.write("totals.sql", "SELECT SUM(qty) AS total FROM items");
        let project = Project::from_path(files.0.join("app.json")).unwrap();
        assert!(
            project
                .inspect_project(&Registry::standard())
                .unwrap()
                .model
                .datasets
                .is_empty()
        );
        let loaded = project
            .load(&Registry::standard(), &|_| None)
            .await
            .unwrap();
        let rows = loaded
            .engine
            .query("SELECT total FROM totals")
            .await
            .unwrap();
        assert_eq!(
            rows[0]
                .column(0)
                .as_any()
                .downcast_ref::<datafusion::arrow::array::Int64Array>()
                .unwrap()
                .value(0),
            5,
            "{name}"
        );
        assert!(
            loaded
                .engine
                .prepare_write("DELETE FROM items")
                .await
                .is_err(),
            "{name}"
        );
        let snapshot = loaded
            .engine
            .explain_read(
                "SELECT * FROM totals",
                semantic_engine::ReadOptions {
                    consistency: semantic_engine::ReadConsistency::Snapshot,
                    ..Default::default()
                },
            )
            .await;
        assert!(
            snapshot.is_err(),
            "{name}: file scans must not claim snapshot support"
        );
    }
}

#[tokio::test]
async fn modeled_files_keep_observed_reads_without_transaction_capabilities() {
    let files = Files::new();
    for connector in ["file", "csv"] {
        let project =
            Project::from_path(files.config_connector(connector, json!({"path": "items.csv"})))
                .unwrap();
        let loaded = project
            .load(&Registry::standard(), &|_| None)
            .await
            .unwrap();
        let result = loaded
            .engine
            .execute_read(
                "SELECT code FROM products ORDER BY code",
                vec![],
                Default::default(),
            )
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert!(
            pretty_format_batches(&result.batches)
                .unwrap()
                .to_string()
                .contains("00123")
        );
        assert_eq!(
            result.report.established,
            semantic_engine::ReadConsistency::Observed
        );
        assert!(
            loaded
                .engine
                .explain_read(
                    "SELECT code FROM selected_products",
                    semantic_engine::ReadOptions {
                        consistency: semantic_engine::ReadConsistency::Snapshot,
                        ..Default::default()
                    }
                )
                .await
                .is_err()
        );
        assert!(
            loaded
                .engine
                .prepare_write("DELETE FROM products")
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn csv_explicit_nonnullable_constraints_are_enforced_by_reader() {
    let files = Files::new();
    let path = files.config_connector(
        "csv",
        json!({"path":"items.csv","non_nullable_columns":["CODE"],"null_regex":r"^\\N$"}),
    );
    let project = Project::from_path(path).unwrap();
    let loaded = project
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    assert!(
        !loaded
            .engine
            .catalog()
            .relation("products")
            .unwrap()
            .schema
            .field_with_name("code")
            .unwrap()
            .is_nullable()
    );
    assert_eq!(
        loaded
            .engine
            .query("SELECT code FROM products")
            .await
            .unwrap()
            .iter()
            .map(|b| b.num_rows())
            .sum::<usize>(),
        2
    );
    // Source remains lazy: a later corrupt record must fail rather than merely advertising nonnull metadata.
    files.write("items.csv", "CODE,qty\n\\N,2\n00456,3\n");
    let error = loaded
        .engine
        .query("SELECT code FROM products")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("non-nullable") || error.contains("null"),
        "{error}"
    );
}

#[tokio::test]
async fn csv_nonnullable_names_are_validated_without_fabricating_columns() {
    let files = Files::new();
    for source in [
        json!({"path":"items.csv","non_nullable_columns":["CODE","CODE"]}),
        json!({"path":"items.parquet","non_nullable_columns":["CODE"]}),
    ] {
        let project = Project::from_path(files.config(source)).unwrap();
        assert!(project.inspect(&Registry::standard()).is_err());
    }
    let project = Project::from_path(
        files.config(json!({"path":"items.csv","non_nullable_columns":["missing"]})),
    )
    .unwrap();
    assert!(
        project
            .load(&Registry::standard(), &|_| None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn csv_nonnullable_preflight_rejects_null_beyond_inference_before_count_optimization() {
    let files = Files::new();
    let mut valid = String::from("CODE,qty\n");
    for index in 0..2048 {
        valid.push_str(&format!("{index:05},2\n"));
    }
    for corrupt in ["\\N,3\n", "00456,\\N\n"] {
        files.write("items.csv", &format!("{valid}{corrupt}"));
        let project=Project::from_path(files.config_connector("csv",json!({"path":"items.csv","schema_infer_max_records":1,"non_nullable_columns":["CODE","qty"],"null_regex":r"^\\N$"}))).unwrap();
        // No Engine is published: COUNT optimizations cannot hide either composite key component.
        let error = project
            .load(&Registry::standard(), &|_| None)
            .await
            .err()
            .expect("nonnull preflight must reject corrupt source")
            .to_string();
        assert!(
            error.contains("non-nullable") || error.contains("null"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn csv_custom_null_token_keeps_empty_text_and_handles_compression_directories() {
    use datafusion::arrow::array::{Array, StringArray};
    use std::io::Write;
    let files = Files::new();
    files.write("items.csv", "CODE,qty\n\"\",2\n\\N,3\n\"\\N\",4\n");
    let mut gzip = flate2::write::GzEncoder::new(
        std::fs::File::create(files.0.join("custom.csv.gz")).unwrap(),
        flate2::Compression::default(),
    );
    gzip.write_all(&std::fs::read(files.0.join("items.csv")).unwrap())
        .unwrap();
    gzip.finish().unwrap();
    std::fs::create_dir(files.0.join("custom-directory")).unwrap();
    std::fs::copy(
        files.0.join("custom.csv.gz"),
        files.0.join("custom-directory/part.csv.gz"),
    )
    .unwrap();
    for path in ["items.csv", "custom.csv.gz", "custom-directory/"] {
        let source = if path.ends_with('/') {
            json!({"path":path,"format":"csv","compression":"gzip","extension":".csv.gz","null_regex":r"^\\N$"})
        } else {
            json!({"path":path,"null_regex":r"^\\N$"})
        };
        let project = Project::from_path(files.config(source)).unwrap();
        let loaded = project
            .load(&Registry::standard(), &|_| None)
            .await
            .unwrap();
        let batches = loaded
            .engine
            .query("SELECT code FROM products ORDER BY qty")
            .await
            .unwrap();
        let values = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(values.value(0), "", "{path}");
        assert!(!values.is_null(0));
        assert!(values.is_null(1), "{path}");
        assert!(
            values.is_null(2),
            "quoted token follows Arrow null semantics: {path}"
        );
    }
    files.write("inferred.csv", "CODE,qty\nvalue,1\n\"\",2\n\\N,3\n");
    files.write("inferred-project.json",&json!({"connections":{"local":{"connector":"csv"}},"app_tables":{"raw":{"connection":"local","path":"inferred.csv","null_regex":r"^\\N$"}}}).to_string());
    let project = Project::from_path(files.0.join("inferred-project.json")).unwrap();
    let loaded = project
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    let result = loaded
        .engine
        .query("SELECT \"CODE\" FROM raw ORDER BY qty")
        .await
        .unwrap();
    let array = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(array.value(1), "");
    assert!(!array.is_null(1));
    assert!(array.is_null(2));
    let invalid =
        Project::from_path(files.config(json!({"path":"items.csv","null_regex":"["}))).unwrap();
    assert!(invalid.inspect(&Registry::standard()).is_err());
}

#[tokio::test]
async fn csv_null_adapter_honors_delimiter_quote_escape_and_header_options() {
    use datafusion::arrow::array::{Array, StringArray};
    let files = Files::new();
    files.write("custom.csv", "CODE;qty\n'left\\'right';2\nNULL;3\n");
    let project=Project::from_path(files.config(json!({"path":"custom.csv","delimiter":";","quote":"'","escape":"\\","null_regex":"^NULL$"}))).unwrap();
    let loaded = project
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    let batches = loaded
        .engine
        .query("SELECT code FROM products ORDER BY qty")
        .await
        .unwrap();
    let array = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(array.value(0), "left'right");
    assert!(array.is_null(1));
    // Header-less inference has generated physical names; direct app table loading checks that path.
    files.write("headerless.csv", "alpha;2\nNULL;3\n");
    files.write("headerless-project.json",&json!({"connections":{"local":{"connector":"csv"}},"app_tables":{"raw":{"connection":"local","path":"headerless.csv","has_header":false,"delimiter":";","null_regex":"^NULL$","physical_types":{"column_1":"Utf8","column_2":"Int64"}}}}).to_string());
    let project = Project::from_path(files.0.join("headerless-project.json")).unwrap();
    let loaded = project
        .load(&Registry::standard(), &|_| None)
        .await
        .unwrap();
    let result = loaded
        .engine
        .query("SELECT column_1 FROM raw ORDER BY column_2")
        .await
        .unwrap();
    let array = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(array.value(0), "alpha");
    assert!(array.is_null(1));
}
