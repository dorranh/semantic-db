use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use chrono::DateTime;
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Date32Array, Int64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    EmptyBehavior, GrainKey, MetricDefinition, MetricTemporalApplicability, Presence, Relation,
    RelationSemantics, SourceGrain,
};
use semantic_compiler::typed::{
    Calendar, CompileOptions, ContextOrigin, DiagnosticKind, DiagnosticStage, NextAction,
    ParameterDeclaration, PreparedReferenceContext, PreparedRows, PreparedType, Recoverability,
    TypedOutcome, compile_rows, diagnostic_details,
};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    AggregateFunction, CalendarPeriod, CalendarUnit, Comparison, FieldRef, Literal,
    MetricApplicability, RelationInput, Requirement, RowOperation, RowPredicate, RowQuery,
};

fn scoped_grain(relation: &str, field: &str) -> SourceGrain {
    SourceGrain {
        entity: None,
        keys: vec![GrainKey {
            relation: relation.into(),
            field: field.into(),
        }],
    }
}

fn items() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("score", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            Arc::new(Int64Array::from(vec![10, 20, 30])) as ArrayRef,
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("items", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn field(instance: &str, name: &str) -> FieldRef {
    FieldRef {
        instance: instance.into(),
        field: name.into(),
    }
}

fn filtered() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "items".into(),
            instance: "i".into(),
        },
        requirements: vec![
            Requirement {
                id: "minimum".into(),
                source_text: "score at least the bound minimum".into(),
                operation: RowOperation::Filter {
                    predicate: RowPredicate::CompareParameter {
                        field: field("i", "score"),
                        operator: Comparison::GtEq,
                        parameter: "minimum".into(),
                    },
                },
            },
            Requirement {
                id: "id".into(),
                source_text: "item IDs".into(),
                operation: RowOperation::Project {
                    field: field("i", "id"),
                    alias: "id".into(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn declaration(name: &str, value_type: PreparedType) -> ParameterDeclaration {
    ParameterDeclaration {
        name: name.into(),
        value_type,
    }
}

fn ids(batches: &[RecordBatch]) -> Vec<i64> {
    let mut ids = batches
        .iter()
        .flat_map(|batch| {
            let values = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            (0..values.len())
                .map(|row| {
                    assert!(!values.is_null(row));
                    values.value(row)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

#[tokio::test]
async fn unbound_node_cannot_compile_and_bound_values_have_sql_direct_parity() {
    let engine = items();
    let unbound = diagnostic_details("unbound_parameter");
    assert_eq!(unbound.stage, DiagnosticStage::Input);
    assert_eq!(unbound.kind, DiagnosticKind::InvalidProposal);
    assert_eq!(unbound.recoverability, Recoverability::ProposalRepair);
    let wrong_value = diagnostic_details("parameter_type");
    assert_eq!(wrong_value.recoverability, Recoverability::UserInput);
    assert_eq!(wrong_value.next_action, NextAction::AskUser);
    assert!(matches!(
        compile_rows(&engine, filtered(), CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "unbound_parameter"
    ));
    let prepared = PreparedRows::prepare(
        &engine,
        filtered(),
        vec![declaration("minimum", PreparedType::Int64)],
        None,
        CompileOptions::default(),
    )
    .unwrap();
    let result = prepared
        .bind_values(
            &engine,
            BTreeMap::from([("minimum".into(), Literal::Int64(20))]),
            None,
        )
        .await
        .unwrap();
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    assert_eq!(
        ids(&query
            .plan_direct(&engine)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap()),
        vec![2, 3]
    );
    assert_eq!(
        ids(&query
            .execute(&engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap()),
        vec![2, 3]
    );
    assert!(query.sql().statement().contains("$1"));
    assert!(!query.sql().statement().contains("20"));
}

#[tokio::test]
async fn declarations_values_snapshot_and_current_scope_fail_closed() {
    let mut engine = items();
    let mut repeated = filtered();
    let RowOperation::Filter { predicate } = &mut repeated.requirements[0].operation else {
        unreachable!()
    };
    *predicate = RowPredicate::All {
        predicates: vec![predicate.clone(), predicate.clone()],
    };
    // Reusing one declared value in multiple predicates is deliberate; each
    // occurrence receives the same exact typed value.
    assert!(
        PreparedRows::prepare(
            &engine,
            repeated,
            vec![declaration("minimum", PreparedType::Int64)],
            None,
            CompileOptions::default(),
        )
        .is_ok()
    );
    assert_eq!(
        PreparedRows::prepare(
            &engine,
            filtered(),
            vec![
                declaration("minimum", PreparedType::Int64),
                declaration("minimum", PreparedType::Int64)
            ],
            None,
            CompileOptions::default()
        )
        .err()
        .unwrap()
        .code,
        "parameter_contract"
    );
    assert_eq!(
        PreparedRows::prepare(
            &engine,
            filtered(),
            vec![declaration("unused", PreparedType::Int64)],
            None,
            CompileOptions::default()
        )
        .err()
        .unwrap()
        .code,
        "parameter_contract"
    );
    let allowed = BTreeSet::from(["items".to_owned()]);
    let mut options = CompileOptions::default();
    options.allowed_relations = Some(allowed.clone());
    let prepared = PreparedRows::prepare(
        &engine,
        filtered(),
        vec![declaration("minimum", PreparedType::Int64)],
        None,
        options,
    )
    .unwrap();
    assert_eq!(
        prepared
            .bind_values(&engine, BTreeMap::new(), Some(&allowed))
            .await
            .err()
            .unwrap()
            .code,
        "parameter_count"
    );
    assert_eq!(
        prepared
            .bind_values(
                &engine,
                BTreeMap::from([
                    ("minimum".into(), Literal::Int64(20)),
                    ("extra".into(), Literal::Int64(1)),
                ]),
                Some(&allowed),
            )
            .await
            .err()
            .unwrap()
            .code,
        "parameter_count"
    );
    assert_eq!(
        prepared
            .bind_values(
                &engine,
                BTreeMap::from([("minimum".into(), Literal::Utf8("20".into()))]),
                Some(&allowed)
            )
            .await
            .err()
            .unwrap()
            .code,
        "parameter_type"
    );
    assert_eq!(
        prepared
            .bind_values(
                &engine,
                BTreeMap::from([("minimum".into(), Literal::Int64(20))]),
                None
            )
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    let authorized = prepared
        .bind_values(
            &engine,
            BTreeMap::from([("minimum".into(), Literal::Int64(20))]),
            Some(&allowed),
        )
        .await
        .unwrap();
    let TypedOutcome::Compiled { query } = authorized.outcome else {
        panic!("{:?}", authorized.outcome)
    };
    assert_eq!(
        query.plan_direct(&engine).await.err().unwrap().code,
        "execution_scope"
    );
    assert_eq!(
        ids(&query
            .plan_direct_authorized(&engine, &allowed)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap()),
        vec![2, 3]
    );
    let revoked = BTreeSet::new();
    let narrowed = prepared
        .bind_values(
            &engine,
            BTreeMap::from([("minimum".into(), Literal::Int64(20))]),
            Some(&revoked),
        )
        .await
        .unwrap();
    assert!(!matches!(narrowed.outcome, TypedOutcome::Compiled { .. }));

    let other_schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
    let other_batch = RecordBatch::try_new(
        other_schema.clone(),
        vec![Arc::new(Int64Array::from(vec![1]))],
    )
    .unwrap();
    engine
        .register_table(
            Relation::base("other", other_schema.clone(), "memory"),
            Arc::new(MemTable::try_new(other_schema, vec![vec![other_batch]]).unwrap()),
        )
        .unwrap();
    assert_eq!(
        prepared
            .bind_values(
                &engine,
                BTreeMap::from([("minimum".into(), Literal::Int64(20))]),
                Some(&allowed)
            )
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
}

fn temporal_engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("day", DataType::Date32, false),
        Field::new("score", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            Arc::new(Date32Array::from(vec![19737, 19763, 19792])) as ArrayRef,
            Arc::new(Int64Array::from(vec![1, 2, 4])) as ArrayRef,
        ],
    )
    .unwrap();
    let mut relation = Relation::base("temporal_scores", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [(
            "monthly_score".into(),
            MetricDefinition {
                id: "metrics/monthly-score".into(),
                description: "Monthly scores in the first quarter of 2024".into(),
                aliases: vec![],
                function: AggregateFunction::Sum,
                field: Some("score".into()),
                distinct: false,
                source_grain: semantic_catalog::SourceGrain {
                    entity: None,
                    keys: vec![semantic_catalog::GrainKey {
                        relation: "temporal_scores".into(),
                        field: "id".into(),
                    }],
                },
                compatible_dimensions: Default::default(),
                compatible_lookup_dimensions: vec![],
                sum_rollup_dimensions: None,
                state: None,
                row_filters: vec![],
                result_type: DataType::Int64,
                unit: Presence::Value(semantic_catalog::Unit::Named {
                    id: "points".into(),
                }),
                temporal: Presence::Value(MetricTemporalApplicability {
                    field: "day".into(),
                    grain: CalendarUnit::Month,
                    coverage_start: Literal::Date32(19723),
                    coverage_end: Literal::Date32(19814),
                }),
                empty_behavior: EmptyBehavior::Null,
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

#[tokio::test]
async fn prepared_civil_dates_bind_to_date32_and_reject_strings_or_invalid_dates() {
    let engine = temporal_engine();
    let mut proposal = filtered();
    proposal.input.relation = "temporal_scores".into();
    proposal.requirements[0].operation = RowOperation::Filter {
        predicate: RowPredicate::CompareParameter {
            field: field("i", "day"),
            operator: Comparison::LtEq,
            parameter: "minimum".into(),
        },
    };
    let prepared = PreparedRows::prepare(
        &engine,
        proposal,
        vec![declaration("minimum", PreparedType::Date32)],
        None,
        CompileOptions::default(),
    )
    .unwrap();
    for value in [
        Literal::Date32(19763),
        Literal::GregorianDate("2024-02-10".into()),
    ] {
        let result = prepared
            .bind_values(&engine, BTreeMap::from([("minimum".into(), value)]), None)
            .await
            .unwrap();
        let TypedOutcome::Compiled { query } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
        assert_eq!(query.sql().parameters(), &[Literal::Date32(19763)]);
        assert_eq!(
            ids(&query
                .plan_direct(&engine)
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()),
            vec![1, 2]
        );
        assert_eq!(
            ids(&query
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()),
            vec![1, 2]
        );
    }
    assert_eq!(
        prepared
            .bind_values(
                &engine,
                BTreeMap::from([("minimum".into(), Literal::Utf8("2024-02-10".into()))]),
                None
            )
            .await
            .unwrap_err()
            .code,
        "parameter_type"
    );
    let invalid = prepared
        .bind_values(
            &engine,
            BTreeMap::from([(
                "minimum".into(),
                Literal::GregorianDate("2024-02-30".into()),
            )]),
            None,
        )
        .await
        .unwrap();
    assert!(
        matches!(invalid.outcome,TypedOutcome::Rejected { diagnostic } if diagnostic.code == "date_literal")
    );
}

fn temporal_query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "temporal_scores".into(),
            instance: "r".into(),
        },
        requirements: vec![
            Requirement {
                id: "period".into(),
                source_text: "previous month".into(),
                operation: RowOperation::CalendarFilter {
                    field: field("r", "day"),
                    period: CalendarPeriod {
                        unit: CalendarUnit::Month,
                        offset: -1,
                        count: 1,
                    },
                },
            },
            Requirement {
                id: "metric".into(),
                source_text: "monthly score".into(),
                operation: RowOperation::Metric {
                    name: "monthly_score".into(),
                    alias: "score".into(),
                    applicability: MetricApplicability {
                        required_unit: Some(semantic_catalog::Unit::Named {
                            id: "points".into(),
                        }),
                        required_source_grain: Some(scoped_grain("temporal_scores", "id")),
                    },
                },
            },
        ],
        unresolved: vec![],
    }
}

fn millis(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_millis()
}

#[tokio::test]
async fn reference_parameter_rechecks_metric_date_coverage_for_each_binding() {
    let engine = temporal_engine();
    let prepared = PreparedRows::prepare(
        &engine,
        temporal_query(),
        vec![declaration("as_of", PreparedType::Int64)],
        Some(PreparedReferenceContext {
            instant_parameter: "as_of".into(),
            timezone: "UTC".into(),
            calendar: Calendar::Gregorian,
            origin: ContextOrigin::Caller,
        }),
        CompileOptions::default(),
    )
    .unwrap();
    let inside = prepared
        .bind_values(
            &engine,
            BTreeMap::from([(
                "as_of".into(),
                Literal::Int64(millis("2024-03-15T12:00:00Z")),
            )]),
            None,
        )
        .await
        .unwrap();
    let TypedOutcome::Compiled { query } = inside.outcome else {
        panic!("{:?}", inside.outcome)
    };
    assert_eq!(
        ids(&query
            .plan_direct(&engine)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap()),
        vec![2]
    );
    assert_eq!(
        ids(&query
            .execute(&engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap()),
        vec![2]
    );
    let outside = prepared
        .bind_values(
            &engine,
            BTreeMap::from([(
                "as_of".into(),
                Literal::Int64(millis("2024-05-15T12:00:00Z")),
            )]),
            None,
        )
        .await
        .unwrap();
    assert!(
        matches!(outside.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_coverage")
    );
}
