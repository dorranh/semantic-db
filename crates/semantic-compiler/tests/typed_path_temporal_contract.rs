use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, Date32Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    FactResolution, Relation, RelationSemantics, RelationshipDefinition, RelationshipKey,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use serde_json::json;

fn relationship(name: &str, right: &str, left_key: &str) -> RelationshipDefinition {
    RelationshipDefinition {
        id: format!("relationships/{name}"),
        ai_context: None,
        right_relation: right.into(),
        role: name.into(),
        key_pairs: vec![RelationshipKey {
            left_field: left_key.into(),
            right_field: "id".into(),
        }],
        null_keys_match: false,
        cardinality: FactResolution::Unknown,
        source_refs: vec![],
    }
}

fn table(schema: Arc<Schema>, columns: Vec<ArrayRef>) -> Arc<MemTable> {
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap())
}

fn fixture(nullable_middle_time: bool, nullable_second_endpoint: bool) -> Engine {
    let mut engine = Engine::new();
    let orders_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("customer_id", DataType::Int64, false),
        Field::new("fact_time", DataType::Date32, false),
    ]));
    let mut orders = Relation::base("orders", orders_schema.clone(), "memory");
    orders.semantics = Some(RelationSemantics {
        relationships: [(
            "customer".into(),
            relationship("customer", "customers", "customer_id"),
        )]
        .into(),
        ..Default::default()
    });
    engine
        .register_table(
            orders,
            table(
                orders_schema,
                vec![
                    Arc::new(Int64Array::from(vec![1, 2])),
                    Arc::new(Int64Array::from(vec![10, 99])),
                    Arc::new(Date32Array::from(vec![5, 5])),
                ],
            ),
        )
        .unwrap();

    let customers_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("manager_id", DataType::Int64, false),
        Field::new("requested_at", DataType::Date32, nullable_middle_time),
        Field::new("valid_from", DataType::Date32, false),
        Field::new("valid_to", DataType::Date32, false),
    ]));
    let mut customers = Relation::base("customers", customers_schema.clone(), "memory");
    customers.semantics = Some(RelationSemantics {
        relationships: [(
            "manager".into(),
            relationship("manager", "managers", "manager_id"),
        )]
        .into(),
        ..Default::default()
    });
    engine
        .register_table(
            customers,
            table(
                customers_schema,
                vec![
                    Arc::new(Int64Array::from(vec![10])),
                    Arc::new(Int64Array::from(vec![100])),
                    Arc::new(Date32Array::from(vec![6])),
                    Arc::new(Date32Array::from(vec![0])),
                    Arc::new(Date32Array::from(vec![10])),
                ],
            ),
        )
        .unwrap();

    let managers_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("valid_from", DataType::Date32, nullable_second_endpoint),
        Field::new("valid_to", DataType::Date32, false),
    ]));
    engine
        .register_table(
            Relation::base("managers", managers_schema.clone(), "memory"),
            table(
                managers_schema,
                vec![
                    Arc::new(Int64Array::from(vec![100])),
                    Arc::new(StringArray::from(vec!["Alice"])),
                    Arc::new(Date32Array::from(vec![0])),
                    Arc::new(Date32Array::from(vec![10])),
                ],
            ),
        )
        .unwrap();
    engine
}

fn proposal() -> semantic_plan::typed::RowQuery {
    serde_json::from_value(json!({
        "version":1,
        "input":{"relation":"orders","instance":"o"},
        "requirements":[
            {"id":"id","source_text":"order ID","operation":{"kind":"project","field":{"instance":"o","field":"id"},"alias":"id"}},
            {"id":"manager","source_text":"manager at the customer's requested date","operation":{
                "kind":"path_lookup",
                "hops":[
                    {"relationship":"customer","role":"customer","instance":"c","as_of":{"fact_time":"fact_time","valid_from":"valid_from","valid_to":"valid_to","timezone":null}},
                    {"relationship":"manager","role":"manager","instance":"m","as_of":{"fact_time":"requested_at","valid_from":"valid_from","valid_to":"valid_to","timezone":null}}
                ],
                "field":"label","alias":"manager","missing":"null"
            }}
        ],
        "unresolved":[]
    }))
    .unwrap()
}

#[tokio::test]
async fn both_temporal_hops_preserve_missing_first_hop_and_sql_direct_rows() {
    let engine = fixture(false, false);
    let result = compile_rows(&engine, proposal(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("both temporal hops rejected: {:?}", result.outcome)
    };
    let values = |batches: &[RecordBatch]| {
        let mut rows = batches
            .iter()
            .flat_map(|batch| {
                (0..batch.num_rows()).map(|row| {
                    batch
                        .columns()
                        .iter()
                        .map(|column| array_value_to_string(column, row).unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        rows.sort();
        rows
    };
    let direct = query
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected: Vec<Vec<String>> = vec![
        vec!["1".into(), "Alice".into()],
        vec!["2".into(), "".into()],
    ];
    assert_eq!(values(&direct), expected);
    assert_eq!(values(&sql), expected);
}

#[tokio::test]
async fn second_hop_rejects_nullable_authored_time_and_interval_endpoint() {
    for (middle_time, endpoint) in [(true, false), (false, true)] {
        let engine = fixture(middle_time, endpoint);
        let result = compile_rows(&engine, proposal(), CompileOptions::default()).await;
        assert!(
            matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "path_lookup_contract"),
            "nullable middle time: {middle_time}, nullable endpoint: {endpoint}"
        );
    }
}
