use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{
            ArrayRef, Decimal128Array, Int8Array, Int32Array, Int64Array, StringArray,
            TimestampMicrosecondArray,
        },
        datatypes::TimeUnit,
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    Authority, ComparisonProfile, DataType, FactResolution, Field, KeyEvidence, PublicationLimits,
    Schema, Unit,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{QueryOptions, TableProvider};
use semantic_ossie::{ImportError, OssieDocument, SourceBindings};
use semantic_plan::typed::{
    AggregateFunction, CalendarUnit, Comparison, Direction, FieldRef, Literal, NullOrder,
    ROW_QUERY_VERSION, RelationInput, Requirement, RowOperation, RowPredicate, RowQuery,
};
use serde_json::{Value, json};

fn extension(data: Value) -> Value {
    json!({"vendor_name":"SEMANTIC_DB","data":data.to_string()})
}

fn model() -> Value {
    json!({"version":"0.2.0.dev0","semantic_model":[{
        "name":"shop",
        "datasets":[{"name":"items","source":"fixture:items","fields":[
            {"name":"id","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"id"}]}},
            {"name":"amount","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"amount"}]}},
            {"name":"state","datatype":"String","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"state"}]}}
        ]}],
        "metrics":[{"name":"total_amount","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":" SUM(amount) "}]},
            "custom_extensions":[extension(json!({"kind":"metric","dataset":"items","source_grain":{"entity":null,"keys":[{"relation":"items","field":"id"}]},"dimensions":["state"],"unit":{"kind":"currency","code":"USD"},"empty":"null"}))]}],
        "custom_extensions":[extension(json!({"kind":"concept","dataset":"items","name":"active","description":"Open items",
            "predicate":{"kind":"compare","field":"state","operator":"eq","value":{"type":"utf8","value":"open"}}}))]
    }]})
}

fn bindings() -> SourceBindings {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Int64, false),
        Field::new("state", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            Arc::new(Int64Array::from(vec![4, 9, 6])) as ArrayRef,
            Arc::new(StringArray::from(vec!["open", "closed", "open"])) as ArrayRef,
        ],
    )
    .unwrap();
    let table =
        Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()) as Arc<dyn TableProvider>;
    let mut sources = SourceBindings::new();
    sources.bind("fixture:items", table).unwrap();
    sources
}

fn unit_model() -> Value {
    let mut value = model();
    value["semantic_model"][0]["datasets"][0]["fields"][1]["custom_extensions"] =
        json!([extension(json!({
            "kind":"unit","unit":{"kind":"currency","code":"USD"}
        }))]);
    value
}

fn conversion_extension() -> Value {
    extension(json!({
        "kind":"conversion","dataset":"amounts","name":"thirds",
        "id":"conversions/whole-to-third","field":"amount",
        "from_unit":{"kind":"named","id":"whole"},
        "to_unit":{"kind":"named","id":"third"},
        "numerator":1,"denominator":3,"rounding":"half_even"
    }))
}

fn conversion_model() -> Value {
    json!({"version":"0.2.0.dev0","semantic_model":[{
        "name":"shop","datasets":[{"name":"amounts","source":"fixture:amounts","fields":[
            {"name":"amount","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"amount"}]},
             "custom_extensions":[extension(json!({"kind":"unit","unit":{"kind":"named","id":"whole"}}))]}
        ]}],
        "custom_extensions":[conversion_extension()]
    }]})
}

fn conversion_bindings(narrow: bool) -> SourceBindings {
    let ty = if narrow {
        DataType::Int32
    } else {
        DataType::Int64
    };
    let schema = Arc::new(Schema::new(vec![Field::new("amount", ty, false)]));
    let values: ArrayRef = if narrow {
        Arc::new(Int32Array::from(vec![4, 9, 6]))
    } else {
        Arc::new(Int64Array::from(vec![4, 9, 6]))
    };
    let batch = RecordBatch::try_new(schema.clone(), vec![values]).unwrap();
    let mut bindings = SourceBindings::new();
    bindings
        .bind(
            "fixture:amounts",
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    bindings
}

fn view_lineage_model() -> Value {
    let mut value = model();
    value["semantic_model"][0]["custom_extensions"]
        .as_array_mut()
        .unwrap()
        .extend([
            extension(json!({
                "kind":"view_lineage","name":"item_ids",
                "source_dataset":"items","columns":{"id":"id"}
            })),
            extension(json!({
                "kind":"view_lineage","name":"item_ids_twice",
                "source_dataset":"item_ids","columns":{"id":"id"}
            })),
        ]);
    value
}

#[tokio::test]
async fn imported_nested_canonical_view_lineage_pins_source_and_executes() {
    let document = OssieDocument::parse(&view_lineage_model().to_string()).unwrap();
    let imported = document.load(None, &bindings()).unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let catalog = imported.engine.catalog().snapshot();
    let first = catalog.relation("item_ids").unwrap();
    let second = catalog.relation("item_ids_twice").unwrap();
    let first_lineage = first
        .definition()
        .semantics
        .as_ref()
        .unwrap()
        .view_lineage
        .as_ref()
        .unwrap();
    let second_lineage = second
        .definition()
        .semantics
        .as_ref()
        .unwrap()
        .view_lineage
        .as_ref()
        .unwrap();
    assert_eq!(
        &first_lineage.source,
        catalog.relation("items").unwrap().reference()
    );
    assert_eq!(&second_lineage.source, first.reference());
    assert!(
        first_lineage
            .source_refs
            .iter()
            .any(|reference| { reference.path.ends_with("/custom_extensions/1") })
    );
    assert!(
        second_lineage
            .source_refs
            .iter()
            .any(|reference| { reference.path.ends_with("/custom_extensions/2") })
    );
    let query = serde_json::from_value(json!({
        "version":1,"input":{"relation":"item_ids_twice","instance":"v"},
        "requirements":[
            {"id":"id","source_text":"item id","operation":{"kind":"project","field":{"instance":"v","field":"id"},"alias":"id"}},
            {"id":"order","source_text":"ascending id","operation":{"kind":"order","field":{"instance":"v","field":"id"},"direction":"asc","nulls":"last"}}
        ],"unresolved":[]
    })).unwrap();
    let result = compile_rows(&imported.engine, query, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("imported view rejected: {:?}", result.outcome)
    };
    let bound = serde_json::to_value(query.bound()).unwrap();
    assert!(
        bound["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|definition| { definition["id"] == "view_lineage/item_ids_twice" })
    );
    for batches in [
        query
            .plan_direct(&imported.engine)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
        query
            .execute(&imported.engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
    ] {
        let values = batches
            .iter()
            .flat_map(|batch| {
                (0..batch.num_rows())
                    .map(|row| array_value_to_string(batch.column(0), row).unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(values, ["1", "2", "3"]);
    }
}

#[test]
fn view_lineage_extension_rejects_malformed_and_stale_sources() {
    for contract in [
        json!({"kind":"view_lineage","name":"item_ids","source_dataset":"items","columns":{"id":"missing"}}),
        json!({"kind":"view_lineage","name":"item_ids","source_dataset":"unknown","columns":{"id":"id"}}),
        json!({"kind":"view_lineage","name":"items","source_dataset":"items","columns":{"id":"id"}}),
        json!({"kind":"view_lineage","name":"item_ids","source_dataset":"items","columns":{"id":"id"},"sql":"SELECT id FROM items"}),
    ] {
        let mut value = view_lineage_model();
        value["semantic_model"][0]["custom_extensions"][1] = extension(contract);
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("malformed view lineage was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                matches!(
                    diagnostic.code,
                    "invalid_view_lineage" | "invalid_extension"
                ) && diagnostic.path.ends_with("/custom_extensions/1")
            }),
            "{diagnostics:?}"
        );
    }
    let mut stale = view_lineage_model();
    stale["semantic_model"][0]["custom_extensions"][1] = extension(json!({
        "kind":"view_lineage","name":"item_ids","source_dataset":"items",
        "columns":{"id":"id"},"expected_source_revision":"stale"
    }));
    let document = OssieDocument::parse(&stale.to_string()).unwrap();
    let Err(ImportError::Diagnostics(diagnostics)) = document.load(None, &bindings()) else {
        panic!("stale authored view revision was imported")
    };
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_view_lineage"
            && diagnostic.path.ends_with("/custom_extensions/1")
    }));
}

