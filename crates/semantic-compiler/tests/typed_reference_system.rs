use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    FactResolution, FieldSemantics, ReferenceSystem, Relation, RelationSemantics,
    RelationshipDefinition, RelationshipKey,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    Direction, FieldRef, LookupUsage, MissingMatch, NullOrder, RelationInput, Requirement,
    RowOperation, RowQuery,
};

fn fixture() -> Engine {
    let mut engine = Engine::new();
    let right_schema = Arc::new(Schema::new(vec![
        Field::new("code", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let right_batch = RecordBatch::try_new(
        right_schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["A", "B"])),
            Arc::new(StringArray::from(vec!["CH", "GB"])),
        ],
    )
    .unwrap();
    let mut right = Relation::base("regions", right_schema.clone(), "memory");
    right.semantics = Some(RelationSemantics {
        fields: [(
            "code".into(),
            FieldSemantics {
                reference_system: Some(ReferenceSystem {
                    id: "region-codes".into(),
                }),
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    });
    engine
        .register_table(
            right,
            Arc::new(MemTable::try_new(right_schema, vec![vec![right_batch]]).unwrap()),
        )
        .unwrap();

    let left_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("code", DataType::Utf8, false),
    ]));
    let left_batch = RecordBatch::try_new(
        left_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["A", "B"])),
        ],
    )
    .unwrap();
    let mut left = Relation::base("orders", left_schema.clone(), "memory");
    left.semantics = Some(RelationSemantics {
        fields: [(
            "code".into(),
            FieldSemantics {
                reference_system: Some(ReferenceSystem {
                    id: "region-codes".into(),
                }),
                ..Default::default()
            },
        )]
        .into(),
        relationships: [(
            "region".into(),
            RelationshipDefinition {
                ai_context: None,
                id: "relationships/order-region".into(),
                right_relation: "regions".into(),
                role: "region".into(),
                key_pairs: vec![RelationshipKey {
                    left_field: "code".into(),
                    right_field: "code".into(),
                }],
                null_keys_match: false,
                cardinality: FactResolution::Unknown,
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    engine
        .register_table(
            left,
            Arc::new(MemTable::try_new(left_schema, vec![vec![left_batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "orders".into(),
            instance: "o".into(),
        },
        requirements: vec![
            Requirement {
                id: "id".into(),
                source_text: "order ID".into(),
                operation: RowOperation::Project {
                    field: FieldRef {
                        instance: "o".into(),
                        field: "id".into(),
                    },
                    alias: "id".into(),
                },
            },
            Requirement {
                id: "region".into(),
                source_text: "region label".into(),
                operation: RowOperation::Lookup {
                    relationship: "region".into(),
                    role: "region".into(),
                    instance: "r".into(),
                    field: "label".into(),
                    alias: "region".into(),
                    missing: MissingMatch::Null,
                    usage: LookupUsage::Project,
                },
            },
            Requirement {
                id: "order".into(),
                source_text: "order by ID".into(),
                operation: RowOperation::Order {
                    field: FieldRef {
                        instance: "o".into(),
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

#[tokio::test]
async fn matching_authored_reference_systems_execute_lookup_in_both_backends() {
    let engine = fixture();
    let compiled = compile_rows(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = compiled.outcome else {
        panic!(
            "matching reference systems rejected: {:?}",
            compiled.outcome
        )
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
    for batches in [&direct, &sql] {
        let rows: Vec<Vec<String>> = batches
            .iter()
            .flat_map(|batch| {
                (0..batch.num_rows()).map(|row| {
                    (0..batch.num_columns())
                        .map(|column| array_value_to_string(batch.column(column), row).unwrap())
                        .collect()
                })
            })
            .collect();
        assert_eq!(rows, vec![vec!["1", "CH"], vec!["2", "GB"]]);
    }
}
