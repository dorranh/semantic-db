use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    ComparisonProfile, FieldSemantics, Relation, RelationSemantics, ValueMapping,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    Comparison, FieldRef, Literal, RelationInput, Requirement, RowOperation, RowPredicate, RowQuery,
};

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("exact", DataType::Utf8, false),
        Field::new("locale", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(StringArray::from(vec!["A", "B"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["A", "B"])) as ArrayRef,
        ],
    )
    .unwrap();
    let mut relation = Relation::base("labels", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        fields: [
            (
                "exact".into(),
                FieldSemantics {
                    comparison_profile: Some(ComparisonProfile::BinaryExact),
                    ..Default::default()
                },
            ),
            (
                "locale".into(),
                FieldSemantics {
                    comparison_profile: Some(ComparisonProfile::Locale {
                        tag: "de-CH".into(),
                    }),
                    ..Default::default()
                },
            ),
        ]
        .into(),
        value_mappings: [(
            "names".into(),
            ValueMapping {
                id: "dictionaries/names".into(),
                field: "locale".into(),
                description: "Exact authored spelling".into(),
                codes: [("first".into(), "A".into())].into(),
                enum_domain: None,
                source_refs: vec![],
            },
        )]
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

fn query(predicate: RowPredicate<FieldRef>) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "labels".into(),
            instance: "l".into(),
        },
        requirements: vec![
            Requirement {
                id: "filter".into(),
                source_text: "first label".into(),
                operation: RowOperation::Filter { predicate },
            },
            Requirement {
                id: "id".into(),
                source_text: "ID".into(),
                operation: RowOperation::Project {
                    field: FieldRef {
                        instance: "l".into(),
                        field: "id".into(),
                    },
                    alias: "id".into(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn field(name: &str) -> FieldRef {
    FieldRef {
        instance: "l".into(),
        field: name.into(),
    }
}

#[tokio::test]
async fn identical_utf8_physical_types_follow_authored_comparison_profile() {
    let engine = fixture();
    let exact = query(RowPredicate::Compare {
        field: field("exact"),
        operator: Comparison::Eq,
        value: Literal::Utf8("A".into()),
    });
    let result = compile_rows(&engine, exact, CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: compiled } = result.outcome else {
        panic!("binary exact comparison rejected: {:?}", result.outcome)
    };
    let direct = compiled
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = compiled
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    for batches in [&direct, &sql] {
        assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
        assert_eq!(array_value_to_string(batches[0].column(0), 0).unwrap(), "1");
    }
    for predicate in [
        RowPredicate::Compare {
            field: field("locale"),
            operator: Comparison::Eq,
            value: Literal::Utf8("A".into()),
        },
        RowPredicate::CompareMapped {
            field: field("locale"),
            operator: Comparison::Eq,
            mapping: "names".into(),
            phrase: "first".into(),
        },
    ] {
        let result = compile_rows(&engine, query(predicate), CompileOptions::default()).await;
        assert!(matches!(
            result.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "comparison_profile"
        ));
    }
}
