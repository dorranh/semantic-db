use datafusion::{
    arrow::{
        array::{
            ArrayRef, BooleanArray, Date32Array, Float32Array, Int16Array, Int32Array, Int64Array,
            StringArray,
        },
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{QueryOptions, TableProvider};
use semantic_ossie::{OssieDocument, SourceBindings};
use semantic_plan::typed::RowQuery;
use serde_json::{Value, json};
use std::sync::Arc;

fn extension(value: Value) -> Value {
    json!({"vendor_name":"SEMANTIC_DB","data":value.to_string()})
}
fn field(name: &str, datatype: &str) -> Value {
    json!({"name":name,"datatype":datatype,"expression":{"dialects":[{"dialect":"ANSI_SQL","expression":name}]}})
}
fn calendar_rule() -> Value {
    json!({"version":1,"id":"calendars/events-fiscal","source_relation":"events","calendar_relation":"days","source_date_field":"day","calendar_date_field":"day","fiscal_year_field":"year","fiscal_period_field":"period","business_day_field":"business","source_basis":"date32","timezone":"Europe/Zurich","mapping_revision":"calendar-fixture-v1","source_refs":[]})
}
fn metric_contract(mean: bool) -> Value {
    let mut contract = json!({"kind":"metric","dataset":"events","source_grain":{"entity":null,"keys":[{"relation":"events","field":"id"}]},"dimensions":["status"],"unit":{"kind":"named","id":"minor"},"empty":"null","lookup_dimensions":[{"relationship":"owner","field":"name","missing":"exclude"}]});
    if mean {
        contract["state"] =
            json!({"version":2,"state":{"kind":"sum_count_average"},"merge_dimensions":["status"]});
        contract["result_type"] = json!({"Decimal128":[38,18]});
        contract["row_filters"] =
            json!([{"field":"status","operator":"eq","value":{"type":"utf8","value":"open"}}]);
    }
    contract
}
fn model() -> Value {
    let mut weight = field("weight", "Float");
    weight["custom_extensions"] = json!([extension(
        json!({"kind":"unit","unit":{"kind":"named","id":"kg"}})
    )]);
    json!({"version":"0.2.0.dev0","semantic_model":[{"name":"fixture","datasets":[
        {"name":"events","source":"memory:events","fields":[field("id","Integer"),field("account_id","Integer"),field("amount","Integer"),field("status","String"),field("day","Date"),weight]},
        {"name":"accounts","source":"memory:accounts","fields":[field("id","Integer"),field("name","String")]},
        {"name":"days","source":"memory:days","fields":[field("day","Date"),field("year","Integer"),field("period","Integer"),field("business","Boolean")]}
    ],"relationships":[{"name":"owner","from":"events","to":"accounts","from_columns":["account_id"],"to_columns":["id"]}],
    "metrics":[
        {"name":"total","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"SUM(amount)"}]},"custom_extensions":[extension(metric_contract(false))]},
        {"name":"mean_open","datatype":"Decimal","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"SUM(amount)"}]},"custom_extensions":[extension(metric_contract(true))]}
    ],"custom_extensions":[extension(json!({"kind":"business_calendar","dataset":"events","name":"fiscal","rule":calendar_rule()}))]}]})
}
fn bind(bindings: &mut SourceBindings, name: &str, fields: Vec<Field>, arrays: Vec<ArrayRef>) {
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
    let provider: Arc<dyn TableProvider> =
        Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap());
    bindings.bind(format!("memory:{name}"), provider).unwrap();
}
fn bindings(narrow_year: bool) -> SourceBindings {
    let mut bindings = SourceBindings::new();
    bind(
        &mut bindings,
        "events",
        vec![
            Field::new("id", DataType::Int64, false),
            Field::new("account_id", DataType::Int64, false),
            Field::new("amount", DataType::Int64, false),
            Field::new("status", DataType::Utf8, false),
            Field::new("day", DataType::Date32, false),
            Field::new("weight", DataType::Float32, false),
        ],
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Int64Array::from(vec![1, 2, 2])),
            Arc::new(Int64Array::from(vec![4, 9, 6])),
            Arc::new(StringArray::from(vec!["open", "closed", "open"])),
            Arc::new(Date32Array::from(vec![19782, 19783, 19783])),
            Arc::new(Float32Array::from(vec![1.25, 2.5, 0.75])),
        ],
    );
    bind(
        &mut bindings,
        "accounts",
        vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ],
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["A", "B"])),
        ],
    );
    let (year_type, years): (DataType, ArrayRef) = if narrow_year {
        (
            DataType::Int16,
            Arc::new(Int16Array::from(vec![2024, 2024])),
        )
    } else {
        (
            DataType::Int32,
            Arc::new(Int32Array::from(vec![2024, 2024])),
        )
    };
    bind(
        &mut bindings,
        "days",
        vec![
            Field::new("day", DataType::Date32, false),
            Field::new("year", year_type, false),
            Field::new("period", DataType::Int16, false),
            Field::new("business", DataType::Boolean, false),
        ],
        vec![
            Arc::new(Date32Array::from(vec![19782, 19783])),
            years,
            Arc::new(Int16Array::from(vec![1, 2])),
            Arc::new(BooleanArray::from(vec![true, true])),
        ],
    );
    bindings
}
fn rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| {
                (0..batch.num_columns())
                    .map(|column| array_value_to_string(batch.column(column), row).unwrap())
                    .collect()
            })
        })
        .collect()
}
async fn parity(model: Value, requirements: Value, expected: Vec<Vec<&str>>) {
    let imported = OssieDocument::parse(&model.to_string())
        .unwrap()
        .load(None, &bindings(false))
        .unwrap();
    let proposal: RowQuery = serde_json::from_value(json!({"version":1,"input":{"relation":"events","instance":"e"},"requirements":requirements,"unresolved":[]})).unwrap();
    let result = compile_rows(&imported.engine, proposal, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let mut expected = expected
        .into_iter()
        .map(|row| row.into_iter().map(str::to_owned).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    expected.sort();
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
    let mut direct_rows = rows(&direct);
    direct_rows.sort();
    let mut sql_rows = rows(&sql);
    sql_rows.sort();
    assert_eq!(direct_rows, expected);
    assert_eq!(sql_rows, expected);
}

#[tokio::test]
async fn imported_mean_filters_and_lookup_permissions_have_sql_direct_parity() {
    parity(model(),json!([{ "id":"mean","source_text":"mean open amount","operation":{"kind":"metric","name":"mean_open","alias":"mean"}}]),vec![vec!["5.000000000000000000"]]).await;
    parity(model(),json!([
        {"id":"owner","source_text":"by owner","operation":{"kind":"lookup","relationship":"owner","role":"owner","instance":"a","field":"name","alias":"owner","missing":"exclude","usage":"group"}},
        {"id":"total","source_text":"total amount","operation":{"kind":"metric","name":"total","alias":"total"}},
        {"id":"sort","source_text":"sort owner","operation":{"kind":"order_output","slot":"owner","direction":"asc","nulls":"last"}}
    ]),vec![vec!["A","4"],vec!["B","15"]]).await;
}

#[tokio::test]
async fn imported_min_max_and_average_keep_explicit_result_types() {
    for (function, datatype, result_type, expected) in [
        ("MIN", "Integer", json!("Int64"), "4"),
        ("MAX", "Integer", json!("Int64"), "9"),
        (
            "AVG",
            "Decimal",
            json!({"Decimal128":[38,18]}),
            "6.333333333333333333",
        ),
    ] {
        let mut value = model();
        let mut contract = metric_contract(false);
        contract["result_type"] = result_type;
        value["semantic_model"][0]["metrics"][0]["datatype"] = json!(datatype);
        value["semantic_model"][0]["metrics"][0]["expression"]["dialects"][0]["expression"] =
            json!(format!("{function}(amount)"));
        value["semantic_model"][0]["metrics"][0]["custom_extensions"][0] = extension(contract);
        parity(value,json!([{ "id":"measure","source_text":"explicit authored measure","operation":{"kind":"metric","name":"total","alias":"measure"}}]),vec![vec![expected]]).await;
    }
}

#[tokio::test]
async fn imported_business_calendar_binds_exact_mapping_and_provenance() {
    let imported = OssieDocument::parse(&model().to_string())
        .unwrap()
        .load(None, &bindings(false))
        .unwrap();
    let relation = imported.engine.catalog().relation("events").unwrap();
    let semantics = relation.semantics.as_ref().unwrap();
    assert!(
        !semantics.business_calendars["fiscal"]
            .source_refs
            .is_empty()
    );
    assert_eq!(
        semantics.fields["weight"].unit,
        Some(semantic_catalog::Unit::Named { id: "kg".into() })
    );
    parity(model(),json!([{ "id":"period","source_text":"fiscal period","operation":{"kind":"business_calendar","calendar":"fiscal","field":"fiscal_period","alias":"period"}}]),vec![vec!["1"],vec!["2"],vec!["2"]]).await;
}

#[test]
fn malformed_calendar_and_metric_contracts_fail_before_provider_loading() {
    for bad in [
        json!({"version":99}),
        json!({"timezone":"Not/AZone"}),
        json!({"source_relation":"accounts"}),
        json!({"calendar_date_field":"missing"}),
        json!({"mapping_revision":""}),
    ] {
        let mut value = model();
        let mut rule = calendar_rule();
        for (key, value) in bad.as_object().unwrap() {
            rule[key] = value.clone();
        }
        value["semantic_model"][0]["custom_extensions"][0] = extension(
            json!({"kind":"business_calendar","dataset":"events","name":"fiscal","rule":rule}),
        );
        assert!(
            OssieDocument::parse(&value.to_string())
                .unwrap()
                .inspect(None)
                .is_err()
        );
    }
    for bad in [
        json!({"state":{"version":99,"state":{"kind":"sum_count_average"},"merge_dimensions":[]}}),
        json!({"lookup_dimensions":[{"relationship":"missing","field":"name","missing":"exclude"}]}),
        json!({"row_filters":[{"field":"missing","operator":"eq","value":{"type":"int64","value":1}}]}),
        json!({"result_type":"Int64"}),
    ] {
        let mut value = model();
        let mut contract = metric_contract(true);
        for (key, value) in bad.as_object().unwrap() {
            contract[key] = value.clone();
        }
        value["semantic_model"][0]["metrics"][1]["custom_extensions"][0] = extension(contract);
        assert!(
            OssieDocument::parse(&value.to_string())
                .unwrap()
                .inspect(None)
                .is_err()
        );
    }
}

#[test]
fn exact_calendar_physical_types_and_floating_money_are_not_implicitly_promoted() {
    assert!(
        OssieDocument::parse(&model().to_string())
            .unwrap()
            .load(None, &bindings(true))
            .is_err()
    );
    let mut value = model();
    value["semantic_model"][0]["datasets"][0]["fields"][5]["custom_extensions"][0] =
        extension(json!({"kind":"unit","unit":{"kind":"currency","code":"CHF"}}));
    assert!(
        OssieDocument::parse(&value.to_string())
            .unwrap()
            .inspect(None)
            .is_err()
    );
}

#[tokio::test]
async fn imported_parameterized_concept_binds_validated_civil_dates() {
    let mut value = model();
    value["semantic_model"][0]["custom_extensions"].as_array_mut().unwrap().push(extension(json!({"kind":"concept","dataset":"events","name":"through_date","description":"Events through the supplied civil date","predicate":{"kind":"compare_parameter","field":"day","operator":"lt_eq","parameter":"as_of_date"}})));
    parity(value.clone(),json!([
        {"id":"eligible","source_text":"through supplied date","operation":{"kind":"concept_filter","concept":"through_date","arguments":{"as_of_date":{"type":"gregorian_date","value":"2024-02-29"}}}},
        {"id":"ids","source_text":"show id","operation":{"kind":"project","field":{"instance":"e","field":"id"},"alias":"id"}}
    ]),vec![vec!["1"]]).await;
    parity(value.clone(),json!([
        {"id":"eligible","source_text":"through supplied date","operation":{"kind":"concept_filter","concept":"through_date","arguments":{"as_of_date":{"type":"date32","value":19782}}}},
        {"id":"ids","source_text":"show id","operation":{"kind":"project","field":{"instance":"e","field":"id"},"alias":"id"}}
    ]),vec![vec!["1"]]).await;
    let imported = OssieDocument::parse(&value.to_string())
        .unwrap()
        .load(None, &bindings(false))
        .unwrap();
    let invalid: RowQuery = serde_json::from_value(json!({"version":1,"input":{"relation":"events","instance":"e"},"requirements":[
        {"id":"eligible","source_text":"through supplied date","operation":{"kind":"concept_filter","concept":"through_date","arguments":{"as_of_date":{"type":"utf8","value":"2024-02-29"}}}},
        {"id":"ids","source_text":"show id","operation":{"kind":"project","field":{"instance":"e","field":"id"},"alias":"id"}}
    ],"unresolved":[]})).unwrap();
    assert!(matches!(
        compile_rows(&imported.engine, invalid, CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::Rejected { .. }
    ));
}
