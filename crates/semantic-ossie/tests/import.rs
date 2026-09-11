use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Int32Array, StringArray},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{DataType, Field, Schema};
use semantic_compiler::{
    Compiler, GroundingOutcome, catalog_context,
    provider::{Message, ModelProvider, ProviderError},
};
use semantic_engine::{Engine, TableProvider, pretty_format_batches};
use semantic_ossie::{ImportError, OssieDocument, SCHEMA_COMMIT, SourceBindings};
use serde_json::{Value, json};

const WELLS: &str = include_str!("../../../examples/geospatial/wells.ossie.yaml");
const CSV: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/geospatial/wells.csv"
);
const QUERY: &str = include_str!("../../../examples/geospatial/query.sql");

fn simple() -> Value {
    json!({"version":"0.2.0.dev0", "semantic_model":[{
        "name":"model", "datasets":[{"name":"items", "source":"private:items",
        "fields":[{"name":"id", "datatype":"Integer", "expression":{"dialects":[{"dialect":"ANSI_SQL", "expression":"id"}]}}]}]
    }]})
}

fn table() -> Arc<dyn TableProvider> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, true),
        Field::new("secret", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![Some(1), Some(1), None])),
            Arc::new(StringArray::from(vec!["a", "b", "c"])),
        ],
    )
    .unwrap();
    Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap())
}

fn bindings() -> SourceBindings {
    let mut bindings = SourceBindings::new();
    bindings.bind("private:items", table()).unwrap();
    bindings
}

fn error_code(result: Result<semantic_ossie::ImportedCatalog, ImportError>, code: &str) {
    let err = result.err().expect("must reject input");
    let ImportError::Diagnostics(diagnostics) = err else {
        panic!("wrong error: {err}")
    };
    assert!(
        diagnostics.iter().any(|d| d.code == code),
        "{diagnostics:?}"
    );
    assert!(diagnostics.iter().all(|d| d.path.starts_with('/')));
}

