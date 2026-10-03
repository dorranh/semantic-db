use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{ConceptDefinition, Relation, RelationSemantics};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

fn engine_with_concepts(concepts: BTreeMap<String, ConceptDefinition>) -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("state", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            Arc::new(StringArray::from(vec![Some("open"), Some("closed"), None])) as ArrayRef,
        ],
    )
    .unwrap();
    let table = MemTable::try_new(schema.clone(), vec![vec![batch]]).unwrap();
    let mut relation = Relation::base("tickets", schema, "source:tickets");
    relation.semantics = Some(RelationSemantics {
        concepts,
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine.register_table(relation, Arc::new(table)).unwrap();
    engine
}

fn engine_with_predicate(predicate: RowPredicate<String>) -> Engine {
    engine_with_concepts(
        [(
            "active".into(),
            ConceptDefinition {
                id: "concepts/active".into(),
                description: "Tickets still open".into(),
                aliases: vec!["live".into()],
                alternatives: vec![],
                predicate,
                source_refs: vec![],
            },
        )]
        .into(),
    )
}

fn engine() -> Engine {
    engine_with_predicate(RowPredicate::Compare {
        field: "state".into(),
        operator: Comparison::Eq,
        value: Literal::Utf8("open".into()),
    })
}

fn parameter_engine() -> Engine {
    engine_with_predicate(RowPredicate::CompareParameter {
        field: "state".into(),
        operator: Comparison::Eq,
        parameter: "desired_state".into(),
    })
}

fn query(concept: &str) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "tickets".into(),
            instance: "t".into(),
        },
        requirements: vec![
            Requirement {
                id: "concept".into(),
                source_text: concept.into(),
                operation: RowOperation::ConceptFilter {
                    concept: concept.into(),
                    arguments: Default::default(),
                },
            },
            Requirement {
                id: "id".into(),
                source_text: "id".into(),
                operation: RowOperation::Project {
                    field: FieldRef {
                        instance: "t".into(),
                        field: "id".into(),
                    },
                    alias: "id".into(),
                },
            },
        ],
        unresolved: vec![],
    }
}

#[tokio::test]
async fn authored_concept_filters_both_execution_paths_and_pins_definition() {
    let engine = engine();
    let result = compile_rows(&engine, query("live"), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome);
    };
    let direct = query
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let emitted = query
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    for batches in [direct, emitted] {
        let values: Vec<_> = batches
            .iter()
            .flat_map(|batch| {
                (0..batch.num_rows())
                    .map(|row| array_value_to_string(batch.column(0), row).unwrap())
            })
            .collect();
        assert_eq!(values, ["1"]);
    }
    let bound = serde_json::to_value(query.bound()).unwrap();
    assert!(
        bound["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| reference["id"] == "concepts/active")
    );
}

#[tokio::test]
async fn unknown_concept_fails_closed() {
    let result = compile_rows(&engine(), query("unlisted"), CompileOptions::default()).await;
    assert!(matches!(result.outcome, TypedOutcome::Rejected { .. }));
}

#[tokio::test]
async fn authored_parameterized_concept_substitutes_exact_typed_value_on_both_paths() {
    let engine = parameter_engine();
    for (state, expected_id) in [("open", "1"), ("closed", "2")] {
        let mut proposal = query("active");
        let RowOperation::ConceptFilter { arguments, .. } = &mut proposal.requirements[0].operation
        else {
            unreachable!()
        };
        arguments.insert("desired_state".into(), Literal::Utf8(state.into()));
        let result = compile_rows(&engine, proposal, CompileOptions::default()).await;
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("parameterized concept rejected: {:?}", result.outcome)
        };
        let definition = &serde_json::to_value(query.bound()).unwrap()["definitions"];
        assert!(
            definition
                .as_array()
                .unwrap()
                .iter()
                .any(|value| { value["id"] == "concepts/active" })
        );
        for batches in [
            query
                .plan_direct(&engine)
                .await
                .unwrap()
                .collect()
                .await
                .unwrap(),
            query
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .unwrap(),
        ] {
            let ids = batches
                .iter()
                .flat_map(|batch| {
                    (0..batch.num_rows())
                        .map(|row| array_value_to_string(batch.column(0), row).unwrap())
                })
                .collect::<Vec<_>>();
            assert_eq!(ids, [expected_id]);
        }
    }
}

