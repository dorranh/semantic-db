use datafusion::{
    arrow::{
        array::{ArrayRef, Date32Array, Decimal128Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::QueryOptions;
use semantic_ossie::{OssieDocument, SourceBindings};
use semantic_plan::typed::RowQuery;
use serde_json::{Value, json};
use std::sync::Arc;
fn model() -> Value {
    let fields=[("currency","String"),("day","Date"),("value","Decimal")].into_iter().map(|(name,datatype)|json!({"name":name,"datatype":datatype,"expression":{"dialects":[{"dialect":"ANSI_SQL","expression":name}]}})).collect::<Vec<_>>();
    let contract = json!({"kind":"exact_decimal_rate","name":"chf","dataset":"rates","rule":{"version":1,"id":"rates/chf","rate_relation":"rates","source_currency_field":"currency","date_field":"day","rate_field":"value","target_currency":"CHF","positive_only":true,"null_rate":"unavailable"}});
    json!({"version":"0.2.0.dev0","semantic_model":[{"name":"finance","datasets":[{"name":"rates","source":"memory:rates","fields":fields}],"custom_extensions":[{"vendor_name":"SEMANTIC_DB","data":contract.to_string()}]}]})
}
fn bindings(currency_nullable: bool) -> SourceBindings {
    let schema = Arc::new(Schema::new(vec![
        Field::new("currency", DataType::Utf8, currency_nullable),
        Field::new("day", DataType::Date32, false),
        Field::new("value", DataType::Decimal128(18, 6), false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["USD", "GBP"])) as ArrayRef,
            Arc::new(Date32Array::from(vec![19814, 19814])),
            Arc::new(
                Decimal128Array::from(vec![950050, 1150050])
                    .with_precision_and_scale(18, 6)
                    .unwrap(),
            ),
        ],
    )
    .unwrap();
    let mut bindings = SourceBindings::new();
    bindings
        .bind(
            "memory:rates",
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    bindings
}
#[tokio::test]
async fn imported_authored_rate_roundtrip_executes_exactly_and_retains_provenance() {
    let source = model().to_string();
    let document = OssieDocument::parse(&source).unwrap();
    let imported = document.load(None, &bindings(false)).unwrap();
    let proposal:RowQuery=serde_json::from_value(json!({"version":1,"input":{"relation":"rates","instance":"r"},"requirements":[{"id":"converted","source_text":"convert 100 foreign major units","operation":{"kind":"convert_rate","rate":"chf","amount":{"kind":"literal","value":{"type":"decimal128","value":{"coefficient":"100","precision":8,"scale":0}},"unit":{"kind":"rate_source_currency"},"basis":"major"},"result_type":{"precision":18,"scale":2},"rounding":"half_away_from_zero","alias":"chf"}}],"unresolved":[]})).unwrap();
    let result = compile_rows(&imported.engine, proposal, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    assert!(
        result
            .record
            .definition_refs
            .iter()
            .any(|r| r.id == "rates/chf")
    );
    let native = query
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
    assert_eq!(native[0].schema(), sql[0].schema());
    for batches in [native, sql] {
        let values = batches
            .iter()
            .flat_map(|b| (0..b.num_rows()).map(|r| array_value_to_string(b.column(0), r).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["95.01", "115.01"]);
    }
    let bound = serde_json::to_value(query.bound()).unwrap();
    assert!(
        !bound["requirements"][0]["operation"]["conversion"]["source_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn malformed_profile_and_nullable_currency_fail_before_compilation() {
    assert!(
        OssieDocument::parse(&model().to_string())
            .unwrap()
            .load(None, &bindings(true))
            .is_err()
    );
    for mutation in 0..3 {
        let mut value = model();
        let mut contract: Value = serde_json::from_str(
            value["semantic_model"][0]["custom_extensions"][0]["data"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        match mutation {
            0 => contract["rule"]["positive_only"] = json!(false),
            1 => contract["rule"]["rate_field"] = json!("currency"),
            _ => contract["rule"]["mode"] = json!("lookup"),
        };
        value["semantic_model"][0]["custom_extensions"][0]["data"] = json!(contract.to_string());
        let result = OssieDocument::parse(&value.to_string())
            .and_then(|doc| doc.load(None, &bindings(false)));
        assert!(result.is_err());
    }
}