#[tokio::test]
async fn imported_typed_conversion_executes_exact_sql_and_direct_rows() {
    let document = OssieDocument::parse(&conversion_model().to_string()).unwrap();
    let imported = document.load(None, &conversion_bindings(false)).unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let relation = imported.engine.catalog().relation("amounts").unwrap();
    let semantics = relation.semantics.as_ref().unwrap();
    let rule = &semantics.conversions["thirds"];
    assert_eq!(rule.from_unit, Unit::Named { id: "whole".into() });
    assert_eq!(rule.to_unit, Unit::Named { id: "third".into() });
    assert!(
        rule.source_refs
            .iter()
            .any(|reference| { reference.path.ends_with("/custom_extensions/0") })
    );
    let query = serde_json::from_value(json!({
        "version":1,"input":{"relation":"amounts","instance":"a"},"requirements":[
            {"id":"thirds","source_text":"convert whole to thirds","operation":{"kind":"convert","conversion":"thirds","alias":"thirds"}}
        ],"unresolved":[]
    }))
    .unwrap();
    let compilation = compile_rows(&imported.engine, query, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = compilation.outcome else {
        panic!("typed conversion rejected: {:?}", compilation.outcome)
    };
    let values = |batches: &[RecordBatch]| {
        let mut values = batches
            .iter()
            .flat_map(|batch| {
                let column = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Decimal128Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| column.value(row))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        values.sort();
        values
    };
    let direct = query
        .plan_direct(&imported.engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&imported.engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = vec![
        1_333_333_333_333_333_333,
        2_000_000_000_000_000_000,
        3_000_000_000_000_000_000,
    ];
    assert_eq!(values(&direct), expected);
    assert_eq!(values(&sql), expected);
}

#[test]
fn conversion_extension_rejects_malformed_mismatched_and_duplicate_rules() {
    for contract in [
        json!({"kind":"conversion","dataset":"amounts","name":"thirds","id":"conversions/x","field":"amount","from_unit":{"kind":"named","id":"whole"},"to_unit":{"kind":"named","id":"third"},"numerator":1,"rounding":"half_even"}),
        json!({"kind":"conversion","dataset":"amounts","name":"thirds","id":"conversions/x","field":"amount","from_unit":{"kind":"named","id":"other"},"to_unit":{"kind":"named","id":"third"},"numerator":1,"denominator":3,"rounding":"half_even"}),
        json!({"kind":"conversion","dataset":"amounts","name":"thirds","id":"conversions/x","field":"missing","from_unit":{"kind":"named","id":"whole"},"to_unit":{"kind":"named","id":"third"},"numerator":1,"denominator":3,"rounding":"half_even"}),
        json!({"kind":"conversion","dataset":"amounts","name":"thirds","id":"conversions/x","field":"amount","from_unit":{"kind":"named","id":"whole"},"to_unit":{"kind":"named","id":"third"},"numerator":1,"denominator":0,"rounding":"half_even"}),
        json!({"kind":"conversion","dataset":"amounts","name":"thirds","id":"conversions/x","field":"amount","from_unit":{"kind":"named","id":"whole"},"to_unit":{"kind":"named","id":"third"},"numerator":1,"denominator":3,"rounding":"half_even","sql":"amount * 3"}),
    ] {
        let mut value = conversion_model();
        value["semantic_model"][0]["custom_extensions"][0] = extension(contract);
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("invalid conversion was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                matches!(
                    diagnostic.code,
                    "invalid_extension" | "invalid_conversion_contract"
                ) && diagnostic.path.ends_with("/custom_extensions/0")
            }),
            "{diagnostics:?}"
        );
    }

    let mut duplicate = conversion_model();
    duplicate["semantic_model"][0]["custom_extensions"]
        .as_array_mut()
        .unwrap()
        .push(conversion_extension());
    let document = OssieDocument::parse(&duplicate.to_string()).unwrap();
    let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
        panic!("duplicate conversion was imported")
    };
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_conversion_contract"
            && diagnostic.path.ends_with("/custom_extensions/1")
    }));

    let mut unknown = conversion_model();
    unknown["semantic_model"][0]["custom_extensions"][0] =
        json!({"vendor_name":"OTHER","data":"{}"});
    error_code(unknown, "unsupported_feature");

    let document = OssieDocument::parse(&conversion_model().to_string()).unwrap();
    let Err(ImportError::Diagnostics(diagnostics)) =
        document.load(None, &conversion_bindings(true))
    else {
        panic!("Int32 exact conversion source was imported")
    };
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_conversion_contract"
            && diagnostic.path.ends_with("/custom_extensions/0")
    }));
}

fn narrow_amount_bindings() -> SourceBindings {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Int8, false),
        Field::new("state", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            Arc::new(Int8Array::from(vec![4, 9, 6])) as ArrayRef,
            Arc::new(StringArray::from(vec!["open", "closed", "open"])) as ArrayRef,
        ],
    )
    .unwrap();
    let mut sources = SourceBindings::new();
    sources
        .bind(
            "fixture:items",
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    sources
}

#[tokio::test]
async fn imported_numeric_field_unit_publishes_with_source_reference() {
    let document = OssieDocument::parse(&unit_model().to_string()).unwrap();
    let imported = document.load(None, &bindings()).unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let relation = imported.engine.catalog().relation("items").unwrap();
    let amount = &relation.semantics.as_ref().unwrap().fields["amount"];
    assert_eq!(amount.unit, Some(Unit::Currency { code: "USD".into() }));
    assert!(
        amount
            .source_refs
            .iter()
            .any(|reference| { reference.path.ends_with("/fields/1/custom_extensions/0") })
    );
    let query = serde_json::from_value(json!({
        "version":1,"input":{"relation":"items","instance":"i"},"requirements":[
            {"id":"sum","source_text":"total amount","operation":{"kind":"metric","name":"total_amount","alias":"total"}}
        ],"unresolved":[]
    }))
    .unwrap();
    let result = compile_rows(&imported.engine, query, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("USD metric rejected: {:?}", result.outcome)
    };
    let direct = query
        .plan_direct(&imported.engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&imported.engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(array_value_to_string(direct[0].column(0), 0).unwrap(), "19");
    assert_eq!(array_value_to_string(sql[0].column(0), 0).unwrap(), "19");
}

#[test]
fn field_unit_extensions_reject_invalid_unknown_duplicate_and_unsupported_type() {
    for contract in [
        json!({"kind":"unit","unit":{"kind":"currency","code":"usd"}}),
        json!({"kind":"unit"}),
        json!({"kind":"unit","unit":{"kind":"opaque","id":"USD"}}),
        json!({"kind":"unit","unit":{"kind":"currency","code":"USD"},"extra":true}),
        json!({"kind":"unknown","unit":{"kind":"currency","code":"USD"}}),
    ] {
        let mut value = unit_model();
        value["semantic_model"][0]["datasets"][0]["fields"][1]["custom_extensions"] =
            json!([extension(contract)]);
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("invalid unit contract was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_extension"
                    && diagnostic.path.ends_with("/fields/1/custom_extensions/0")
            }),
            "{diagnostics:?}"
        );
    }

    let mut nonnumeric = unit_model();
    let unit_extension =
        nonnumeric["semantic_model"][0]["datasets"][0]["fields"][1]["custom_extensions"].clone();
    nonnumeric["semantic_model"][0]["datasets"][0]["fields"][2]["custom_extensions"] =
        unit_extension;
    error_code(nonnumeric, "invalid_extension");

    let mut duplicate = unit_model();
    duplicate["semantic_model"][0]["datasets"][0]["fields"][1]["custom_extensions"]
        .as_array_mut()
        .unwrap()
        .push(extension(json!({
            "kind":"unit","unit":{"kind":"currency","code":"EUR"}
        })));
    let document = OssieDocument::parse(&duplicate.to_string()).unwrap();
    let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
        panic!("conflicting unit extensions were imported")
    };
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_extension"
            && diagnostic.path.ends_with("/fields/1/custom_extensions/1")
    }));

    let document = OssieDocument::parse(&unit_model().to_string()).unwrap();
    let Err(ImportError::Diagnostics(diagnostics)) = document.load(None, &narrow_amount_bindings())
    else {
        panic!("unsupported physical unit field was imported")
    };
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_field_unit" && diagnostic.path.ends_with("/fields/1")
    }));
}

