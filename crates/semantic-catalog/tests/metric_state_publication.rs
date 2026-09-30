use semantic_catalog::{
    Catalog, DataType, EmptyBehavior, Field, METRIC_STATE_VERSION, MetricDefinition,
    MetricStateContract, MetricStateKind, Presence, PublicationError, PublicationLimits, Relation,
    RelationSemantics, Schema,
};
use semantic_plan::typed::AggregateFunction;
use std::sync::Arc;

fn relation(state: Option<MetricStateContract>) -> Relation {
    let result_type = if state.as_ref().is_some_and(|state| {
        matches!(
            state.state,
            MetricStateKind::SumCountAverage | MetricStateKind::WeightedAverage { .. }
        )
    }) {
        DataType::Decimal128(38, 18)
    } else {
        DataType::Int64
    };
    let mut relation = Relation::base(
        "facts",
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("region", DataType::Utf8, false),
            Field::new("as_of", DataType::Date32, false),
            Field::new("amount", DataType::Int64, true),
            Field::new("weight", DataType::Int64, true),
        ])),
        "source:facts",
    );
    relation.semantics = Some(RelationSemantics {
        metrics: [(
            "amount_state".into(),
            MetricDefinition {
                id: "metrics/amount-state".into(),
                description: "Versioned amount state".into(),
                aliases: vec![],
                function: AggregateFunction::Sum,
                field: Some("amount".into()),
                distinct: false,
                source_grain: semantic_catalog::SourceGrain {
                    entity: None,
                    keys: vec![semantic_catalog::GrainKey {
                        relation: "facts".into(),
                        field: "id".into(),
                    }],
                },
                compatible_dimensions: ["region".into()].into(),
                compatible_lookup_dimensions: vec![],
                sum_rollup_dimensions: None,
                state,
                row_filters: vec![],
                result_type,
                unit: Presence::Value(semantic_catalog::Unit::Named {
                    id: "points".into(),
                }),
                temporal: Presence::Missing,
                empty_behavior: EmptyBehavior::Null,
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    relation
}
fn state(kind: MetricStateKind) -> MetricStateContract {
    MetricStateContract {
        version: METRIC_STATE_VERSION,
        state: kind,
        merge_dimensions: ["region".into()].into(),
    }
}
fn invalid_code(relation: Relation) -> &'static str {
    let catalog = Catalog::from_relations([relation]).unwrap();
    let Err(PublicationError::InvalidDefinition { code, .. }) =
        catalog.validate(&PublicationLimits::default())
    else {
        panic!("expected invalid publication")
    };
    code
}

#[test]
fn state_contract_is_published_with_revision_dependency() {
    let baseline = Catalog::from_relations([relation(None)]).unwrap();
    baseline.validate(&PublicationLimits::default()).unwrap();
    let with_state =
        Catalog::from_relations([relation(Some(state(MetricStateKind::SumCountAverage)))]).unwrap();
    with_state.validate(&PublicationLimits::default()).unwrap();
    assert_ne!(baseline.snapshot().id(), with_state.snapshot().id());
    let base_snapshot = baseline.snapshot();
    let changed_snapshot = with_state.snapshot();
    let base_metric = &base_snapshot
        .relation("facts")
        .unwrap()
        .definition()
        .semantics
        .as_ref()
        .unwrap()
        .metrics["amount_state"];
    let changed_metric = &changed_snapshot
        .relation("facts")
        .unwrap()
        .definition()
        .semantics
        .as_ref()
        .unwrap()
        .metrics["amount_state"];
    assert_ne!(
        base_metric.reference().revision,
        changed_metric.reference().revision
    );
}

#[test]
fn publication_rejects_invalid_state_fields_scope_version_and_types() {
    let mut bad = state(MetricStateKind::SumCountAverage);
    bad.version += 1;
    assert_eq!(invalid_code(relation(Some(bad))), "invalid_metric_state");
    let mut bad = state(MetricStateKind::SumCountAverage);
    bad.merge_dimensions.insert("as_of".into());
    assert_eq!(
        invalid_code(relation(Some(bad))),
        "invalid_metric_state_scope"
    );
    let mut bad = state(MetricStateKind::SumCountAverage);
    bad.merge_dimensions = ["missing".into()].into();
    assert_eq!(
        invalid_code(relation(Some(bad))),
        "invalid_metric_state_scope"
    );
    let bad = state(MetricStateKind::ExactDistinct {
        identity_fields: vec!["missing".into()],
    });
    assert_eq!(
        invalid_code(relation(Some(bad))),
        "missing_or_ambiguous_field"
    );
    let bad = state(MetricStateKind::SnapshotBalance {
        time_field: "amount".into(),
        tie_break_fields: vec!["id".into()],
    });
    assert_eq!(
        invalid_code(relation(Some(bad))),
        "invalid_metric_state_time_type"
    );
    let mut bad_type = relation(Some(state(MetricStateKind::WeightedAverage {
        weight_field: "weight".into(),
        zero: semantic_catalog::ZeroWeight::Null,
    })));
    bad_type
        .semantics
        .as_mut()
        .unwrap()
        .metrics
        .get_mut("amount_state")
        .unwrap()
        .result_type = DataType::Utf8;
    assert_eq!(invalid_code(bad_type), "invalid_metric_state_type");

    let valid =
        Catalog::from_relations([relation(Some(state(MetricStateKind::WeightedAverage {
            weight_field: "weight".into(),
            zero: semantic_catalog::ZeroWeight::Null,
        })))])
        .unwrap();
    valid.validate(&PublicationLimits::default()).unwrap();
    let missing = relation(Some(state(MetricStateKind::WeightedAverage {
        weight_field: "missing".into(),
        zero: semantic_catalog::ZeroWeight::Null,
    })));
    assert_eq!(invalid_code(missing), "missing_or_ambiguous_field");
    let invalid_weight = relation(Some(state(MetricStateKind::WeightedAverage {
        weight_field: "region".into(),
        zero: semantic_catalog::ZeroWeight::Null,
    })));
    assert_eq!(invalid_code(invalid_weight), "invalid_metric_state_type");
}

#[test]
fn exact_distinct_requires_one_int64_identity_and_count_profile() {
    let exact = || {
        let mut relation = relation(Some(state(MetricStateKind::ExactDistinct {
            identity_fields: vec!["id".into()],
        })));
        let metric = relation
            .semantics
            .as_mut()
            .unwrap()
            .metrics
            .get_mut("amount_state")
            .unwrap();
        metric.function = AggregateFunction::Count;
        metric.field = Some("id".into());
        metric.empty_behavior = EmptyBehavior::Zero;
        relation
    };
    Catalog::from_relations([exact()])
        .unwrap()
        .validate(&PublicationLimits::default())
        .unwrap();

    let mut wrong_function = exact();
    wrong_function
        .semantics
        .as_mut()
        .unwrap()
        .metrics
        .get_mut("amount_state")
        .unwrap()
        .function = AggregateFunction::Sum;
    assert_eq!(invalid_code(wrong_function), "invalid_metric_state_type");

    let mut two_identities = exact();
    two_identities
        .semantics
        .as_mut()
        .unwrap()
        .metrics
        .get_mut("amount_state")
        .unwrap()
        .state
        .as_mut()
        .unwrap()
        .state = MetricStateKind::ExactDistinct {
        identity_fields: vec!["id".into(), "amount".into()],
    };
    assert_eq!(invalid_code(two_identities), "invalid_metric_state_type");

    let mut text_identity = exact();
    let metric = text_identity
        .semantics
        .as_mut()
        .unwrap()
        .metrics
        .get_mut("amount_state")
        .unwrap();
    metric.field = Some("region".into());
    metric.state.as_mut().unwrap().state = MetricStateKind::ExactDistinct {
        identity_fields: vec!["region".into()],
    };
    assert_eq!(invalid_code(text_identity), "invalid_metric_state_type");
}