#[tokio::test]
async fn wells_import_matches_direct_csv_and_composes_views() {
    let document = OssieDocument::parse(WELLS).unwrap();
    assert_eq!(document.original_text(), WELLS);
    assert_eq!(
        document.json()["semantic_model"][0]["name"],
        "geospatial_wells"
    );
    let mut sources = SourceBindings::new();
    sources
        .bind_csv("fixtures.geospatial.wells", CSV)
        .await
        .unwrap();
    let mut imported = document.load(None, &sources).unwrap();
    assert_eq!(imported.warnings.len(), 1);
    assert_eq!(imported.warnings[0].code, "unenforced_keys");
    let mut reference = Engine::new();
    reference.register_csv("wells", CSV).await.unwrap();
    let actual = imported.engine.query(QUERY).await.unwrap();
    let expected = reference.query(QUERY).await.unwrap();
    assert_eq!(actual, expected);
    let result = pretty_format_batches(&actual).unwrap().to_string();
    assert!(result.contains("W-001") && result.contains("W-004"));
    assert_eq!(actual.iter().map(|b| b.num_rows()).sum::<usize>(), 2);
    let relation = imported.engine.catalog().relation("wells").unwrap();
    assert_eq!(
        relation.schema,
        reference.catalog().relation("wells").unwrap().schema
    );
    let metadata = relation.semantics.as_ref().unwrap();
    assert_eq!(metadata.declared_primary_key, ["well_id"]);
    assert!(
        metadata.fields["total_depth_m"]
            .description
            .as_ref()
            .unwrap()
            .contains("metres")
    );
    let origin = metadata.origin.as_ref().unwrap();
    assert_eq!(origin.schema_revision, SCHEMA_COMMIT);
    assert_eq!(origin.document_sha256.len(), 64);
    imported
        .engine
        .create_view("selected", QUERY)
        .await
        .unwrap();
    assert_eq!(
        imported
            .engine
            .query("SELECT * FROM selected")
            .await
            .unwrap(),
        actual
    );
    assert!(
        imported
            .engine
            .plan_generated_sql("SELECT * FROM fixtures.geospatial.wells")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn compiler_receives_imported_semantics_without_physical_paths_or_rows() {
    struct Model;
    impl ModelProvider for Model {
        async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
            let context: Value = serde_json::from_str(&messages[1].content).unwrap();
            let relation = &context["catalog"][0];
            assert_eq!(
                relation["semantics"]["declared_primary_key"],
                json!(["well_id"])
            );
            assert!(
                relation["semantics"]["fields"]["total_depth_m"]["description"]
                    .as_str()
                    .unwrap()
                    .contains("not a general definition of deep")
            );
            assert!(
                relation["semantics"]["model_ai_context"]["instructions"]
                    .as_str()
                    .unwrap()
                    .contains("explicit depth cutoff")
            );
            assert_eq!(
                relation["semantics"]["fields"]["latitude_deg"]["ai_context"]["synonyms"],
                json!(["latitude"])
            );
            assert!(
                messages[0]
                    .content
                    .contains("Example questions are illustrations")
            );
            for excluded in [
                "fixtures.geospatial.wells",
                "wells.csv",
                "Juniper-1",
                "Birch-4",
            ] {
                assert!(!messages[1].content.contains(excluded), "leaked {excluded}");
            }
            Ok(json!({"status":"grounded", "query":{"sql":QUERY,"evidence":[
                {"phrase":"wells","catalog_reference":"wells","interpretation":"Imported inventory"},
                {"phrase":"at least 2500 metres","catalog_reference":"wells.total_depth_m","interpretation":"User supplied cutoff, inclusive, in the documented metres"}
            ]}}).to_string())
        }
    }
    let document = OssieDocument::parse(WELLS).unwrap();
    let mut sources = SourceBindings::new();
    sources
        .bind_csv("fixtures.geospatial.wells", CSV)
        .await
        .unwrap();
    let imported = document.load(None, &sources).unwrap();
    let compilation = Compiler::new(Model)
        .compile(
            &imported.engine,
            "List active wells in North Basin at least 2500 metres deep",
        )
        .await
        .unwrap();
    let GroundingOutcome::Grounded { query } = compilation.outcome else {
        panic!("expected grounded query")
    };
    assert_eq!(
        imported
            .engine
            .query(&query.sql)
            .await
            .unwrap()
            .iter()
            .map(|b| b.num_rows())
            .sum::<usize>(),
        2
    );
}

#[tokio::test]
async fn projection_hides_extra_columns_preserves_types_and_does_not_enforce_declared_keys() {
    let mut input = simple();
    input["semantic_model"][0]["datasets"][0]["primary_key"] = json!(["id"]);
    let imported = OssieDocument::parse(&input.to_string())
        .unwrap()
        .load(None, &bindings())
        .unwrap();
    let relation = imported.engine.catalog().relation("items").unwrap();
    assert_eq!(relation.schema.fields().len(), 1);
    assert_eq!(relation.schema.field(0).data_type(), &DataType::Int32);
    assert!(relation.schema.field(0).is_nullable());
    let result = imported.engine.query("SELECT * FROM items").await.unwrap();
    assert_eq!(result[0].num_rows(), 3); // Duplicate and NULL keys remain visible.
    assert_eq!(result[0].column(0).null_count(), 1);
    assert!(
        imported
            .engine
            .query("SELECT secret FROM items")
            .await
            .is_err()
    );
    assert!(
        !catalog_context(imported.engine.catalog())
            .to_string()
            .contains("secret")
    );
}

#[test]
fn rejects_invalid_syntax_duplicates_and_schema_drift() {
    for text in [
        "version: 0.2.0.dev0\nversion: 0.2.0.dev0\nsemantic_model: []",
        r#"{"version":"0.2.0.dev0","version":"0.2.0.dev0","semantic_model":[]}"#,
        "version: 0.2.0.dev0\nsemantic_model: []\n---\nversion: 0.2.0.dev0\nsemantic_model: []",
        "version: [",
    ] {
        assert!(OssieDocument::parse(text).is_err(), "accepted {text}");
    }
    for (pointer, value) in [
        ("/version", json!("999")),
        ("/semantic_model/0/datasets/0/name", json!(42)),
    ] {
        let mut input = simple();
        *input.pointer_mut(pointer).unwrap() = value;
        assert!(OssieDocument::parse(&input.to_string()).is_err());
    }
    let mut input = simple();
    input.as_object_mut().unwrap().remove("version");
    assert!(OssieDocument::parse(&input.to_string()).is_err());
    let mut input = simple();
    input["semantic_model"][0]["datasets"][0]["typo"] = json!(true);
    assert!(OssieDocument::parse(&input.to_string()).is_err());
}

#[test]
fn rejects_unsupported_semantics_and_binding_errors_with_paths() {
    let changes = [
        (
            "/semantic_model/0/datasets/0/name",
            json!("Invalid.Name"),
            "dataset_name",
        ),
        (
            "/semantic_model/0/datasets/0/fields",
            json!([]),
            "fields_required",
        ),
        (
            "/semantic_model/0/datasets/0/source",
            json!("unbound"),
            "missing_binding",
        ),
        (
            "/semantic_model/0/datasets/0/fields/0/name",
            json!("missing"),
            "unsupported_expression",
        ),
        (
            "/semantic_model/0/datasets/0/fields/0/datatype",
            json!("String"),
            "type_mismatch",
        ),
        (
            "/semantic_model/0/datasets/0/fields/0/expression/dialects/0/expression",
            json!("id + 1"),
            "unsupported_expression",
        ),
        (
            "/semantic_model/0/datasets/0/fields/0/expression/dialects/0/dialect",
            json!("SNOWFLAKE"),
            "unsupported_dialect",
        ),
    ];
    for (pointer, value, code) in changes {
        let mut input = simple();
        *input.pointer_mut(pointer).unwrap() = value;
        error_code(
            OssieDocument::parse(&input.to_string())
                .unwrap()
                .load(None, &bindings()),
            code,
        );
    }
    for key in ["metrics", "relationships", "custom_extensions"] {
        let mut input = simple();
        input["semantic_model"][0][key] = match key {
            "metrics" => {
                json!([{"name":"total","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"COUNT(*)"}]}}])
            }
            "relationships" => {
                json!([{"name":"self","from":"items","to":"items","from_columns":["id"],"to_columns":["id"]}])
            }
            _ => json!([{"vendor_name":"VENDOR","data":"{}"}]),
        };
        error_code(
            OssieDocument::parse(&input.to_string())
                .unwrap()
                .load(None, &bindings()),
            "unsupported_feature",
        );
    }
    let mut input = simple();
    input["semantic_model"][0]["ai_context"] = json!({"unknown": "cannot drop this"});
    error_code(
        OssieDocument::parse(&input.to_string())
            .unwrap()
            .load(None, &bindings()),
        "unsupported_ai_context",
    );
    let mut input = simple();
    input["semantic_model"][0]["datasets"][0]["primary_key"] = json!(["missing"]);
    error_code(
        OssieDocument::parse(&input.to_string())
            .unwrap()
            .load(None, &bindings()),
        "invalid_key",
    );
    let mut input = simple();
    input["semantic_model"][0]["datasets"][0]["fields"][0]["name"] = json!("absent");
    input["semantic_model"][0]["datasets"][0]["fields"][0]["expression"]["dialects"][0]["expression"] =
        json!("absent");
    error_code(
        OssieDocument::parse(&input.to_string())
            .unwrap()
            .load(None, &bindings()),
        "missing_column",
    );
}

#[test]
fn selects_models_and_rejects_duplicates() {
    let mut input = simple();
    let mut second = input["semantic_model"][0].clone();
    second["name"] = json!("second");
    input["semantic_model"].as_array_mut().unwrap().push(second);
    let document = OssieDocument::parse(&input.to_string()).unwrap();
    error_code(document.load(None, &bindings()), "model_selection");
    error_code(
        document.load(Some("absent"), &bindings()),
        "model_selection",
    );
    assert!(document.load(Some("second"), &bindings()).is_ok());
    input["semantic_model"][1]["name"] = json!("model");
    error_code(
        OssieDocument::parse(&input.to_string())
            .unwrap()
            .load(Some("model"), &bindings()),
        "duplicate_name",
    );
    let mut input = simple();
    let duplicate = input["semantic_model"][0]["datasets"][0].clone();
    input["semantic_model"][0]["datasets"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    error_code(
        OssieDocument::parse(&input.to_string())
            .unwrap()
            .load(None, &bindings()),
        "dataset_name",
    );
    let mut sources = bindings();
    assert!(sources.bind("private:items", table()).is_err());
}
