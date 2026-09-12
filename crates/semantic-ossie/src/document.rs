use std::{collections::BTreeSet, fmt, sync::OnceLock};

use datafusion::{common::Column, logical_expr::Expr, prelude::SessionContext};
use semantic_catalog::{
    AiContext, DataType, FieldSemantics, Relation, RelationSemantics, SemanticOrigin,
};
use semantic_engine::Engine;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{ImportedCatalog, SourceBindings};

pub const SPEC_VERSION: &str = "0.2.0.dev0";
pub const SCHEMA_COMMIT: &str = "28365cd638f3833765c5b940ada5b8cbc65f1c42";
const SCHEMA: &str = include_str!("../vendor/ossie/ossie-schema.json");

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    /// JSON pointer into the input document (or /bindings for application input).
    pub path: String,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.code, self.path, self.message)
    }
}

fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("Ossie import failed:\n{}", render_diagnostics(.0))]
    Diagnostics(Vec<Diagnostic>),
    #[error(transparent)]
    DataFusion(#[from] datafusion::error::DataFusionError),
    #[error(transparent)]
    Engine(#[from] semantic_engine::EngineError),
}

impl ImportError {
    pub(crate) fn diagnostic(code: &'static str, path: &str, message: impl Into<String>) -> Self {
        Self::Diagnostics(vec![Diagnostic {
            code,
            path: path.into(),
            message: message.into(),
        }])
    }
}

fn issue(
    out: &mut Vec<Diagnostic>,
    code: &'static str,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    out.push(Diagnostic {
        code,
        path: path.into(),
        message: message.into(),
    });
}

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// A schema-valid document with its original text and unmodified parsed payload.
/// Schema validity is separate from this adapter's executable capabilities.
#[derive(Debug)]
pub struct OssieDocument {
    original: String,
    value: Value,
}

