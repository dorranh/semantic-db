//! Exact executable subset of Ossie vendor extensions. Unrecognized authored
//! syntax is rejected rather than silently becoming catalog authority.

use std::collections::{BTreeMap, BTreeSet};

use datafusion::sql::sqlparser::{
    ast::{Expr, FunctionArg, FunctionArgExpr, FunctionArguments},
    dialect::GenericDialect,
    parser::Parser,
    tokenizer::Token,
};
use semantic_catalog::{
    Authority, CalendarReference, CalendarSystem, ComparisonProfile, ConceptDefinition,
    ConversionRounding, DataType, EmptyBehavior, EntityId, EntityIdentity, EnumDomain, Fact,
    FactResolution, GrainKey, KeyEvidence, MetricDefinition, Presence, ReferenceSystem,
    RelationSemantics, SourceGrain, UNIT_CONVERSION_VERSION, Unit, UnitConversion, ValueMapping,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Dataset, Diagnostic, Model, OssieDocument, identifier, issue};

const VENDOR: &str = "SEMANTIC_DB";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Extension {
    vendor_name: String,
    data: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricContract {
    kind: String,
    dataset: String,
    source_grain: semantic_catalog::SourceGrain,
    dimensions: BTreeSet<String>,
    unit: Unit,
    empty: EmptyBehavior,
    #[serde(default)]
    lookup_dimensions: Vec<semantic_catalog::MetricLookupDimension>,
    #[serde(default)]
    sum_rollup_dimensions: Option<BTreeSet<String>>,
    #[serde(default)]
    state: Option<semantic_catalog::MetricStateContract>,
    #[serde(default)]
    row_filters: Vec<semantic_catalog::GovernedFilter>,
    #[serde(default)]
    result_type: Option<DataType>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConceptContract {
    kind: String,
    dataset: String,
    name: String,
    description: String,
    predicate: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BusinessCalendarContract {
    kind: String,
    dataset: String,
    name: String,
    rule: semantic_catalog::BusinessCalendarRule,
}

fn apply_business_calendar(
    document: &OssieDocument,
    definitions: &mut [(Dataset, String, RelationSemantics)],
    value: Value,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) {
    let Ok(mut contract) = serde_json::from_value::<BusinessCalendarContract>(value) else {
        issue(
            errors,
            "invalid_extension",
            path,
            "expected complete SEMANTIC_DB business-calendar contract",
        );
        return;
    };
    let rule = &contract.rule;
    let declared = |relation: &str, name: &str, datatype: &str| {
        definitions
            .iter()
            .find(|(dataset, _, _)| dataset.name == relation)
            .is_some_and(|(dataset, _, _)| {
                dataset
                    .fields
                    .iter()
                    .any(|field| field.name == name && field.datatype.as_deref() == Some(datatype))
            })
    };
    let source_datatype = match rule.source_basis {
        semantic_catalog::CalendarSourceBasis::Date32 => "Date",
        semantic_catalog::CalendarSourceBasis::UtcInstantMicros => "DateTimeTz",
    };
    if contract.kind != "business_calendar"
        || !identifier(&contract.name)
        || rule.version != semantic_catalog::BUSINESS_CALENDAR_VERSION
        || rule.source_relation != contract.dataset
        || rule.id.trim().is_empty()
        || rule.id.len() > 256
        || rule.mapping_revision.trim().is_empty()
        || rule.mapping_revision.len() > 256
        || rule.timezone.len() > 128
        || rule.timezone.parse::<chrono_tz::Tz>().is_err()
        || !declared(&contract.dataset, &rule.source_date_field, source_datatype)
        || !declared(&rule.calendar_relation, &rule.calendar_date_field, "Date")
        || !declared(&rule.calendar_relation, &rule.fiscal_year_field, "Integer")
        || !declared(
            &rule.calendar_relation,
            &rule.fiscal_period_field,
            "Integer",
        )
        || !declared(&rule.calendar_relation, &rule.business_day_field, "Boolean")
    {
        issue(
            errors,
            "invalid_business_calendar_contract",
            path,
            "calendar rule requires a bounded identity/revision, valid IANA timezone, and declared fields of matching logical types",
        );
        return;
    }
    contract.rule.source_refs = document
        .normalized
        .pointer(path)
        .map(|node| node.origins.clone())
        .unwrap_or_default();
    let (_, _, semantics) = definitions
        .iter_mut()
        .find(|(dataset, _, _)| dataset.name == contract.dataset)
        .expect("checked calendar source");
    if semantics.business_calendars.contains_key(&contract.name) {
        issue(
            errors,
            "duplicate_name",
            path,
            "duplicate business-calendar name",
        );
        return;
    }
    semantics
        .business_calendars
        .insert(contract.name, contract.rule);
}

fn metric_datatype_matches(datatype: Option<&str>, result: &DataType) -> bool {
    match result {
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => datatype == Some("Integer"),
        DataType::Decimal128(precision, scale) => {
            datatype == Some("Decimal")
                && (1..=38).contains(precision)
                && *scale >= 0
                && *scale <= *precision as i8
        }
        DataType::Float32 | DataType::Float64 => datatype == Some("Float"),
        DataType::Utf8 => datatype == Some("String"),
        DataType::Date32 => datatype == Some("Date"),
        DataType::Timestamp(_, timezone) => {
            datatype
                == Some(if timezone.is_some() {
                    "DateTimeTz"
                } else {
                    "DateTime"
                })
        }
        _ => false,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldReferenceSystemContract {
    kind: String,
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldCalendarReferenceContract {
    kind: String,
    system: CalendarSystem,
    timezone: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldComparisonProfileContract {
    kind: String,
    profile: ComparisonProfile,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldUnitContract {
    kind: String,
    unit: Unit,
}

#[derive(Default)]
pub(super) struct FieldMeanings {
    pub reference_system: Option<ReferenceSystem>,
    pub calendar_reference: Option<CalendarReference>,
    pub comparison_profile: Option<ComparisonProfile>,
    pub unit: Option<Unit>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnumMappingContract {
    kind: String,
    dataset: String,
    name: String,
    id: String,
    field: String,
    domain: String,
    codes: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityIdentityContract {
    kind: String,
    dataset: String,
    id: String,
    keys: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversionContract {
    kind: String,
    dataset: String,
    name: String,
    id: String,
    field: String,
    from_unit: Unit,
    to_unit: Unit,
    numerator: i64,
    denominator: i64,
    rounding: ConversionRounding,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewLineageContract {
    kind: String,
    name: String,
    source_dataset: String,
    columns: BTreeMap<String, String>,
    #[serde(default)]
    expected_source_revision: Option<String>,
}

pub(super) struct PreparedView {
    pub name: String,
    pub source_dataset: String,
    pub columns: BTreeMap<String, String>,
    pub expected_source_revision: Option<String>,
    pub source_refs: Vec<semantic_catalog::SourceRef>,
    pub path: String,
}

fn apply_conversion(
    document: &OssieDocument,
    definitions: &mut [(Dataset, String, RelationSemantics)],
    value: Value,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) {
    let Ok(contract) = serde_json::from_value::<ConversionContract>(value) else {
        issue(
            errors,
            "invalid_extension",
            path,
            "expected complete SEMANTIC_DB conversion contract",
        );
        return;
    };
    if contract.kind != "conversion"
        || !identifier(&contract.name)
        || contract.id.trim().is_empty()
        || contract.id.len() > 256
        || !identifier(&contract.field)
    {
        issue(
            errors,
            "invalid_conversion_contract",
            path,
            "invalid conversion identity",
        );
        return;
    }
    let Some((dataset, _, semantics)) = definitions
        .iter_mut()
        .find(|(dataset, _, _)| dataset.name == contract.dataset)
    else {
        issue(
            errors,
            "invalid_conversion_contract",
            path,
            "conversion dataset is missing",
        );
        return;
    };
    if semantics.conversions.contains_key(&contract.name)
        || !dataset.fields.iter().any(|field| {
            field.name == contract.field.as_str() && field.datatype.as_deref() == Some("Integer")
        })
        || semantics
            .fields
            .get(&contract.field)
            .and_then(|field| field.unit.as_ref())
            .is_some_and(|unit| unit != &contract.from_unit)
    {
        issue(
            errors,
            "invalid_conversion_contract",
            path,
            "conversion requires a unique name, Integer source, and exact authored source unit",
        );
        return;
    }
    let refs = document
        .normalized
        .pointer(path)
        .map(|node| node.origins.clone())
        .unwrap_or_default();
    let rule = UnitConversion {
        version: UNIT_CONVERSION_VERSION,
        id: contract.id,
        field: contract.field,
        from_unit: contract.from_unit,
        to_unit: contract.to_unit,
        numerator: contract.numerator,
        denominator: contract.denominator,
        rounding: contract.rounding,
        source_refs: refs,
    };
    if rule.validate().is_err() {
        issue(
            errors,
            "invalid_conversion_contract",
            path,
            "invalid exact conversion rule",
        );
        return;
    }
    semantics.conversions.insert(contract.name, rule);
}

fn apply_entity_identity(
    document: &OssieDocument,
    definitions: &mut [(Dataset, String, RelationSemantics)],
    value: Value,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) {
    let Ok(contract) = serde_json::from_value::<EntityIdentityContract>(value) else {
        issue(
            errors,
            "invalid_extension",
            path,
            "expected complete SEMANTIC_DB entity-identity contract",
        );
        return;
    };
    if contract.kind != "entity_identity"
        || contract.id.trim().is_empty()
        || contract.id.len() > 256
        || contract.keys.is_empty()
        || contract.keys.len() > 32
        || contract.keys.iter().any(|key| !identifier(key))
        || contract.keys.iter().collect::<BTreeSet<_>>().len() != contract.keys.len()
    {
        issue(
            errors,
            "invalid_extension",
            path,
            "invalid entity identity contract",
        );
        return;
    }
    let Some((dataset, _, semantics)) = definitions
        .iter_mut()
        .find(|(dataset, _, _)| dataset.name == contract.dataset)
    else {
        issue(
            errors,
            "invalid_extension",
            path,
            "entity dataset is missing",
        );
        return;
    };
    if semantics.entity_identity.is_some()
        || !(dataset.primary_key.as_slice() == contract.keys.as_slice()
            || dataset
                .unique_keys
                .iter()
                .any(|keys| keys.as_slice() == contract.keys.as_slice()))
        || contract.keys.iter().any(|key| {
            !dataset
                .fields
                .iter()
                .any(|field| field.name == key.as_str())
        })
    {
        issue(
            errors,
            "invalid_extension",
            path,
            "entity keys must match one declared key tuple and identity must be unique",
        );
        return;
    }
    let refs = document
        .normalized
        .pointer(path)
        .map(|node| node.origins.clone())
        .unwrap_or_default();
    let fact_id = format!("{}/key_evidence", contract.id);
    let id = EntityId(contract.id);
    semantics.entity_identity = Some(EntityIdentity {
        id: id.clone(),
        relation: contract.dataset.clone(),
        source_grain: SourceGrain {
            entity: Some(id),
            keys: contract
                .keys
                .into_iter()
                .map(|field| GrainKey {
                    relation: contract.dataset.clone(),
                    field,
                })
                .collect(),
        },
        key_evidence: FactResolution::Known {
            value: KeyEvidence::AuthoredDeclaration,
            contributors: vec![Fact {
                id: fact_id,
                scope: contract.dataset,
                value: KeyEvidence::AuthoredDeclaration,
                authority: Authority::Authored,
                origins: refs.clone(),
                evidence: vec![],
            }],
        },
        source_refs: refs,
    });
}

fn apply_enum_mapping(
    document: &OssieDocument,
    definitions: &mut [(Dataset, String, RelationSemantics)],
    value: Value,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) {
    let Ok(contract) = serde_json::from_value::<EnumMappingContract>(value) else {
        issue(
            errors,
            "invalid_extension",
            path,
            "expected complete SEMANTIC_DB enum mapping contract",
        );
        return;
    };
    if contract.kind != "enum_mapping"
        || !identifier(&contract.name)
        || contract.id.trim().is_empty()
        || contract.id.len() > 256
        || contract.domain.trim().is_empty()
        || contract.domain.len() > 256
        || contract.codes.is_empty()
        || contract.codes.len() > 128
        || contract.codes.iter().any(|(phrase, code)| {
            phrase.trim().is_empty() || phrase.len() > 256 || code.is_empty() || code.len() > 256
        })
    {
        issue(
            errors,
            "invalid_extension",
            path,
            "invalid enum mapping contract",
        );
        return;
    }
    let Some((dataset, _, semantics)) = definitions
        .iter_mut()
        .find(|(dataset, _, _)| dataset.name == contract.dataset)
    else {
        issue(
            errors,
            "invalid_extension",
            path,
            "enum mapping dataset is missing",
        );
        return;
    };
    if !dataset
        .fields
        .iter()
        .any(|field| field.name == contract.field && field.datatype.as_deref() == Some("String"))
        || semantics.value_mappings.contains_key(&contract.name)
    {
        issue(
            errors,
            "invalid_extension",
            path,
            "enum mapping requires one declared String field and unique name",
        );
        return;
    }
    let Some(field) = semantics.fields.get_mut(&contract.field) else {
        issue(
            errors,
            "invalid_extension",
            path,
            "enum mapping field is missing",
        );
        return;
    };
    if field
        .enum_domain
        .as_ref()
        .is_some_and(|domain| domain.id != contract.domain)
    {
        issue(errors, "invalid_extension", path, "conflicting enum domain");
        return;
    }
    let refs = document
        .normalized
        .pointer(path)
        .map(|node| node.origins.clone())
        .unwrap_or_default();
    let domain = EnumDomain {
        id: contract.domain,
    };
    field.enum_domain = Some(domain.clone());
    field.source_refs.extend(refs.clone());
    semantics.value_mappings.insert(
        contract.name,
        ValueMapping {
            id: contract.id,
            field: contract.field,
            description: String::new(),
            codes: contract.codes,
            enum_domain: Some(domain),
            source_refs: refs,
        },
    );
}

pub(super) fn field_meanings(
    extensions: &[Value],
    field_path: &str,
    datatype: Option<&str>,
    errors: &mut Vec<Diagnostic>,
) -> FieldMeanings {
    let mut meanings = FieldMeanings::default();
    if extensions.len() > 4 {
        issue(
            errors,
            "invalid_extension",
            format!("{field_path}/custom_extensions"),
            "at most one each of reference-system, calendar-reference, comparison-profile, and unit contracts is supported",
        );
        return meanings;
    }
    for (index, value) in extensions.iter().enumerate() {
        let path = format!("{field_path}/custom_extensions/{index}");
        let Some(data) = extension(value, &path, errors) else {
            continue;
        };
        match data.get("kind").and_then(Value::as_str) {
            Some("reference_system") => {
                let Ok(contract) = serde_json::from_value::<FieldReferenceSystemContract>(data)
                else {
                    issue(
                        errors,
                        "invalid_extension",
                        &path,
                        "expected complete SEMANTIC_DB field reference-system contract",
                    );
                    continue;
                };
                if contract.kind != "reference_system"
                    || contract.id.trim().is_empty()
                    || contract.id.len() > 256
                    || meanings.reference_system.is_some()
                {
                    issue(
                        errors,
                        "invalid_extension",
                        &path,
                        "field reference-system identity is invalid or duplicated",
                    );
                    continue;
                }
                meanings.reference_system = Some(ReferenceSystem { id: contract.id });
            }
            Some("calendar_reference") => {
                let Ok(contract) = serde_json::from_value::<FieldCalendarReferenceContract>(data)
                else {
                    issue(
                        errors,
                        "invalid_extension",
                        &path,
                        "expected complete SEMANTIC_DB field calendar-reference contract",
                    );
                    continue;
                };
                let calendar_reference = CalendarReference {
                    system: contract.system,
                    timezone: contract.timezone,
                };
                if contract.kind != "calendar_reference"
                    || calendar_reference.timezone.len() > 128
                    || matches!(&calendar_reference.system, CalendarSystem::Fiscal { id } if id.len() > 256)
                    || !semantic_catalog::valid_calendar_reference(&calendar_reference)
                    || meanings.calendar_reference.is_some()
                {
                    issue(
                        errors,
                        "invalid_extension",
                        &path,
                        "field calendar reference is invalid or duplicated",
                    );
                    continue;
                }
                meanings.calendar_reference = Some(calendar_reference);
            }
            Some("comparison_profile") => {
                let Ok(contract) = serde_json::from_value::<FieldComparisonProfileContract>(data)
                else {
                    issue(
                        errors,
                        "invalid_extension",
                        &path,
                        "expected complete SEMANTIC_DB field comparison-profile contract",
                    );
                    continue;
                };
                if contract.kind != "comparison_profile"
                    || datatype != Some("String")
                    || !matches!(contract.profile, ComparisonProfile::BinaryExact)
                    || meanings.comparison_profile.is_some()
                {
                    issue(
                        errors,
                        "invalid_extension",
                        &path,
                        "only one binary-exact String comparison profile is executable",
                    );
                    continue;
                }
                meanings.comparison_profile = Some(ComparisonProfile::BinaryExact);
            }
            Some("unit") => {
                let Ok(contract) = serde_json::from_value::<FieldUnitContract>(data) else {
                    issue(
                        errors,
                        "invalid_extension",
                        &path,
                        "expected complete SEMANTIC_DB field unit contract",
                    );
                    continue;
                };
                if contract.kind != "unit"
                    || !(matches!(datatype, Some("Integer" | "Decimal"))
                        || datatype == Some("Float")
                            && matches!(&contract.unit, Unit::Named { .. }))
                    || !semantic_catalog::valid_unit(&contract.unit)
                    || meanings.unit.is_some()
                {
                    issue(
                        errors,
                        "invalid_extension",
                        &path,
                        "field unit requires one valid numeric contract",
                    );
                    continue;
                }
                meanings.unit = Some(contract.unit);
            }
            _ => issue(
                errors,
                "invalid_extension",
                &path,
                "unsupported SEMANTIC_DB field contract kind",
            ),
        }
    }
    meanings
}

fn extension(value: &Value, path: &str, errors: &mut Vec<Diagnostic>) -> Option<Value> {
    let Ok(extension) = serde_json::from_value::<Extension>(value.clone()) else {
        issue(
            errors,
            "unsupported_feature",
            path,
            "invalid vendor extension",
        );
        return None;
    };
    if extension.vendor_name != VENDOR {
        issue(
            errors,
            "unsupported_feature",
            path,
            "unknown vendor extension in executable scope",
        );
        return None;
    }
    match serde_json::from_str::<Value>(&extension.data) {
        Ok(value) if value.is_object() => Some(value),
        _ => {
            issue(
                errors,
                "invalid_extension",
                path,
                "SEMANTIC_DB data must be a JSON object",
            );
            None
        }
    }
}

fn aggregate(expression: &str) -> Option<(&'static str, Option<String>)> {
    let dialect = GenericDialect {};
    let mut parser = Parser::new(&dialect).try_with_sql(expression).ok()?;
    let Expr::Function(function) = parser.parse_expr().ok()? else {
        return None;
    };
    if parser.peek_token().token != Token::EOF
        || function.uses_odbc_syntax
        || function.parameters != FunctionArguments::None
        || function.filter.is_some()
        || function.null_treatment.is_some()
        || function.over.is_some()
        || !function.within_group.is_empty()
    {
        return None;
    }
    let kind = if function.name.to_string().eq_ignore_ascii_case("SUM") {
        "sum"
    } else if function.name.to_string().eq_ignore_ascii_case("COUNT") {
        "count"
    } else if function.name.to_string().eq_ignore_ascii_case("MIN") {
        "min"
    } else if function.name.to_string().eq_ignore_ascii_case("MAX") {
        "max"
    } else if function.name.to_string().eq_ignore_ascii_case("AVG") {
        "avg"
    } else {
        return None;
    };
    let FunctionArguments::List(list) = function.args else {
        return None;
    };
    if list.duplicate_treatment.is_some() || !list.clauses.is_empty() || list.args.len() != 1 {
        return None;
    }
    match &list.args[0] {
        FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Identifier(field)))
            if identifier(&field.value) && field.quote_style.is_none() =>
        {
            Some((kind, Some(field.value.clone())))
        }
        FunctionArg::Unnamed(FunctionArgExpr::Wildcard) if kind == "count" => Some((kind, None)),
        _ => None,
    }
}

// This profile accepts only small, explicit Boolean trees over local fields.
fn bounded_predicate(
    value: &Value,
    fields: &BTreeSet<String>,
    depth: usize,
    nodes: &mut usize,
) -> bool {
    *nodes += 1;
    if depth > 8 || *nodes > 32 {
        return false;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    match object.get("kind").and_then(Value::as_str) {
        Some("compare" | "is_null") => object
            .get("field")
            .and_then(Value::as_str)
            .is_some_and(|field| fields.contains(field)),
        Some("compare_parameter") => {
            object
                .get("field")
                .and_then(Value::as_str)
                .is_some_and(|field| fields.contains(field))
                && object
                    .get("parameter")
                    .and_then(Value::as_str)
                    .is_some_and(|parameter| {
                        !parameter.is_empty()
                            && parameter.len() <= 64
                            && parameter
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                            && parameter
                                .as_bytes()
                                .first()
                                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                    })
        }
        Some("all" | "any") => object
            .get("predicates")
            .and_then(Value::as_array)
            .is_some_and(|children| {
                !children.is_empty()
                    && children
                        .iter()
                        .all(|child| bounded_predicate(child, fields, depth + 1, nodes))
            }),
        Some("not") => object
            .get("predicate")
            .is_some_and(|child| bounded_predicate(child, fields, depth + 1, nodes)),
        _ => false,
    }
}

pub(super) fn apply(
    document: &OssieDocument,
    model: &Model,
    path: &str,
    definitions: &mut [(Dataset, String, RelationSemantics)],
    errors: &mut Vec<Diagnostic>,
) -> Vec<PreparedView> {
    let mut views = Vec::new();
    for (index, value) in model.custom_extensions.iter().enumerate() {
        let ep = format!("{path}/custom_extensions/{index}");
        let Some(data) = extension(value, &ep, errors) else {
            continue;
        };
        if data.get("kind").and_then(Value::as_str) == Some("enum_mapping") {
            apply_enum_mapping(document, definitions, data, &ep, errors);
            continue;
        }
        if data.get("kind").and_then(Value::as_str) == Some("entity_identity") {
            apply_entity_identity(document, definitions, data, &ep, errors);
            continue;
        }
        if data.get("kind").and_then(Value::as_str) == Some("conversion") {
            apply_conversion(document, definitions, data, &ep, errors);
            continue;
        }
        if data.get("kind").and_then(Value::as_str) == Some("business_calendar") {
            apply_business_calendar(document, definitions, data, &ep, errors);
            continue;
        }
        if data.get("kind").and_then(Value::as_str) == Some("view_lineage") {
            let Ok(contract) = serde_json::from_value::<ViewLineageContract>(data) else {
                issue(
                    errors,
                    "invalid_extension",
                    &ep,
                    "invalid view-lineage contract",
                );
                continue;
            };
            if contract.kind != "view_lineage"
                || !identifier(&contract.name)
                || contract.name.len() > 128
                || !identifier(&contract.source_dataset)
                || contract.source_dataset.len() > 128
                || contract.columns.is_empty()
                || contract.columns.len() > 64
                || contract.columns.iter().any(|(output, source)| {
                    !identifier(output)
                        || output.len() > 128
                        || !identifier(source)
                        || source.len() > 128
                })
                || definitions
                    .iter()
                    .all(|(dataset, _, _)| dataset.name != contract.source_dataset)
                    && views
                        .iter()
                        .all(|view: &PreparedView| view.name != contract.source_dataset)
                || definitions
                    .iter()
                    .any(|(dataset, _, _)| dataset.name == contract.name)
                || views
                    .iter()
                    .any(|view: &PreparedView| view.name == contract.name)
            {
                issue(
                    errors,
                    "invalid_view_lineage",
                    &ep,
                    "view lineage needs a unique name and bounded columns from one imported dataset",
                );
                continue;
            }
            let fields = definitions
                .iter()
                .find(|(dataset, _, _)| dataset.name == contract.source_dataset)
                .map(|(dataset, _, _)| {
                    dataset
                        .fields
                        .iter()
                        .map(|field| field.name.as_str())
                        .collect::<BTreeSet<_>>()
                })
                .or_else(|| {
                    views
                        .iter()
                        .find(|view: &&PreparedView| view.name == contract.source_dataset)
                        .map(|view| {
                            view.columns
                                .keys()
                                .map(String::as_str)
                                .collect::<BTreeSet<_>>()
                        })
                })
                .expect("checked source relation");
            if contract
                .columns
                .values()
                .any(|source| !fields.contains(source.as_str()))
            {
                issue(
                    errors,
                    "invalid_view_lineage",
                    &ep,
                    "view lineage references an undeclared source field",
                );
                continue;
            }
            views.push(PreparedView {
                name: contract.name,
                source_dataset: contract.source_dataset,
                columns: contract.columns,
                expected_source_revision: contract.expected_source_revision,
                source_refs: document
                    .normalized
                    .pointer(&ep)
                    .map(|node| node.origins.clone())
                    .unwrap_or_default(),
                path: ep,
            });
            continue;
        }
        let Ok(contract) = serde_json::from_value::<ConceptContract>(data) else {
            issue(
                errors,
                "invalid_extension",
                &ep,
                "expected complete SEMANTIC_DB concept contract",
            );
            continue;
        };
        if contract.kind != "concept" || !identifier(&contract.name) {
            issue(
                errors,
                "invalid_extension",
                &ep,
                "expected named concept contract",
            );
            continue;
        }
        let Some((dataset, _, semantics)) = definitions
            .iter_mut()
            .find(|(dataset, _, _)| dataset.name == contract.dataset)
        else {
            issue(
                errors,
                "invalid_extension",
                &ep,
                "concept dataset is not declared",
            );
            continue;
        };
        let fields = dataset
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect();
        if !bounded_predicate(&contract.predicate, &fields, 0, &mut 0) {
            issue(
                errors,
                "invalid_concept_predicate",
                &ep,
                "predicate must be a bounded tree over declared local fields",
            );
            continue;
        }
        let Ok(predicate) = serde_json::from_value(contract.predicate) else {
            issue(
                errors,
                "invalid_concept_predicate",
                &ep,
                "predicate has invalid typed operators or literals",
            );
            continue;
        };
        let refs = document
            .normalized
            .pointer(&ep)
            .map(|node| node.origins.clone())
            .unwrap_or_default();
        let definition = ConceptDefinition {
            id: format!("ossie/{}/concepts/{}", model.name, contract.name),
            description: contract.description,
            aliases: vec![],
            alternatives: vec![],
            predicate,
            source_refs: refs,
        };
        if semantics
            .concepts
            .insert(contract.name, definition)
            .is_some()
        {
            issue(errors, "duplicate_name", &ep, "duplicate concept name");
        }
    }

    for (index, value) in model.metrics.iter().enumerate() {
        let mp = format!("{path}/metrics/{index}");
        let Some(name) = value.get("name").and_then(Value::as_str) else {
            continue;
        };
        if !identifier(name) {
            issue(
                errors,
                "metric_name",
                format!("{mp}/name"),
                "expected lowercase SQL identifier",
            );
            continue;
        }
        let Some(extensions) = value.get("custom_extensions").and_then(Value::as_array) else {
            issue(
                errors,
                "invalid_extension",
                &mp,
                "executable metric requires one SEMANTIC_DB contract",
            );
            continue;
        };
        if extensions.len() != 1 {
            issue(
                errors,
                "invalid_extension",
                &mp,
                "executable metric requires exactly one SEMANTIC_DB contract",
            );
            continue;
        }
        let ep = format!("{mp}/custom_extensions/0");
        let Some(data) = extension(&extensions[0], &ep, errors) else {
            continue;
        };
        let Ok(contract) = serde_json::from_value::<MetricContract>(data) else {
            issue(
                errors,
                "invalid_extension",
                &ep,
                "expected complete SEMANTIC_DB metric contract",
            );
            continue;
        };
        let result_type = contract.result_type.clone().unwrap_or(DataType::Int64);
        if contract.kind != "metric"
            || contract.source_grain.keys.is_empty()
            || !semantic_catalog::valid_unit(&contract.unit)
            || !metric_datatype_matches(value.get("datatype").and_then(Value::as_str), &result_type)
        {
            issue(
                errors,
                "invalid_metric_contract",
                &ep,
                "metric needs explicit grain, unit, and a matching result type",
            );
            continue;
        }
        let Some((dataset, _, semantics)) = definitions
            .iter_mut()
            .find(|(dataset, _, _)| dataset.name == contract.dataset)
        else {
            issue(
                errors,
                "invalid_metric_contract",
                &ep,
                "metric dataset is not declared",
            );
            continue;
        };
        if contract.source_grain.entity.is_some()
            && semantics.entity_identity.as_ref().is_none_or(|identity| {
                contract.source_grain.entity.as_ref() != Some(&identity.id)
                    || contract.source_grain != identity.source_grain
            })
        {
            issue(
                errors,
                "invalid_metric_contract",
                &ep,
                "entity-grained metric requires the same dataset's exact authored entity identity and key tuple",
            );
            continue;
        }
        let fields: BTreeSet<_> = dataset
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        if contract.state.as_ref().is_some_and(|state| {
            state.validate().is_err()
                || !state.merge_dimensions.is_subset(&contract.dimensions)
                || contract.sum_rollup_dimensions.is_some()
        }) || contract
            .sum_rollup_dimensions
            .as_ref()
            .is_some_and(|dimensions| !dimensions.is_subset(&contract.dimensions))
            || contract.row_filters.len() > 32
            || contract
                .row_filters
                .iter()
                .any(|filter| !fields.contains(filter.field.as_str()))
            || contract.lookup_dimensions.len() > 32
            || contract
                .lookup_dimensions
                .iter()
                .enumerate()
                .any(|(index, dimension)| {
                    contract.lookup_dimensions[..index].contains(dimension)
                        || !semantics
                            .relationships
                            .contains_key(&dimension.relationship)
                })
        {
            issue(
                errors,
                "invalid_metric_contract",
                &ep,
                "metric state, rollup, filters or lookup dimensions contradict the declared source contract",
            );
            continue;
        }
        let unique_grain_fields: BTreeSet<_> = contract
            .source_grain
            .keys
            .iter()
            .map(|key| key.field.as_str())
            .collect();
        if unique_grain_fields.len() != contract.source_grain.keys.len()
            || contract
                .source_grain
                .keys
                .iter()
                .any(|key| key.relation != contract.dataset || !fields.contains(key.field.as_str()))
            || contract
                .dimensions
                .iter()
                .any(|field| !fields.contains(field.as_str()))
        {
            issue(
                errors,
                "invalid_metric_contract",
                &ep,
                "grain and dimensions must name declared fields",
            );
            continue;
        }
        let Some(dialects) = value
            .pointer("/expression/dialects")
            .and_then(Value::as_array)
        else {
            issue(
                errors,
                "unsupported_expression",
                format!("{mp}/expression"),
                "one ANSI_SQL expression is required",
            );
            continue;
        };
        if dialects.len() != 1 || dialects[0]["dialect"] != "ANSI_SQL" {
            issue(
                errors,
                "unsupported_dialect",
                format!("{mp}/expression"),
                "exactly one ANSI_SQL expression is required",
            );
            continue;
        }
        let expression = dialects[0]["expression"].as_str().unwrap_or_default();
        let Some((kind, field)) = aggregate(expression) else {
            issue(
                errors,
                "unsupported_expression",
                format!("{mp}/expression"),
                "only SUM/AVG/MIN/MAX(field) and COUNT(field|*) are executable",
            );
            continue;
        };
        if field
            .as_ref()
            .is_some_and(|field| !fields.contains(field.as_str()))
            || matches!(kind, "sum" | "avg")
                && !field.as_ref().is_some_and(|field| {
                    dataset.fields.iter().any(|candidate| {
                        candidate.name == *field
                            && matches!(
                                candidate.datatype.as_deref(),
                                Some("Integer" | "Decimal" | "Float")
                            )
                    })
                })
            || contract.empty
                != if kind == "count"
                    || matches!(
                        contract.state.as_ref().map(|state| &state.state),
                        Some(semantic_catalog::MetricStateKind::WeightedAverage {
                            zero: semantic_catalog::ZeroWeight::Zero,
                            ..
                        })
                    )
                {
                    EmptyBehavior::Zero
                } else {
                    EmptyBehavior::Null
                }
        {
            issue(
                errors,
                "invalid_metric_contract",
                &ep,
                "aggregate field/type or empty behavior contradicts the expression",
            );
            continue;
        }
        let refs = document
            .normalized
            .pointer(&mp)
            .map(|node| node.origins.clone())
            .unwrap_or_default();
        let expression_path = format!("{mp}/expression/dialects/0/expression");
        let expression_refs = document
            .normalized
            .pointer(&expression_path)
            .map(|node| node.origins.clone())
            .unwrap_or_default();
        let definition = MetricDefinition {
            id: format!("ossie/{}/metrics/{name}", model.name),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            aliases: vec![],
            function: serde_json::from_value(json!(kind)).expect("whitelisted aggregate"),
            field,
            distinct: false,
            source_grain: contract.source_grain,
            compatible_dimensions: contract.dimensions,
            compatible_lookup_dimensions: contract.lookup_dimensions,
            sum_rollup_dimensions: contract.sum_rollup_dimensions,
            state: contract.state,
            row_filters: contract.row_filters,
            result_type,
            unit: Presence::Value(contract.unit),
            temporal: Presence::Null,
            empty_behavior: contract.empty,
            source_refs: refs,
        };
        if semantics.metrics.insert(name.into(), definition).is_some() {
            issue(errors, "duplicate_name", &mp, "duplicate metric name");
        }
        let fact_key = format!("metric_expression/{name}");
        let fact_value = json!({"dialect":"ANSI_SQL","expression":expression});
        semantics.facts.insert(
            fact_key.clone(),
            FactResolution::Known {
                value: fact_value.clone(),
                contributors: vec![Fact {
                    id: format!("ossie/{}/metrics/{name}/expression", model.name),
                    scope: format!("ossie/{}/metrics/{name}", model.name),
                    value: fact_value,
                    authority: Authority::Authored,
                    origins: expression_refs,
                    evidence: vec![],
                }],
            },
        );
    }
    views
}