#[tokio::test]
async fn parameterized_concept_rejects_missing_extra_and_wrong_typed_values() {
    let engine = parameter_engine();
    let missing = compile_rows(&engine, query("active"), CompileOptions::default()).await;
    assert!(matches!(
        missing.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "parameter_missing"
    ));
    for (name, value, code) in [
        ("desired_state", Literal::Int64(1), "comparison_type"),
        ("unknown", Literal::Utf8("open".into()), "parameter_missing"),
    ] {
        let mut proposal = query("active");
        let RowOperation::ConceptFilter { arguments, .. } = &mut proposal.requirements[0].operation
        else {
            unreachable!()
        };
        arguments.insert(name.into(), value);
        let result = compile_rows(&engine, proposal, CompileOptions::default()).await;
        assert!(matches!(
            result.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == code
        ));
    }
    let mut extra = query("active");
    let RowOperation::ConceptFilter { arguments, .. } = &mut extra.requirements[0].operation else {
        unreachable!()
    };
    arguments.insert("desired_state".into(), Literal::Utf8("open".into()));
    arguments.insert("unknown".into(), Literal::Utf8("closed".into()));
    let result = compile_rows(&engine, extra, CompileOptions::default()).await;
    assert!(matches!(
        result.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "parameter_extra"
    ));
}

fn alternatives_engine() -> Engine {
    let active = ConceptDefinition {
        id: "concepts/active".into(),
        description: "Open tickets".into(),
        aliases: vec!["current".into()],
        alternatives: vec!["closed".into()],
        predicate: RowPredicate::Compare {
            field: "state".into(),
            operator: Comparison::Eq,
            value: Literal::Utf8("open".into()),
        },
        source_refs: vec![],
    };
    let mut closed = active.clone();
    closed.id = "concepts/closed".into();
    closed.description = "Closed tickets".into();
    closed.aliases.clear();
    closed.alternatives.clear();
    closed.predicate = RowPredicate::Compare {
        field: "state".into(),
        operator: Comparison::Eq,
        value: Literal::Utf8("closed".into()),
    };
    // The author lists competing definitions, without asserting that their
    // predicates imply or equal one another.
    engine_with_concepts([("active".into(), active), ("closed".into(), closed)].into())
}

#[tokio::test]
async fn competing_concept_requires_an_explicit_choice_and_pins_it() {
    let engine = alternatives_engine();
    for phrase in ["active", "current"] {
        let result = compile_rows(&engine, query(phrase), CompileOptions::default()).await;
        assert!(matches!(
            result.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "ambiguous_concept"
        ));
    }
    for (choice, expected) in [("concepts/active", "1"), ("closed", "2")] {
        let result = compile_rows(&engine, query(choice), CompileOptions::default()).await;
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("explicit concept choice rejected: {:?}", result.outcome)
        };
        let bound = serde_json::to_value(query.bound()).unwrap();
        let selected = if choice == "closed" {
            "concepts/closed"
        } else {
            "concepts/active"
        };
        assert!(
            bound["definitions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reference| reference["id"] == selected)
        );
        for batches in [
            query
                .plan_direct(&engine)
                .await
                .unwrap()
                .collect()
                .await
                .unwrap(),
            query
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .unwrap(),
        ] {
            let ids = batches
                .iter()
                .flat_map(|batch| {
                    (0..batch.num_rows())
                        .map(|row| array_value_to_string(batch.column(0), row).unwrap())
                })
                .collect::<Vec<_>>();
            assert_eq!(ids, [expected]);
        }
    }
}