impl OssieDocument {
    /// Parse YAML or JSON with duplicate-key rejection and offline schema validation.
    pub fn parse(text: &str) -> Result<Self, ImportError> {
        let options = serde_saphyr::options! {
            strict_booleans: true,
        };
        let value: Value = serde_saphyr::from_str_with_options(text, options)
            .map_err(|e| ImportError::diagnostic("parse", "/", e.to_string()))?;
        static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
        let validator = VALIDATOR.get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).expect("bundled JSON schema");
            jsonschema::validator_for(&schema).expect("bundled schema must compile offline")
        });
        let diagnostics: Vec<_> = validator
            .iter_errors(&value)
            .map(|e| Diagnostic {
                code: "schema",
                path: e.instance_path().to_string(),
                message: e.to_string(),
            })
            .collect();
        if !diagnostics.is_empty() {
            return Err(ImportError::Diagnostics(diagnostics));
        }
        Ok(Self {
            original: text.into(),
            value,
        })
    }

    pub fn original_text(&self) -> &str {
        &self.original
    }
    pub fn json(&self) -> &Value {
        &self.value
    }

    /// Inspect executable semantics and source requirements without providers or I/O.
    pub fn inspect(&self, model_name: Option<&str>) -> Result<ModelInspection, ImportError> {
        let prepared = self.prepare(model_name)?;
        Ok(ModelInspection {
            name: prepared.name,
            datasets: prepared
                .definitions
                .iter()
                .map(|(dataset, path, _)| DatasetRequirement {
                    name: dataset.name.clone(),
                    source: dataset.source.clone(),
                    path: path.clone(),
                    fields: dataset
                        .fields
                        .iter()
                        .map(|field| FieldRequirement {
                            name: field.name.clone(),
                            source_column: source_column(field).expect("validated expression"),
                            datatype: field.datatype.clone(),
                        })
                        .collect(),
                })
                .collect(),
            warnings: prepared.warnings,
        })
    }

    /// Load one model into a fresh engine. With no name, exactly one model must
    /// exist. Failure never returns a partially populated engine. Keys are
    /// descriptive declarations, reported in warnings, not enforced constraints.
    pub fn load(
        &self,
        model_name: Option<&str>,
        bindings: &SourceBindings,
    ) -> Result<ImportedCatalog, ImportError> {
        let prepared = self.prepare(model_name)?;
        let mut errors = Vec::new();
        for (dataset, path, _) in &prepared.definitions {
            if !bindings.providers.contains_key(&dataset.source) {
                issue(
                    &mut errors,
                    "missing_binding",
                    format!("{path}/source"),
                    format!("no provider bound for {:?}", dataset.source),
                );
            }
        }
        if !errors.is_empty() {
            return Err(ImportError::Diagnostics(errors));
        }
        self.load_prepared(prepared, bindings)
    }

    fn prepare(&self, model_name: Option<&str>) -> Result<PreparedModel, ImportError> {
        let models = self.value["semantic_model"]
            .as_array()
            .expect("validated array");
        let mut names = BTreeSet::new();
        for (i, model) in models.iter().enumerate() {
            let name = model["name"].as_str().expect("validated model name");
            if !names.insert(name) {
                return Err(ImportError::diagnostic(
                    "duplicate_name",
                    &format!("/semantic_model/{i}/name"),
                    format!("duplicate model {name:?}"),
                ));
            }
        }
        let index = match model_name {
            Some(name) => models
                .iter()
                .position(|m| m["name"] == name)
                .ok_or_else(|| {
                    ImportError::diagnostic(
                        "model_selection",
                        "/semantic_model",
                        format!("no model named {name:?}"),
                    )
                })?,
            None if models.len() == 1 => 0,
            None => {
                return Err(ImportError::diagnostic(
                    "model_selection",
                    "/semantic_model",
                    "specify a model name unless the document contains exactly one model",
                ));
            }
        };
        let model: Model =
            serde_json::from_value(models[index].clone()).expect("schema-validated model");
        let path = format!("/semantic_model/{index}");
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        unsupported(&model.metrics, "metrics", &path, &mut errors);
        unsupported(&model.relationships, "relationships", &path, &mut errors);
        unsupported(
            &model.custom_extensions,
            "custom_extensions",
            &path,
            &mut errors,
        );
        let model_ai = ai_context(
            &model.ai_context,
            &format!("{path}/ai_context"),
            &mut errors,
        );
        let mut dataset_names = BTreeSet::new();
        let mut definitions = Vec::new();
        for (i, dataset) in model.datasets.iter().enumerate() {
            let p = format!("{path}/datasets/{i}");
            if !identifier(&dataset.name) || !dataset_names.insert(&dataset.name) {
                issue(
                    &mut errors,
                    "dataset_name",
                    format!("{p}/name"),
                    "expected a unique lowercase SQL identifier",
                );
            }
            unsupported(
                &dataset.custom_extensions,
                "custom_extensions",
                &p,
                &mut errors,
            );
            if dataset.fields.is_empty() {
                issue(
                    &mut errors,
                    "fields_required",
                    format!("{p}/fields"),
                    "declare a nonempty explicit field list",
                );
            }
            let mut semantics = RelationSemantics {
                model_description: model.description.clone(),
                model_ai_context: model_ai.clone(),
                ai_context: ai_context(
                    &dataset.ai_context,
                    &format!("{p}/ai_context"),
                    &mut errors,
                ),
                declared_primary_key: dataset.primary_key.clone(),
                declared_unique_keys: dataset.unique_keys.clone(),
                origin: Some(SemanticOrigin {
                    format: "ossie".into(),
                    version: SPEC_VERSION.into(),
                    schema_revision: SCHEMA_COMMIT.into(),
                    schema_sha256: digest(SCHEMA),
                    document_sha256: digest(&self.original),
                    adapter_version: env!("CARGO_PKG_VERSION").into(),
                    model: model.name.clone(),
                    dataset: dataset.name.clone(),
                }),
                ..Default::default()
            };
            let mut fields = BTreeSet::new();
            for (j, field) in dataset.fields.iter().enumerate() {
                let fp = format!("{p}/fields/{j}");
                if !identifier(&field.name) || !fields.insert(&field.name) {
                    issue(
                        &mut errors,
                        "field_name",
                        format!("{fp}/name"),
                        "expected a unique lowercase SQL identifier",
                    );
                }
                if field.datatype.as_deref() == Some("Opaque") {
                    issue(
                        &mut errors,
                        "unsupported_datatype",
                        format!("{fp}/datatype"),
                        "Opaque has no executable physical-type mapping",
                    );
                }
                unsupported(
                    &field.custom_extensions,
                    "custom_extensions",
                    &fp,
                    &mut errors,
                );
                let mut dialects = BTreeSet::new();
                for dialect in &field.expression.dialects {
                    if !dialects.insert(&dialect.dialect) {
                        issue(
                            &mut errors,
                            "duplicate_dialect",
                            format!("{fp}/expression/dialects"),
                            "a dialect may occur only once",
                        );
                    }
                }
                match field
                    .expression
                    .dialects
                    .iter()
                    .find(|d| d.dialect == "ANSI_SQL")
                {
                    Some(_) if source_column(field).is_some() => {}
                    Some(_) => issue(
                        &mut errors,
                        "unsupported_expression",
                        format!("{fp}/expression"),
                        "expected a single column reference (optionally double-quoted); computed expressions are unsupported",
                    ),
                    None => issue(
                        &mut errors,
                        "unsupported_dialect",
                        format!("{fp}/expression"),
                        "an ANSI_SQL column expression is required",
                    ),
                }
                if field.expression.dialects.len() > 1 {
                    issue(
                        &mut warnings,
                        "dialect_selection",
                        format!("{fp}/expression"),
                        "using ANSI_SQL; other variants remain in the original document",
                    );
                }
                semantics.fields.insert(
                    field.name.clone(),
                    FieldSemantics {
                        description: field.description.clone(),
                        logical_type: field.datatype.clone(),
                        label: field.label.clone(),
                        is_time: field.dimension.as_ref().map(|d| {
                            d.is_time.unwrap_or(matches!(
                                field.datatype.as_deref(),
                                Some("Date" | "Time" | "DateTime" | "DateTimeTz")
                            ))
                        }),
                        ai_context: ai_context(
                            &field.ai_context,
                            &format!("{fp}/ai_context"),
                            &mut errors,
                        ),
                    },
                );
            }
            if !dataset.primary_key.is_empty() {
                check_key(
                    &dataset.primary_key,
                    &fields,
                    &format!("{p}/primary_key"),
                    &mut errors,
                );
            }
            for (k, key) in dataset.unique_keys.iter().enumerate() {
                check_key(key, &fields, &format!("{p}/unique_keys/{k}"), &mut errors);
            }
            if !dataset.primary_key.is_empty() || !dataset.unique_keys.is_empty() {
                issue(
                    &mut warnings,
                    "unenforced_keys",
                    &p,
                    "keys are retained as declarations; uniqueness and non-nullness are not enforced",
                );
            }
            definitions.push((dataset.clone(), p, semantics));
        }
        if !errors.is_empty() {
            return Err(ImportError::Diagnostics(errors));
        }
        Ok(PreparedModel {
            name: model.name,
            definitions,
            warnings,
        })
    }

    fn load_prepared(
        &self,
        prepared: PreparedModel,
        bindings: &SourceBindings,
    ) -> Result<ImportedCatalog, ImportError> {
        let PreparedModel {
            definitions,
            warnings,
            ..
        } = prepared;
        let mut errors = Vec::new();
        let mut engine = Engine::new();
        for (dataset, path, semantics) in definitions {
            let provider = bindings.providers[&dataset.source].clone();
            let schema = provider.schema();
            let mut physical_names = BTreeSet::new();
            if schema
                .fields()
                .iter()
                .any(|f| !physical_names.insert(f.name()))
            {
                return Err(ImportError::diagnostic(
                    "ambiguous_schema",
                    &path,
                    "provider schema contains duplicate column names",
                ));
            }
            for (j, field) in dataset.fields.iter().enumerate() {
                let p = format!("{path}/fields/{j}");
                let column = source_column(field).expect("validated expression");
                match schema.field_with_name(&column) {
                    Err(_) => issue(
                        &mut errors,
                        "missing_column",
                        &p,
                        format!(
                            "provider has no column {column:?} for field {:?}",
                            field.name
                        ),
                    ),
                    Ok(physical) => {
                        if let Some(logical) = &field.datatype
                            && !compatible(logical, physical.data_type())
                        {
                            issue(
                                &mut errors,
                                "type_mismatch",
                                format!("{p}/datatype"),
                                format!(
                                    "{logical} is incompatible with provider type {}",
                                    physical.data_type()
                                ),
                            );
                        }
                    }
                }
            }
            if !errors.is_empty() {
                return Err(ImportError::Diagnostics(errors));
            }
            // Projection uses bound Arrow columns, never interpolated source SQL.
            // Raw providers and undeclared fields stay outside the engine catalog.
            let projection: Vec<_> = dataset
                .fields
                .iter()
                .map(|f| {
                    let column = source_column(f).expect("validated expression");
                    let expr = Expr::Column(Column::new_unqualified(&column));
                    if column == f.name {
                        expr
                    } else {
                        expr.alias(&f.name)
                    }
                })
                .collect();
            let frame = SessionContext::new()
                .read_table(provider)?
                .select(projection)?;
            let projected = frame.into_view();
            let mut relation = Relation::base(&dataset.name, projected.schema(), &dataset.source);
            relation.description = dataset.description.clone();
            relation.semantics = Some(semantics);
            engine.register_table(relation, projected)?;
        }
        Ok(ImportedCatalog { engine, warnings })
    }
}