fn relationship_model() -> Value {
    let mut value = model();
    value["semantic_model"][0]["datasets"][0]["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "name":"customer_id","datatype":"Integer",
            "expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"customer_id"}]}
        }));
    value["semantic_model"][0]["datasets"].as_array_mut().unwrap().push(json!({
        "name":"customers","source":"fixture:customers","fields":[
            {"name":"id","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"id"}]}},
            {"name":"label","datatype":"String","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"label"}]}}
        ]
    }));
    value["semantic_model"][0]["relationships"] = json!([{
        "name":"buyer","from":"items","to":"customers",
        "from_columns":["customer_id"],"to_columns":["id"]
    }]);
    value
}

fn relationship_model_with_systems(left: Option<&str>, right: Option<&str>) -> Value {
    let mut value = relationship_model();
    if let Some(id) = left {
        value["semantic_model"][0]["datasets"][0]["fields"][3]["custom_extensions"] =
            json!([extension(json!({"kind":"reference_system","id":id}))]);
    }
    if let Some(id) = right {
        value["semantic_model"][0]["datasets"][1]["fields"][0]["custom_extensions"] =
            json!([extension(json!({"kind":"reference_system","id":id}))]);
    }
    value
}

fn entity_model() -> Value {
    let mut value = relationship_model();
    value["semantic_model"][0]["datasets"][1]["primary_key"] = json!(["id"]);
    value["semantic_model"][0]["custom_extensions"]
        .as_array_mut()
        .unwrap()
        .push(extension(json!({
            "kind":"entity_identity","dataset":"customers",
            "id":"entities/customer","keys":["id"]
        })));
    value
}

fn entity_metric_model() -> Value {
    let mut value = entity_model();
    value["semantic_model"][0]["metrics"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "name":"customer_count","datatype":"Integer",
            "expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"COUNT(*)"}]},
            "custom_extensions":[extension(json!({
                "kind":"metric","dataset":"customers",
                "source_grain":{"entity":"entities/customer","keys":[{"relation":"customers","field":"id"}]},
                "dimensions":[],"unit":{"kind":"named","id":"customers"},"empty":"zero"
            }))]
        }));
    value
}

#[tokio::test]
async fn imported_entity_grained_metric_executes_without_claiming_key_enforcement() {
    let document = OssieDocument::parse(&entity_metric_model().to_string()).unwrap();
    let imported = document
        .load(None, &relationship_bindings(false, false, false))
        .unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let customers = imported.engine.catalog().relation("customers").unwrap();
    let semantics = customers.semantics.as_ref().unwrap();
    assert_eq!(
        semantics.metrics["customer_count"].source_grain,
        semantics.entity_identity.as_ref().unwrap().source_grain
    );
    let items = imported.engine.catalog().relation("items").unwrap();
    assert!(matches!(
        &items.semantics.as_ref().unwrap().relationships["buyer"].cardinality,
        FactResolution::Unknown
    ));
    let query = serde_json::from_value(json!({
        "version":1,"input":{"relation":"customers","instance":"c"},"requirements":[
            {"id":"count","source_text":"count customers","operation":{"kind":"metric","name":"customer_count","alias":"count"}}
        ],"unresolved":[]
    }))
    .unwrap();
    let compilation = compile_rows(&imported.engine, query, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = compilation.outcome else {
        panic!("entity metric rejected: {:?}", compilation.outcome)
    };
    let direct = query
        .plan_direct(&imported.engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&imported.engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(array_value_to_string(direct[0].column(0), 0).unwrap(), "2");
    assert_eq!(array_value_to_string(sql[0].column(0), 0).unwrap(), "2");
}

#[test]
fn entity_grained_metric_import_requires_exact_authored_identity_and_keys() {
    let mut missing_identity = entity_metric_model();
    missing_identity["semantic_model"][0]["custom_extensions"]
        .as_array_mut()
        .unwrap()
        .pop();
    error_code(missing_identity, "invalid_metric_contract");

    for grain in [
        json!({"entity":"entities/other","keys":[{"relation":"customers","field":"id"}]}),
        json!({"entity":"entities/customer","keys":[{"relation":"customers","field":"label"}]}),
        json!({"entity":"entities/customer","keys":[{"relation":"items","field":"id"}]}),
    ] {
        let mut value = entity_metric_model();
        value["semantic_model"][0]["metrics"][1]["custom_extensions"][0] = extension(json!({
            "kind":"metric","dataset":"customers","source_grain":grain,
            "dimensions":[],"unit":{"kind":"named","id":"customers"},"empty":"zero"
        }));
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("ungrounded entity-grained metric was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_metric_contract"
                    && diagnostic.path.ends_with("/metrics/1/custom_extensions/0")
            }),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn imported_entity_identity_is_authored_and_does_not_upgrade_cardinality() {
    let document = OssieDocument::parse(&entity_model().to_string()).unwrap();
    let imported = document
        .load(None, &relationship_bindings(false, false, false))
        .unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let customers = imported.engine.catalog().relation("customers").unwrap();
    let identity = customers
        .semantics
        .as_ref()
        .unwrap()
        .entity_identity
        .as_ref()
        .unwrap();
    assert_eq!(identity.id.0, "entities/customer");
    assert_eq!(identity.relation, "customers");
    assert_eq!(identity.source_grain.entity.as_ref(), Some(&identity.id));
    assert_eq!(identity.source_grain.keys[0].relation, "customers");
    assert_eq!(identity.source_grain.keys[0].field, "id");
    assert!(identity.source_refs.iter().any(|reference| {
        reference
            .path
            .ends_with("/semantic_model/0/custom_extensions/1")
    }));
    let FactResolution::Known {
        value,
        contributors,
    } = &identity.key_evidence
    else {
        panic!("authored entity key evidence missing")
    };
    assert_eq!(value, &KeyEvidence::AuthoredDeclaration);
    assert_eq!(contributors[0].authority, Authority::Authored);
    assert_eq!(contributors[0].origins, identity.source_refs);
    let items = imported.engine.catalog().relation("items").unwrap();
    assert!(matches!(
        &items.semantics.as_ref().unwrap().relationships["buyer"].cardinality,
        FactResolution::Unknown
    ));
    // A provider with duplicate key values can still load this authored
    // declaration. Runtime lookup retains its separate duplicate-match guard.
    let duplicate_rows = document
        .load(None, &relationship_bindings(true, false, false))
        .unwrap();
    assert!(matches!(
        &duplicate_rows
            .engine
            .catalog()
            .relation("items")
            .unwrap()
            .semantics
            .as_ref()
            .unwrap()
            .relationships["buyer"]
            .cardinality,
        FactResolution::Unknown
    ));
}

#[test]
fn entity_import_rejects_missing_wrong_nullable_and_opaque_keys() {
    for contract in [
        json!({"kind":"entity_identity","dataset":"missing","id":"entities/customer","keys":["id"]}),
        json!({"kind":"entity_identity","dataset":"customers","id":"entities/customer","keys":["other_id"]}),
        json!({"kind":"entity_identity","dataset":"customers","id":"entities/customer","keys":[]}),
        json!({"kind":"entity_identity","dataset":"customers","id":"entities/customer","keys":["id"],"evidence":"source_constraint"}),
        json!({"kind":"unknown","dataset":"customers","id":"entities/customer","keys":["id"]}),
    ] {
        let mut value = entity_model();
        value["semantic_model"][0]["custom_extensions"][1] = extension(contract);
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("invalid entity contract was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_extension"
                    && diagnostic.path.ends_with("/custom_extensions/1")
            }),
            "{diagnostics:?}"
        );
    }

    let document = OssieDocument::parse(&entity_model().to_string()).unwrap();
    let Err(ImportError::Diagnostics(diagnostics)) =
        document.load(None, &relationship_bindings(false, false, true))
    else {
        panic!("nullable physical entity key was imported")
    };
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "entity_key_nullable"
                && diagnostic.path.ends_with("/custom_extensions/1")
        }),
        "{diagnostics:?}"
    );

    let mut value = entity_model();
    value["semantic_model"][0]["custom_extensions"][1] = json!({"vendor_name":"OTHER","data":"{}"});
    error_code(value, "unsupported_feature");
}

