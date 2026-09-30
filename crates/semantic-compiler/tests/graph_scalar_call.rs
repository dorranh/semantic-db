use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_graph};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::{graph::*, typed::*};

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("n", DataType::Int64, false),
        Field::new("d", DataType::Int64, true),
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![10, 10, 10])),
            Arc::new(Int64Array::from(vec![Some(2), Some(0), None])),
            Arc::new(StringArray::from(vec!["a", "b", "c"])),
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("numbers", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query(zero: ZeroDivision) -> GraphQuery {
    GraphQuery {
        version: 1,
        nodes: vec![
            QueryNode {
                id: "source".into(),
                source_text: "numerator and denominator".into(),
                operation: GraphOperation::Rows {
                    query: RowQuery {
                        version: 1,
                        input: RelationInput {
                            relation: "numbers".into(),
                            instance: "x".into(),
                        },
                        requirements: ["n", "d", "label"]
                            .into_iter()
                            .map(|name| Requirement {
                                id: name.into(),
                                source_text: name.into(),
                                operation: RowOperation::Project {
                                    field: FieldRef {
                                        instance: "x".into(),
                                        field: name.into(),
                                    },
                                    alias: name.into(),
                                },
                            })
                            .collect(),
                        unresolved: vec![],
                    },
                },
            },
            QueryNode {
                id: "ratio".into(),
                source_text: "n divided by d".into(),
                operation: GraphOperation::Calculate {
                    input: "source".into(),
                    passthrough: vec![],
                    ratios: vec![GraphRatio {
                        id: "ratio".into(),
                        numerator: "n".into(),
                        denominator: "d".into(),
                        required_unit: None,
                        zero,
                        alias: "ratio".into(),
                    }],
                },
            },
        ],
        root: "ratio".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    }
}

fn values(batches: &[RecordBatch]) -> Vec<Option<i128>> {
    let mut values = batches
        .iter()
        .flat_map(|batch| {
            let decimal = batch
                .column(0)
                .as_any()
                .downcast_ref::<datafusion::arrow::array::Decimal128Array>()
                .unwrap();
            (0..decimal.len())
                .map(|row| (!decimal.is_null(row)).then(|| decimal.value(row)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    values.sort();
    values
}

#[tokio::test]
async fn checked_ratio_call_keeps_exact_sql_direct_null_and_zero_behavior() {
    let engine = fixture();
    for (zero, expected) in [
        (
            ZeroDivision::Null,
            vec![None, None, Some(5_000_000_000_000_000_000)],
        ),
        (
            ZeroDivision::Zero,
            vec![None, Some(0), Some(5_000_000_000_000_000_000)],
        ),
    ] {
        let result = compile_graph(&engine, query(zero), CompileOptions::default()).await;
        let TypedOutcome::CompiledGraph { query: artifact } = result.outcome else {
            panic!("expected compiled graph: {:?}", result.outcome);
        };
        let direct = artifact
            .plan_direct(&engine)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        let sql = artifact
            .execute(&engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(values(&direct), expected);
        assert_eq!(values(&sql), expected);

        let statement = artifact.sql().statement();
        let function = &statement[statement.find("semantic_ratio_i64_v1").unwrap()..];
        let args = &function[..function.find(')').unwrap()];
        assert!(args.find("\"n\"").unwrap() < args.find("\"d\"").unwrap());
        assert!(
            args.find("\"d\"").unwrap()
                < args
                    .find(if zero == ZeroDivision::Zero {
                        "true"
                    } else {
                        "false"
                    })
                    .unwrap()
        );
    }
}

#[tokio::test]
async fn checked_ratio_call_rejects_wrong_operand_type_before_backend() {
    let engine = fixture();
    let mut wrong = query(ZeroDivision::Null);
    let GraphOperation::Calculate { ratios, .. } = &mut wrong.nodes[1].operation else {
        unreachable!()
    };
    ratios[0].denominator = "label".into();
    assert!(matches!(
        compile_graph(&engine, wrong, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_ratio_type"
    ));
}