/// Requirements for one executable model. Physical types are checked only on load.
#[derive(Debug)]
pub struct ModelInspection {
    pub name: String,
    pub datasets: Vec<DatasetRequirement>,
    pub warnings: Vec<Diagnostic>,
}

#[derive(Debug)]
pub struct DatasetRequirement {
    pub name: String,
    pub source: String,
    pub path: String,
    pub fields: Vec<FieldRequirement>,
}

#[derive(Debug)]
pub struct FieldRequirement {
    pub name: String,
    pub source_column: String,
    pub datatype: Option<String>,
}

struct PreparedModel {
    name: String,
    definitions: Vec<(Dataset, String, RelationSemantics)>,
    warnings: Vec<Diagnostic>,
}

fn source_column(field: &Field) -> Option<String> {
    let text = field
        .expression
        .dialects
        .iter()
        .find(|d| d.dialect == "ANSI_SQL")?
        .expression
        .trim();
    if let Some(inner) = text.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        if inner.is_empty() {
            return None;
        }
        let mut chars = inner.chars();
        let mut name = String::new();
        while let Some(ch) = chars.next() {
            if ch == '"' && chars.next() != Some('"') {
                return None;
            }
            name.push(ch);
        }
        Some(name)
    } else {
        let mut chars = text.chars();
        let first = chars.next()?;
        ((first.is_ascii_alphabetic() || first == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .then(|| text.to_owned())
    }
}

fn unsupported(values: &[Value], name: &str, parent: &str, errors: &mut Vec<Diagnostic>) {
    for (i, _) in values.iter().enumerate() {
        issue(
            errors,
            "unsupported_feature",
            format!("{parent}/{name}/{i}"),
            format!("{name} are not supported by executable import; original document preserved"),
        );
    }
}

fn ai_context(
    value: &Option<Value>,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) -> Option<AiContext> {
    match value {
        None => None,
        Some(Value::String(text)) => Some(AiContext {
            instructions: Some(text.clone()),
            ..Default::default()
        }),
        Some(value) => match serde_json::from_value(value.clone()) {
            Ok(context) => Some(context),
            Err(error) => {
                issue(errors, "unsupported_ai_context", path, error.to_string());
                None
            }
        },
    }
}

fn check_key(key: &[String], fields: &BTreeSet<&String>, path: &str, errors: &mut Vec<Diagnostic>) {
    let mut seen = BTreeSet::new();
    if key.is_empty()
        || key
            .iter()
            .any(|column| !fields.contains(column) || !seen.insert(column))
    {
        issue(
            errors,
            "invalid_key",
            path,
            "key must contain distinct declared field names",
        );
    }
}

fn compatible(logical: &str, physical: &DataType) -> bool {
    use DataType::*;
    match logical {
        "String" => matches!(physical, Utf8 | LargeUtf8 | Utf8View),
        "Integer" => matches!(
            physical,
            Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64
        ),
        "Float" => matches!(physical, Float16 | Float32 | Float64),
        "Decimal" => matches!(
            physical,
            Decimal32(_, _) | Decimal64(_, _) | Decimal128(_, _) | Decimal256(_, _)
        ),
        "Boolean" => matches!(physical, Boolean),
        "Date" => matches!(physical, Date32 | Date64),
        "Time" => matches!(physical, Time32(_) | Time64(_)),
        "DateTime" => matches!(physical, Timestamp(_, None)),
        "DateTimeTz" => matches!(physical, Timestamp(_, Some(_))),
        _ => false,
    }
}

#[derive(Clone, Deserialize)]
struct Model {
    name: String,
    description: Option<String>,
    ai_context: Option<Value>,
    datasets: Vec<Dataset>,
    #[serde(default)]
    metrics: Vec<Value>,
    #[serde(default)]
    relationships: Vec<Value>,
    #[serde(default)]
    custom_extensions: Vec<Value>,
}
#[derive(Clone, Deserialize)]
struct Dataset {
    name: String,
    source: String,
    description: Option<String>,
    ai_context: Option<Value>,
    #[serde(default)]
    fields: Vec<Field>,
    #[serde(default)]
    primary_key: Vec<String>,
    #[serde(default)]
    unique_keys: Vec<Vec<String>>,
    #[serde(default)]
    custom_extensions: Vec<Value>,
}
#[derive(Clone, Deserialize)]
struct Field {
    name: String,
    expression: Expression,
    description: Option<String>,
    datatype: Option<String>,
    label: Option<String>,
    dimension: Option<Dimension>,
    ai_context: Option<Value>,
    #[serde(default)]
    custom_extensions: Vec<Value>,
}
#[derive(Clone, Deserialize)]
struct Expression {
    dialects: Vec<DialectExpression>,
}
#[derive(Clone, Deserialize)]
struct DialectExpression {
    dialect: String,
    expression: String,
}
#[derive(Clone, Deserialize)]
struct Dimension {
    is_time: Option<bool>,
}