#[test]
fn duplicate_or_conflicting_entity_extension_fails_at_second_source_path() {
    for second in [
        json!({"kind":"entity_identity","dataset":"customers","id":"entities/customer","keys":["id"]}),
        json!({"kind":"entity_identity","dataset":"customers","id":"entities/other","keys":["id"]}),
    ] {
        let mut value = entity_model();
        value["semantic_model"][0]["custom_extensions"]
            .as_array_mut()
            .unwrap()
            .push(extension(second));
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("duplicate entity identity was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_extension"
                    && diagnostic.path.ends_with("/custom_extensions/2")
            }),
            "{diagnostics:?}"
        );
    }
}

fn relationship_bindings(
    duplicate_right: bool,
    wrong_right_type: bool,
    nullable_right_key: bool,
) -> SourceBindings {
    let items_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Int64, false),
        Field::new("state", DataType::Utf8, false),
        Field::new("customer_id", DataType::Int64, false),
    ]));
    let items = RecordBatch::try_new(
        items_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            Arc::new(Int64Array::from(vec![4, 9, 6])) as ArrayRef,
            Arc::new(StringArray::from(vec!["open", "closed", "open"])) as ArrayRef,
            Arc::new(Int64Array::from(vec![10, 20, 10])) as ArrayRef,
        ],
    )
    .unwrap();
    let customer_schema = Arc::new(Schema::new(vec![
        Field::new(
            "id",
            if wrong_right_type {
                DataType::Utf8
            } else {
                DataType::Int64
            },
            nullable_right_key,
        ),
        Field::new("label", DataType::Utf8, false),
    ]));
    let mut labels = vec!["CH", "GB"];
    if duplicate_right {
        labels.push("Other");
    }
    let ids: ArrayRef = if wrong_right_type {
        let mut values = vec!["10", "20"];
        if duplicate_right {
            values.push("10");
        }
        Arc::new(StringArray::from(values))
    } else {
        let mut values = vec![10, 20];
        if duplicate_right {
            values.push(10);
        }
        Arc::new(Int64Array::from(values))
    };
    let customers = RecordBatch::try_new(
        customer_schema.clone(),
        vec![ids, Arc::new(StringArray::from(labels)) as ArrayRef],
    )
    .unwrap();
    let mut bindings = SourceBindings::new();
    bindings
        .bind(
            "fixture:items",
            Arc::new(MemTable::try_new(items_schema, vec![vec![items]]).unwrap()),
        )
        .unwrap();
    bindings
        .bind(
            "fixture:customers",
            Arc::new(MemTable::try_new(customer_schema, vec![vec![customers]]).unwrap()),
        )
        .unwrap();
    bindings
}

fn error_code(value: Value, code: &str) {
    let document = OssieDocument::parse(&value.to_string()).unwrap();
    let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
        panic!("expected import diagnostic {code}");
    };
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic.code == code),
        "{diagnostics:?}"
    );
}

fn enum_mapping_model() -> Value {
    let mut value = model();
    value["semantic_model"][0]["custom_extensions"] = json!([extension(json!({
        "kind":"enum_mapping","dataset":"items","name":"status_codes",
        "id":"values/status-codes","field":"state","domain":"order-status",
        "codes":{"active":"open","inactive":"closed"}
    }))]);
    value
}

fn comparison_model() -> Value {
    let mut value = model();
    value["semantic_model"][0]["datasets"][0]["fields"][2]["custom_extensions"] =
        json!([extension(json!({
            "kind":"comparison_profile","profile":{"kind":"binary_exact"}
        }))]);
    value
}

#[tokio::test]
async fn imported_binary_exact_comparison_executes_fixed_utf8_rows() {
    let document = OssieDocument::parse(&comparison_model().to_string()).unwrap();
    let imported = document.load(None, &bindings()).unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let relation = imported.engine.catalog().relation("items").unwrap();
    let state = &relation.semantics.as_ref().unwrap().fields["state"];
    assert_eq!(
        state.comparison_profile,
        Some(ComparisonProfile::BinaryExact)
    );
    assert!(
        state
            .source_refs
            .iter()
            .any(|reference| { reference.path.ends_with("/fields/2/custom_extensions/0") })
    );
    let compilation = compile_rows(
        &imported.engine,
        enum_query(false),
        CompileOptions::default(),
    )
    .await;
    let TypedOutcome::Compiled { query } = compilation.outcome else {
        panic!(
            "binary-exact comparison rejected: {:?}",
            compilation.outcome
        )
    };
    let ids = |batches: &[RecordBatch]| {
        batches
            .iter()
            .flat_map(|batch| {
                let column = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| column.value(row))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let direct = query
        .plan_direct(&imported.engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&imported.engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(ids(&direct), [1, 3]);
    assert_eq!(ids(&sql), [1, 3]);
}

#[test]
fn comparison_extensions_reject_locale_unknown_malformed_and_duplicate_profiles() {
    for contract in [
        json!({"kind":"comparison_profile","profile":{"kind":"locale","tag":"de-CH"}}),
        json!({"kind":"comparison_profile","profile":{"kind":"unknown"}}),
        json!({"kind":"comparison_profile"}),
        json!({"kind":"comparison_profile","profile":{"kind":"binary_exact"},"opaque":true}),
    ] {
        let mut value = comparison_model();
        value["semantic_model"][0]["datasets"][0]["fields"][2]["custom_extensions"] =
            json!([extension(contract)]);
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("unsupported comparison profile was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_extension"
                    && diagnostic.path.ends_with("/fields/2/custom_extensions/0")
            }),
            "{diagnostics:?}"
        );
    }

    let mut duplicate = comparison_model();
    duplicate["semantic_model"][0]["datasets"][0]["fields"][2]["custom_extensions"]
        .as_array_mut()
        .unwrap()
        .push(extension(json!({
            "kind":"comparison_profile","profile":{"kind":"binary_exact"}
        })));
    let document = OssieDocument::parse(&duplicate.to_string()).unwrap();
    let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
        panic!("duplicate comparison profile was imported")
    };
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_extension"
            && diagnostic.path.ends_with("/fields/2/custom_extensions/1")
    }));

    let mut wrong_type = comparison_model();
    let comparison_extension =
        wrong_type["semantic_model"][0]["datasets"][0]["fields"][2]["custom_extensions"].clone();
    wrong_type["semantic_model"][0]["datasets"][0]["fields"][1]["custom_extensions"] =
        comparison_extension;
    error_code(wrong_type, "invalid_extension");

    let mut coexist = comparison_model();
    coexist["semantic_model"][0]["datasets"][0]["fields"][2]["custom_extensions"]
        .as_array_mut()
        .unwrap()
        .push(extension(
            json!({"kind":"reference_system","id":"status-codes"}),
        ));
    let document = OssieDocument::parse(&coexist.to_string()).unwrap();
    let imported = document.load(None, &bindings()).unwrap();
    let field = &imported
        .engine
        .catalog()
        .relation("items")
        .unwrap()
        .semantics
        .as_ref()
        .unwrap()
        .fields["state"];
    assert_eq!(
        field.comparison_profile,
        Some(ComparisonProfile::BinaryExact)
    );
    assert_eq!(field.reference_system.as_ref().unwrap().id, "status-codes");
}

