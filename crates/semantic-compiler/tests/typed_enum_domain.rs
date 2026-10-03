use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{EnumDomain, FieldSemantics, Relation, RelationSemantics, ValueMapping};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    Comparison, Direction, FieldRef, Literal, NullOrder, ROW_QUERY_VERSION, RelationInput,
    Requirement, RowOperation, RowPredicate, RowQuery,
};

fn engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("order_status", DataType::Utf8, false),
        Field::new("risk_status", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            Arc::new(StringArray::from(vec!["A", "B", "A"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["B", "A", "B"])) as ArrayRef,
        ],
    )
    .unwrap();
    let mut relation = Relation::base("events", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        fields: [
            (
                "order_status".into(),
                FieldSemantics {
                    enum_domain: Some(EnumDomain {
                        id: "order-status".into(),
                    }),
                    ..Default::default()
                },
            ),
            (
                "risk_status".into(),
                FieldSemantics {
                    enum_domain: Some(EnumDomain {
                        id: "risk-status".into(),
                    }),
                    ..Default::default()
                },
            ),
        ]
        .into(),
        value_mappings: [
            (
                "active_orders".into(),
                ValueMapping {
                    id: "values/active-orders".into(),
                    field: "order_status".into(),
                    description: "Active order status".into(),
                    codes: [("active".into(), "A".into())].into(),
                    enum_domain: Some(EnumDomain {
                        id: "order-status".into(),
                    }),
                    source_refs: vec![],
                },
            ),
            (
                "active_risks".into(),
                ValueMapping {
                    id: "values/active-risks".into(),
                    field: "risk_status".into(),
                    description: "Active risk status".into(),
                    codes: [("active".into(), "A".into())].into(),
                    enum_domain: Some(EnumDomain {
                        id: "risk-status".into(),
                    }),
                    source_refs: vec![],
                },
            ),
        ]
        .into(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query(field: &str, mapping: &str, mapped: bool) -> RowQuery {
    let reference = FieldRef {
        instance: "e".into(),
        field: field.into(),
    };
    let predicate = if mapped {
        RowPredicate::CompareMapped {
            field: reference,
            operator: Comparison::Eq,
            mapping: mapping.into(),
            phrase: "active".into(),
        }
    } else {
        RowPredicate::Compare {
            field: reference,
            operator: Comparison::Eq,
            value: Literal::Utf8("A".into()),
        }
    };
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "events".into(),
            instance: "e".into(),
        },
        requirements: vec![
            Requirement {
                id: "status".into(),
                source_text: "active status".into(),
                operation: RowOperation::Filter { predicate },
            },
            Requirement {
                id: "id".into(),
                source_text: "event ID".into(),
                operation: RowOperation::Project {
                    field: FieldRef {
                        instance: "e".into(),
                        field: "id".into(),
                    },
                    alias: "id".into(),
                },
            },
            Requirement {
                id: "order".into(),
                source_text: "order by ID".into(),
                operation: RowOperation::Order {
                    field: FieldRef {
                        instance: "e".into(),
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

fn rows(batches: &[RecordBatch]) -> Vec<i64> {
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
        .collect()
}

#[tokio::test]
async fn exact_mapped_domain_executes_while_raw_enum_code_is_rejected() {
    let engine = engine();
    for (field, mapping, expected) in [
        ("order_status", "active_orders", vec![1, 3]),
        ("risk_status", "active_risks", vec![2]),
    ] {
        let compiled = compile_rows(
            &engine,
            query(field, mapping, true),
            CompileOptions::default(),
        )
        .await;
        let TypedOutcome::Compiled { query } = compiled.outcome else {
            panic!("exact mapped domain rejected: {:?}", compiled.outcome)
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
        assert_eq!(rows(&direct), expected);
        assert_eq!(rows(&sql), expected);
    }

    let rejected = compile_rows(
        &engine,
        query("order_status", "active_orders", false),
        CompileOptions::default(),
    )
    .await;
    assert!(matches!(
        rejected.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "enum_literal"
    ));
}
