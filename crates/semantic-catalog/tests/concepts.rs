use std::sync::Arc;

use semantic_catalog::{
    Catalog, ConceptDefinition, DataType, Field, PublicationError, PublicationLimits, Relation,
    RelationSemantics, Schema,
};
use semantic_plan::typed::{Comparison, Literal, RowPredicate};

fn catalog(value: &str) -> Catalog {
    let mut relation = Relation::base(
        "tickets",
        Arc::new(Schema::new(vec![Field::new(
            "state",
            DataType::Utf8,
            false,
        )])),
        "source:tickets",
    );
    relation.semantics = Some(RelationSemantics {
        concepts: [(
            "active".into(),
            ConceptDefinition {
                id: "concepts/active".into(),
                description: "Active".into(),
                aliases: vec![],
                alternatives: vec![],
                predicate: RowPredicate::Compare {
                    field: "state".into(),
                    operator: Comparison::Eq,
                    value: Literal::Utf8(value.into()),
                },
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    Catalog::from_relations([relation]).unwrap()
}

#[test]
fn concept_placeholders_have_bounded_exact_authored_types() {
    let mut relation = catalog("open").relation("tickets").unwrap().clone();
    relation.schema = Arc::new(Schema::new(vec![
        Field::new("state", DataType::Utf8, false),
        Field::new("id", DataType::Int64, false),
    ]));
    let concept = relation
        .semantics
        .as_mut()
        .unwrap()
        .concepts
        .get_mut("active")
        .unwrap();
    concept.predicate = RowPredicate::CompareParameter {
        field: "state".into(),
        operator: Comparison::Eq,
        parameter: "requested_state".into(),
    };
    Catalog::from_relations([relation.clone()])
        .unwrap()
        .validate(&PublicationLimits::default())
        .unwrap();

    concept_parameter(&mut relation).predicate = RowPredicate::CompareParameter {
        field: "state".into(),
        operator: Comparison::Eq,
        parameter: "bad-name".into(),
    };
    assert_invalid_parameter(relation.clone());

    concept_parameter(&mut relation).predicate = RowPredicate::All {
        predicates: vec![
            RowPredicate::CompareParameter {
                field: "state".into(),
                operator: Comparison::Eq,
                parameter: "value".into(),
            },
            RowPredicate::CompareParameter {
                field: "id".into(),
                operator: Comparison::Eq,
                parameter: "value".into(),
            },
        ],
    };
    assert_invalid_parameter(relation);
}

#[test]
fn civil_date_concept_parameters_remain_exact_and_reject_mixed_field_types() {
    let mut relation = catalog("open").relation("tickets").unwrap().clone();
    relation.schema = Arc::new(Schema::new(vec![
        Field::new("day", DataType::Date32, false),
        Field::new("id", DataType::Int64, false),
    ]));
    concept_parameter(&mut relation).predicate = RowPredicate::CompareParameter {
        field: "day".into(),
        operator: Comparison::LtEq,
        parameter: "as_of_date".into(),
    };
    Catalog::from_relations([relation.clone()])
        .unwrap()
        .validate(&PublicationLimits::default())
        .unwrap();
    concept_parameter(&mut relation).predicate = RowPredicate::All {
        predicates: vec![
            RowPredicate::CompareParameter {
                field: "day".into(),
                operator: Comparison::LtEq,
                parameter: "as_of_date".into(),
            },
            RowPredicate::CompareParameter {
                field: "id".into(),
                operator: Comparison::Eq,
                parameter: "as_of_date".into(),
            },
        ],
    };
    assert_invalid_parameter(relation);
}

fn concept_parameter(relation: &mut Relation) -> &mut ConceptDefinition {
    relation
        .semantics
        .as_mut()
        .unwrap()
        .concepts
        .get_mut("active")
        .unwrap()
}

fn assert_invalid_parameter(relation: Relation) {
    assert!(matches!(
        Catalog::from_relations([relation])
            .unwrap()
            .validate(&PublicationLimits::default()),
        Err(PublicationError::InvalidDefinition {
            code: "invalid_concept_parameter",
            ..
        })
    ));
}

#[test]
fn competing_concepts_require_existing_distinct_relation_scoped_references() {
    let before = catalog("open");
    let mut relation = before.relation("tickets").unwrap().clone();
    let semantics = relation.semantics.as_mut().unwrap();
    let mut pending = semantics.concepts["active"].clone();
    pending.id = "concepts/pending".into();
    pending.predicate = RowPredicate::Compare {
        field: "state".into(),
        operator: Comparison::Eq,
        value: Literal::Utf8("pending".into()),
    };
    semantics.concepts.insert("pending".into(), pending);
    semantics.concepts.get_mut("active").unwrap().alternatives = vec!["pending".into()];
    let published = Catalog::from_relations([relation.clone()]).unwrap();
    published.validate(&PublicationLimits::default()).unwrap();
    let reference = |catalog: &Catalog| {
        catalog
            .snapshot()
            .relation("tickets")
            .unwrap()
            .definition_reference("concept", "active")
            .unwrap()
            .clone()
    };
    assert_ne!(reference(&before), reference(&published));
    assert_ne!(before.snapshot().id(), published.snapshot().id());

    for alternatives in [
        vec!["missing".into()],
        vec!["active".into()],
        vec!["pending".into(), "pending".into()],
    ] {
        relation
            .semantics
            .as_mut()
            .unwrap()
            .concepts
            .get_mut("active")
            .unwrap()
            .alternatives = alternatives;
        assert!(matches!(
            Catalog::from_relations([relation.clone()])
                .unwrap()
                .validate(&PublicationLimits::default()),
            Err(PublicationError::InvalidDefinition {
                code: "invalid_concept_alternatives",
                ..
            })
        ));
    }
}

#[test]
fn concept_predicate_is_validated_and_revisioned() {
    let first = catalog("open");
    first.validate(&PublicationLimits::default()).unwrap();
    let second = catalog("pending");
    second.validate(&PublicationLimits::default()).unwrap();
    let a = first.snapshot();
    let b = second.snapshot();
    assert_ne!(
        a.relation("tickets")
            .unwrap()
            .definition_reference("concept", "active"),
        b.relation("tickets")
            .unwrap()
            .definition_reference("concept", "active")
    );
    let mut invalid = catalog("open");
    let relation = invalid.relation("tickets").unwrap().clone();
    let mut relation = relation;
    let concept = relation
        .semantics
        .as_mut()
        .unwrap()
        .concepts
        .get_mut("active")
        .unwrap();
    concept.predicate = RowPredicate::IsNull {
        field: "missing".into(),
        negated: false,
    };
    invalid = Catalog::from_relations([relation]).unwrap();
    assert!(invalid.validate(&PublicationLimits::default()).is_err());
}