fn enum_query(mapped: bool) -> RowQuery {
    let state = FieldRef {
        instance: "i".into(),
        field: "state".into(),
    };
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "items".into(),
            instance: "i".into(),
        },
        requirements: vec![
            Requirement {
                id: "state".into(),
                source_text: "active items".into(),
                operation: RowOperation::Filter {
                    predicate: if mapped {
                        RowPredicate::CompareMapped {
                            field: state,
                            operator: Comparison::Eq,
                            mapping: "status_codes".into(),
                            phrase: "active".into(),
                        }
                    } else {
                        RowPredicate::Compare {
                            field: state,
                            operator: Comparison::Eq,
                            value: Literal::Utf8("open".into()),
                        }
                    },
                },
            },
            Requirement {
                id: "id".into(),
                source_text: "item ID".into(),
                operation: RowOperation::Project {
                    field: FieldRef {
                        instance: "i".into(),
                        field: "id".into(),
                    },
                    alias: "id".into(),
                },
            },
            Requirement {
                id: "order".into(),
                source_text: "sort IDs".into(),
                operation: RowOperation::Order {
                    field: FieldRef {
                        instance: "i".into(),
                        field: "id".into(),
                    },
                    direction: Direction::Asc,
                    nulls: NullOrder::Last,
                },
            },
        ],
        unresolved: vec![],
    }
}

fn calendar_model() -> Value {
    json!({"version":"0.2.0.dev0","semantic_model":[{
        "name":"shop","datasets":[{"name":"events","source":"fixture:events","fields":[
            {"name":"gregorian_at","datatype":"DateTimeTz","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"gregorian_at"}]},
             "custom_extensions":[extension(json!({"kind":"calendar_reference","system":{"kind":"gregorian"},"timezone":"UTC"}))]},
            {"name":"fiscal_at","datatype":"DateTimeTz","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"fiscal_at"}]},
             "custom_extensions":[extension(json!({"kind":"calendar_reference","system":{"kind":"fiscal","id":"company-445"},"timezone":"Europe/Zurich"}))]}
        ]}]
    }]})
}

fn calendar_bindings() -> SourceBindings {
    let timestamp = DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()));
    let schema = Arc::new(Schema::new(vec![
        Field::new("gregorian_at", timestamp.clone(), false),
        Field::new("fiscal_at", timestamp, false),
    ]));
    let values = vec![
        1_769_903_999_999_999,
        1_769_904_000_000_000,
        1_772_323_199_999_999,
    ];
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(TimestampMicrosecondArray::from(values.clone()).with_timezone_opt(Some("UTC")))
                as ArrayRef,
            Arc::new(TimestampMicrosecondArray::from(values).with_timezone_opt(Some("UTC")))
                as ArrayRef,
        ],
    )
    .unwrap();
    let mut bindings = SourceBindings::new();
    bindings
        .bind(
            "fixture:events",
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    bindings
}

fn calendar_query(field: &str) -> RowQuery {
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "events".into(),
            instance: "e".into(),
        },
        requirements: vec![
            Requirement {
                id: "month".into(),
                source_text: "by UTC month".into(),
                operation: RowOperation::CalendarGroup {
                    field: FieldRef {
                        instance: "e".into(),
                        field: field.into(),
                    },
                    grain: CalendarUnit::Month,
                    timezone: "UTC".into(),
                    alias: "month".into(),
                },
            },
            Requirement {
                id: "count".into(),
                source_text: "count events".into(),
                operation: RowOperation::Aggregate {
                    function: AggregateFunction::Count,
                    field: None,
                    distinct: false,
                    alias: "events".into(),
                },
            },
        ],
        unresolved: vec![],
    }
}

#[tokio::test]
async fn imported_calendar_reference_controls_utc_month_execution() {
    let document = OssieDocument::parse(&calendar_model().to_string()).unwrap();
    let imported = document.load(None, &calendar_bindings()).unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let relation = imported.engine.catalog().relation("events").unwrap();
    let semantics = relation.semantics.as_ref().unwrap();
    let gregorian = &semantics.fields["gregorian_at"];
    assert_eq!(
        gregorian.calendar_reference.as_ref().unwrap().timezone,
        "UTC"
    );
    assert!(
        gregorian
            .source_refs
            .iter()
            .any(|reference| { reference.path.ends_with("/fields/0/custom_extensions/0") })
    );
    assert_eq!(
        semantics.fields["fiscal_at"]
            .calendar_reference
            .as_ref()
            .unwrap()
            .timezone,
        "Europe/Zurich"
    );
    let accepted = compile_rows(
        &imported.engine,
        calendar_query("gregorian_at"),
        CompileOptions::default(),
    )
    .await;
    let TypedOutcome::Compiled { query } = accepted.outcome else {
        panic!("Gregorian UTC month rejected: {:?}", accepted.outcome)
    };
    let rows = |batches: &[RecordBatch]| {
        let mut found = std::collections::BTreeMap::new();
        for batch in batches {
            let months = batch
                .column(0)
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            let counts = batch
                .column(1)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            for row in 0..batch.num_rows() {
                found.insert(months.value(row), counts.value(row));
            }
        }
        found
    };
    let direct = query
        .plan_direct(&imported.engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&imported.engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected =
        std::collections::BTreeMap::from([(1_767_225_600_000_000, 1), (1_769_904_000_000_000, 2)]);
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);

    let fiscal = compile_rows(
        &imported.engine,
        calendar_query("fiscal_at"),
        CompileOptions::default(),
    )
    .await;
    assert!(matches!(
        fiscal.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "calendar_reference"
    ));
}

#[test]
fn calendar_field_extensions_reject_invalid_unknown_and_duplicate_contracts() {
    for contract in [
        json!({"kind":"calendar_reference","system":{"kind":"gregorian"}}),
        json!({"kind":"calendar_reference","system":{"kind":"gregorian"},"timezone":"Bad/Zone"}),
        json!({"kind":"calendar_reference","system":{"kind":"fiscal","id":" "},"timezone":"UTC"}),
        json!({"kind":"calendar_reference","system":{"kind":"gregorian"},"timezone":"UTC","opaque":true}),
        json!({"kind":"unknown","system":{"kind":"gregorian"},"timezone":"UTC"}),
    ] {
        let mut value = calendar_model();
        value["semantic_model"][0]["datasets"][0]["fields"][0]["custom_extensions"] =
            json!([extension(contract)]);
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("invalid calendar extension was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_extension"
                    && diagnostic.path.ends_with("/fields/0/custom_extensions/0")
            }),
            "{diagnostics:?}"
        );
    }

    for second in [
        json!({"kind":"calendar_reference","system":{"kind":"gregorian"},"timezone":"UTC"}),
        json!({"kind":"calendar_reference","system":{"kind":"fiscal","id":"other"},"timezone":"Europe/Zurich"}),
    ] {
        let mut value = calendar_model();
        value["semantic_model"][0]["datasets"][0]["fields"][0]["custom_extensions"]
            .as_array_mut()
            .unwrap()
            .push(extension(second));
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("duplicate or conflicting calendar extension was imported")
        };
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "invalid_extension"
                && diagnostic.path.ends_with("/fields/0/custom_extensions/1")
        }));
    }

    let mut value = calendar_model();
    value["semantic_model"][0]["datasets"][0]["fields"][0]["custom_extensions"] =
        json!([{"vendor_name":"OTHER","data":"{}"}]);
    error_code(value, "unsupported_feature");
}

#[test]
fn calendar_and_reference_system_field_contracts_coexist_without_overwrite() {
    let mut value = calendar_model();
    value["semantic_model"][0]["datasets"][0]["fields"][0]["custom_extensions"]
        .as_array_mut()
        .unwrap()
        .push(extension(
            json!({"kind":"reference_system","id":"event-instants"}),
        ));
    let document = OssieDocument::parse(&value.to_string()).unwrap();
    let imported = document.load(None, &calendar_bindings()).unwrap();
    let relation = imported.engine.catalog().relation("events").unwrap();
    let field = &relation.semantics.as_ref().unwrap().fields["gregorian_at"];
    assert_eq!(
        field.reference_system.as_ref().unwrap().id,
        "event-instants"
    );
    assert_eq!(field.calendar_reference.as_ref().unwrap().timezone, "UTC");
}

#[tokio::test]
async fn imported_enum_mapping_publishes_and_executes_exact_domain() {
    let document = OssieDocument::parse(&enum_mapping_model().to_string()).unwrap();
    let imported = document.load(None, &bindings()).unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let relation = imported.engine.catalog().relation("items").unwrap();
    let semantics = relation.semantics.as_ref().unwrap();
    assert_eq!(
        semantics.fields["state"].enum_domain.as_ref().unwrap().id,
        "order-status"
    );
    assert_eq!(
        semantics.value_mappings["status_codes"]
            .enum_domain
            .as_ref()
            .unwrap()
            .id,
        "order-status"
    );
    assert!(
        semantics.fields["state"]
            .source_refs
            .iter()
            .any(|reference| { reference.path.ends_with("/custom_extensions/0") })
    );
    assert!(
        semantics.value_mappings["status_codes"]
            .source_refs
            .iter()
            .any(|reference| reference.path.ends_with("/custom_extensions/0"))
    );

    let compiled = compile_rows(
        &imported.engine,
        enum_query(true),
        CompileOptions::default(),
    )
    .await;
    let TypedOutcome::Compiled { query } = compiled.outcome else {
        panic!("{:?}", compiled.outcome)
    };
    let rows = |batches: &[RecordBatch]| {
        batches
            .iter()
            .flat_map(|batch| {
                let ids = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| ids.value(row))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let direct = query
        .plan_direct(&imported.engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&imported.engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(rows(&direct), [1, 3]);
    assert_eq!(rows(&sql), [1, 3]);

    let raw = compile_rows(
        &imported.engine,
        enum_query(false),
        CompileOptions::default(),
    )
    .await;
    assert!(matches!(
        raw.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "enum_literal"
    ));
}

#[test]
fn enum_mapping_extension_fails_closed_with_source_paths() {
    for contract in [
        json!({"kind":"enum_mapping","dataset":"items","name":"status_codes","id":"values/status-codes","field":"state","domain":"order-status"}),
        json!({"kind":"enum_mapping","dataset":"items","name":"status_codes","id":"values/status-codes","field":"state","domain":" ","codes":{"active":"open"}}),
        json!({"kind":"enum_mapping","dataset":"items","name":"status_codes","id":"values/status-codes","field":"amount","domain":"order-status","codes":{"active":"open"}}),
        json!({"kind":"enum_mapping","dataset":"items","name":"status_codes","id":"values/status-codes","field":"missing","domain":"order-status","codes":{"active":"open"}}),
        json!({"kind":"enum_mapping","dataset":"items","name":"status_codes","id":"values/status-codes","field":"state","domain":"order-status","codes":{"active":"open"},"opaque":true}),
        json!({"kind":"unknown","dataset":"items","name":"status_codes"}),
    ] {
        let mut value = model();
        value["semantic_model"][0]["custom_extensions"] = json!([extension(contract)]);
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("invalid enum extension was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_extension"
                    && diagnostic.path.ends_with("/custom_extensions/0")
            }),
            "{diagnostics:?}"
        );
    }

    let mut conflict = enum_mapping_model();
    conflict["semantic_model"][0]["custom_extensions"]
        .as_array_mut()
        .unwrap()
        .push(extension(json!({
            "kind":"enum_mapping","dataset":"items","name":"other_codes",
            "id":"values/other-codes","field":"state","domain":"risk-status",
            "codes":{"active":"open"}
        })));
    error_code(conflict, "invalid_extension");

    let mut unknown_vendor = model();
    unknown_vendor["semantic_model"][0]["custom_extensions"] =
        json!([{"vendor_name":"OTHER","data":"{}"}]);
    error_code(unknown_vendor, "unsupported_feature");
}

#[tokio::test]
async fn imported_contract_publishes_and_executes_expected_rows() {
    let document = OssieDocument::parse(&model().to_string()).unwrap();
    let imported = document.load(None, &bindings()).unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let relation = imported.engine.catalog().relation("items").unwrap();
    let semantics = relation.semantics.as_ref().unwrap();
    assert_eq!(
        semantics.metrics["total_amount"].unit,
        semantic_catalog::Presence::Value(semantic_catalog::Unit::Currency { code: "USD".into() })
    );
    assert_eq!(
        semantics.metrics["total_amount"]
            .compatible_dimensions
            .len(),
        1
    );
    assert_eq!(semantics.concepts["active"].description, "Open items");
    let FactResolution::Known {
        value,
        contributors,
    } = &semantics.facts["metric_expression/total_amount"]
    else {
        panic!("expression fact missing");
    };
    assert_eq!(value["expression"], " SUM(amount) ");
    assert!(
        contributors[0]
            .origins
            .iter()
            .any(|origin| origin.path.ends_with("/expression/dialects/0/expression"))
    );

    let query = serde_json::from_value(json!({
        "version":1,"input":{"relation":"items","instance":"i"},"requirements":[
            {"id":"active","source_text":"active","operation":{"kind":"concept_filter","concept":"active"}},
            {"id":"sum","source_text":"total amount","operation":{"kind":"metric","name":"total_amount","alias":"total"}}
        ],"unresolved":[]
    })).unwrap();
    let compilation = compile_rows(&imported.engine, query, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = compilation.outcome else {
        panic!("{:?}", compilation.outcome)
    };
    let batches = query
        .plan_direct(&imported.engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let rows: Vec<_> = batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| array_value_to_string(batch.column(0), row).unwrap())
        })
        .collect();
    assert_eq!(rows, ["10"]);
}

#[tokio::test]
async fn count_star_is_bound_to_its_explicit_zero_empty_contract() {
    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["expression"]["dialects"][0]["expression"] =
        json!("COUNT(*)");
    value["semantic_model"][0]["metrics"][0]["custom_extensions"][0] = extension(json!({
        "kind":"metric","dataset":"items","source_grain":{"entity":null,"keys":[{"relation":"items","field":"id"}]},
        "dimensions":["state"],"unit":{"kind":"named","id":"rows"},"empty":"zero"
    }));
    let document = OssieDocument::parse(&value.to_string()).unwrap();
    let imported = document.load(None, &bindings()).unwrap();
    assert!(
        imported
            .engine
            .catalog()
            .relation("items")
            .unwrap()
            .semantics
            .as_ref()
            .unwrap()
            .metrics["total_amount"]
            .field
            .is_none()
    );
    let query = serde_json::from_value(json!({
        "version":1,"input":{"relation":"items","instance":"i"},"requirements":[
            {"id":"count","source_text":"count items","operation":{"kind":"metric","name":"total_amount","alias":"count"}}
        ],"unresolved":[]
    })).unwrap();
    let result = compile_rows(&imported.engine, query, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let batches = query
        .plan_direct(&imported.engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(array_value_to_string(batches[0].column(0), 0).unwrap(), "3");
}

#[tokio::test]
async fn imported_relationship_uses_unknown_cardinality_and_checked_lookup_rows() {
    let document = OssieDocument::parse(
        &relationship_model_with_systems(Some("customer-ids"), Some("customer-ids")).to_string(),
    )
    .unwrap();
    let imported = document
        .load(None, &relationship_bindings(false, false, false))
        .unwrap();
    imported
        .engine
        .catalog()
        .validate(&PublicationLimits::default())
        .unwrap();
    let relation = imported.engine.catalog().relation("items").unwrap();
    assert_eq!(
        relation.semantics.as_ref().unwrap().fields["customer_id"]
            .reference_system
            .as_ref()
            .map(|system| system.id.as_str()),
        Some("customer-ids")
    );
    assert_eq!(
        imported
            .engine
            .catalog()
            .relation("customers")
            .unwrap()
            .semantics
            .as_ref()
            .unwrap()
            .fields["id"]
            .reference_system
            .as_ref()
            .map(|system| system.id.as_str()),
        Some("customer-ids")
    );
    let relationship = &relation.semantics.as_ref().unwrap().relationships["buyer"];
    assert!(matches!(&relationship.cardinality, FactResolution::Unknown));
    assert!(!relationship.source_refs.is_empty());
    let query = serde_json::from_value(json!({
        "version":1,"input":{"relation":"items","instance":"i"},"requirements":[
            {"id":"id","source_text":"item ID","operation":{"kind":"project","field":{"instance":"i","field":"id"},"alias":"id"}},
            {"id":"buyer","source_text":"buyer label","operation":{"kind":"lookup","relationship":"buyer","role":"buyer","instance":"c","field":"label","alias":"buyer","missing":"null"}}
        ],"unresolved":[]
    })).unwrap();
    let result = compile_rows(&imported.engine, query, CompileOptions::default()).await;
    assert_eq!(result.record.execution_obligations.len(), 1);
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let rows = |batches: &[RecordBatch]| {
        let mut rows = batches
            .iter()
            .flat_map(|batch| {
                (0..batch.num_rows())
                    .map(|row| {
                        (0..batch.num_columns())
                            .map(|column| array_value_to_string(batch.column(column), row).unwrap())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        rows.sort();
        rows
    };
    let direct = query
        .plan_direct(&imported.engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&imported.engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected: Vec<Vec<String>> = vec![
        vec!["1".into(), "CH".into()],
        vec!["2".into(), "GB".into()],
        vec!["3".into(), "CH".into()],
    ];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
}

#[test]
fn field_reference_system_extensions_fail_closed_with_source_paths() {
    for (left, right) in [
        (Some("customer-ids"), Some("supplier-ids")),
        (Some("customer-ids"), None),
    ] {
        let document =
            OssieDocument::parse(&relationship_model_with_systems(left, right).to_string())
                .unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) =
            document.load(None, &relationship_bindings(false, false, false))
        else {
            panic!("mismatched or one-sided key identity was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "relationship_reference_system"
                    && diagnostic.path.ends_with("/relationships/0")
            }),
            "{diagnostics:?}"
        );
    }

    for contract in [
        json!({"kind":"reference_system","id":" "}),
        json!({"kind":"unknown","id":"customer-ids"}),
        json!({"kind":"reference_system","id":"customer-ids","extra":true}),
    ] {
        let mut value = relationship_model();
        value["semantic_model"][0]["datasets"][0]["fields"][3]["custom_extensions"] =
            json!([extension(contract)]);
        let document = OssieDocument::parse(&value.to_string()).unwrap();
        let Err(ImportError::Diagnostics(diagnostics)) = document.inspect(None) else {
            panic!("invalid field extension was imported")
        };
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_extension"
                    && diagnostic
                        .path
                        .ends_with("/datasets/0/fields/3/custom_extensions/0")
            }),
            "{diagnostics:?}"
        );
    }

    let mut value = relationship_model();
    value["semantic_model"][0]["datasets"][0]["fields"][3]["custom_extensions"] =
        json!([{"vendor_name":"OTHER","data":"{}"}]);
    error_code(value, "unsupported_feature");

    let mut value = relationship_model();
    let contract = extension(json!({"kind":"reference_system","id":"customer-ids"}));
    value["semantic_model"][0]["datasets"][0]["fields"][3]["custom_extensions"] =
        json!([contract.clone(), contract]);
    error_code(value, "invalid_extension");
}

#[tokio::test]
async fn imported_relationship_duplicate_right_fails_in_both_backends() {
    let document = OssieDocument::parse(&relationship_model().to_string()).unwrap();
    let imported = document
        .load(None, &relationship_bindings(true, false, false))
        .unwrap();
    let query = serde_json::from_value(json!({
        "version":1,"input":{"relation":"items","instance":"i"},"requirements":[
            {"id":"buyer","source_text":"buyer label","operation":{"kind":"lookup","relationship":"buyer","role":"buyer","instance":"c","field":"label","alias":"buyer","missing":"null"}}
        ],"unresolved":[]
    })).unwrap();
    let result = compile_rows(&imported.engine, query, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    assert!(
        query
            .plan_direct(&imported.engine)
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
    assert!(
        query
            .execute(&imported.engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
}

#[test]
fn relationship_import_rejects_opaque_extensions_invalid_keys_and_type_mismatch() {
    let mut value = relationship_model();
    value["semantic_model"][0]["relationships"][0]["custom_extensions"] = json!([extension(
        json!({"kind":"relationship","unrecognized":"opaque"})
    )]);
    error_code(value, "unsupported_feature");

    let mut value = relationship_model();
    value["semantic_model"][0]["relationships"][0]["from_columns"] = json!(["hidden"]);
    error_code(value, "relationship_keys");

    let mut value = relationship_model();
    value["semantic_model"][0]["relationships"][0]["from_columns"] = json!([]);
    value["semantic_model"][0]["relationships"][0]["to_columns"] = json!([]);
    let Err(ImportError::Diagnostics(diagnostics)) = OssieDocument::parse(&value.to_string())
    else {
        panic!("zero relationship key pairs must fail the bundled schema");
    };
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "schema")
    );

    let mut value = relationship_model();
    value["semantic_model"][0]["datasets"][0]["fields"][3]["expression"]["dialects"][0]["expression"] =
        json!("id + 1");
    error_code(value, "unsupported_expression");

    let mut value = relationship_model();
    value["semantic_model"][0]["datasets"][1]["fields"][0]["datatype"] = json!("String");
    let document = OssieDocument::parse(&value.to_string()).unwrap();
    let error = document
        .load(None, &relationship_bindings(false, true, false))
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("relationship_key_type"),
        "{error}"
    );
}

#[test]
fn unsupported_metric_and_vendor_extensions_fail_closed() {
    let mut legacy_unit = model();
    legacy_unit["semantic_model"][0]["metrics"][0]["custom_extensions"][0] = extension(
        json!({"kind":"metric","dataset":"items","source_grain":{"entity":null,"keys":[{"relation":"items","field":"id"}]},"dimensions":["state"],"unit":"USD","empty":"null"}),
    );
    error_code(legacy_unit, "invalid_extension");

    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["expression"]["dialects"][0]["expression"] =
        json!("SUM(amount + 1)");
    error_code(value, "unsupported_expression");

    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["expression"]["dialects"][0]["expression"] =
        json!("SUM(CAST(amount AS BIGINT))");
    error_code(value, "unsupported_expression");

    for expression in [
        "SUM(DISTINCT amount)",
        "SUM(amount) FILTER (WHERE state = 'open')",
        "SUM(amount); DROP TABLE items",
        "COUNT(amount + 1)",
    ] {
        let mut value = model();
        value["semantic_model"][0]["metrics"][0]["expression"]["dialects"][0]["expression"] =
            json!(expression);
        error_code(value, "unsupported_expression");
    }

    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["custom_extensions"][0]["vendor_name"] =
        json!("OTHER");
    error_code(value, "unsupported_feature");

    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["custom_extensions"][0]["data"] =
        json!(r#"{"kind":"metric","dataset":"items"}"#);
    error_code(value, "invalid_extension");
}

#[test]
fn concept_predicates_and_metric_contracts_are_bounded() {
    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["custom_extensions"][0] = extension(json!({
        "kind":"metric","dataset":"items","source_grain":["id"],"dimensions":["state"],
        "unit":{"kind":"currency","code":"USD"},"empty":"null"
    }));
    error_code(value, "invalid_extension");

    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["custom_extensions"][0] = extension(json!({
        "kind":"metric","dataset":"items","source_grain":{"entity":null,"keys":[{"relation":"other","field":"id"}]},
        "dimensions":["state"],"unit":{"kind":"currency","code":"USD"},"empty":"null"
    }));
    error_code(value, "invalid_metric_contract");

    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["custom_extensions"][0] = extension(json!({
        "kind":"metric","dataset":"items","source_grain":{"entity":null,"keys":[
            {"relation":"items","field":"id"},{"relation":"items","field":"id"}
        ]},"dimensions":["state"],"unit":{"kind":"currency","code":"USD"},"empty":"null"
    }));
    error_code(value, "invalid_metric_contract");

    let mut value = model();
    value["semantic_model"][0]["custom_extensions"][0] = extension(json!({
        "kind":"concept","dataset":"items","name":"active","description":"bad",
        "predicate":{"kind":"compare","field":"secret","operator":"eq","value":{"type":"utf8","value":"open"}}
    }));
    error_code(value, "invalid_concept_predicate");

    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["custom_extensions"][0] = extension(json!({
        "kind":"metric","dataset":"items","source_grain":{"entity":null,"keys":[{"relation":"items","field":"id"}]},"dimensions":["state"],"unit":{"kind":"currency","code":"USD"},"empty":"zero"
    }));
    error_code(value, "invalid_metric_contract");
}

#[tokio::test]
async fn imported_representation_profiles_do_not_forbid_typed_aggregation() {
    let mut value = model();
    value["semantic_model"][0]["custom_extensions"].as_array_mut().unwrap().push(extension(json!({
        "kind":"view_lineage","name":"selected_items","source_dataset":"items","columns":{"id":"id","state":"state"}
    })));
    let document = OssieDocument::parse(&value.to_string()).unwrap();
    let imported = document.load(None, &bindings()).unwrap();
    for (name, profile) in [
        ("items", "ossie/source-bound-dataset"),
        ("selected_items", "ossie/direct-projection-view"),
    ] {
        let context = semantic_interpreter::catalog_context(imported.engine.catalog());
        let relation = context
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .unwrap();
        assert_eq!(relation["semantics"]["capability"]["profile"], profile);
        assert_eq!(relation["semantics"]["capability"]["revision"], "2");
        let mut requirements = vec![Requirement {
            id: "count".into(),
            source_text: "count items".into(),
            operation: RowOperation::Aggregate {
                function: AggregateFunction::Count,
                field: None,
                distinct: false,
                alias: "count".into(),
            },
        }];
        if name == "items" {
            requirements.insert(
                0,
                Requirement {
                    id: "open".into(),
                    source_text: "open items".into(),
                    operation: RowOperation::Filter {
                        predicate: RowPredicate::Compare {
                            field: FieldRef {
                                instance: "i".into(),
                                field: "state".into(),
                            },
                            operator: Comparison::Eq,
                            value: Literal::Utf8("open".into()),
                        },
                    },
                },
            );
            requirements.insert(
                1,
                Requirement {
                    id: "state".into(),
                    source_text: "by state".into(),
                    operation: RowOperation::Group {
                        field: FieldRef {
                            instance: "i".into(),
                            field: "state".into(),
                        },
                        alias: "state".into(),
                    },
                },
            );
        }
        let result = compile_rows(
            &imported.engine,
            RowQuery {
                version: 1,
                input: RelationInput {
                    relation: name.into(),
                    instance: "i".into(),
                },
                requirements,
                unresolved: vec![],
            },
            CompileOptions::default(),
        )
        .await;
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("{name}: {:?}", result.outcome)
        };
        let batches = query
            .execute(&imported.engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
        let last = batches[0].num_columns() - 1;
        assert_eq!(
            array_value_to_string(batches[0].column(last).as_ref(), 0).unwrap(),
            if name == "items" { "2" } else { "3" }
        );
    }
}

#[tokio::test]
async fn representation_profile_guidance_keeps_nonexecutable_capability_gate() {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1]))]).unwrap();
    let mut relation = semantic_catalog::Relation::base("descriptive", schema.clone(), "memory");
    relation.semantics = Some(semantic_catalog::RelationSemantics {
        capability: Some(semantic_catalog::Capability::DescriptiveOnly {
            reason: "inspection only".into(),
        }),
        ..Default::default()
    });
    let mut engine = semantic_engine::Engine::new();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    let result = compile_rows(
        &engine,
        RowQuery {
            version: 1,
            input: RelationInput {
                relation: "descriptive".into(),
                instance: "d".into(),
            },
            requirements: vec![Requirement {
                id: "count".into(),
                source_text: "count rows".into(),
                operation: RowOperation::Aggregate {
                    function: AggregateFunction::Count,
                    field: None,
                    distinct: false,
                    alias: "count".into(),
                },
            }],
            unresolved: vec![],
        },
        CompileOptions::default(),
    )
    .await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "catalog_capability")
    );
}
