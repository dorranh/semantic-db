//! Release-gate fixtures for the deliberately narrow compiler MVP.
//!
//! Expected rows and diagnostic codes are checked in separately from query
//! construction. The single model-provider case uses scripted output and tests
//! orchestration only; it is not evidence of live-model interpretation quality.

use std::sync::{Arc, Mutex};

use datafusion::{
    arrow::{
        array::{
            ArrayRef, BooleanArray, Date32Array, Int64Array, StringArray, TimestampMillisecondArray,
        },
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    EmptyBehavior, FactResolution, GovernedFilter, MetricDefinition, MetricTemporalApplicability,
    Presence, Relation, RelationSemantics, RelationshipDefinition, RelationshipKey, RowPolicy,
};
use semantic_compiler::{
    Compiler,
    provider::{Message, ModelProvider, ProviderError},
    typed::{
        Calendar, CompileOptions, CompiledGraph, CompiledQuery, ContextOrigin, RequestContext,
        SelectionMode, TypedCompilation, TypedOutcome, compile_graph, compile_rows,
    },
};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::{graph::*, typed::*};
use serde_json::{Value, json};

const ACCEPTANCE: &str = include_str!("fixtures/mvp_acceptance.json");

fn manifest() -> Value {
    serde_json::from_str(ACCEPTANCE).expect("valid acceptance fixture manifest")
}

fn expected_rows(case: &str) -> Vec<Vec<String>> {
    serde_json::from_value(manifest()["cases"][case]["rows"].clone())
        .unwrap_or_else(|error| panic!("missing rows for {case}: {error}"))
}

fn expected_diagnostic(case: &str) -> String {
    manifest()["cases"][case]["diagnostic"]
        .as_str()
        .unwrap_or_else(|| panic!("missing diagnostic for {case}"))
        .to_owned()
}

fn field(instance: &str, name: &str) -> FieldRef {
    FieldRef {
        instance: instance.into(),
        field: name.into(),
    }
}

fn requirement(id: &str, operation: RowOperation) -> Requirement {
    Requirement {
        id: id.into(),
        source_text: id.into(),
        operation,
    }
}

fn base_query(relation: &str) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: relation.into(),
            instance: "r".into(),
        },
        requirements: vec![],
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| {
                (0..batch.num_columns())
                    .map(|column| array_value_to_string(batch.column(column), row).unwrap())
                    .collect()
            })
        })
        .collect()
}

fn compiled_row(result: TypedCompilation) -> Box<CompiledQuery> {
    match result.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("expected compiled row query, got {other:?}"),
    }
}

fn compiled_graph(result: TypedCompilation) -> Box<CompiledGraph> {
    match result.outcome {
        TypedOutcome::CompiledGraph { query } => query,
        other => panic!("expected compiled graph query, got {other:?}"),
    }
}

async fn assert_row_paths(engine: &Engine, query: RowQuery, case: &str) {
    let artifact = compiled_row(compile_rows(engine, query, CompileOptions::default()).await);
    let direct = artifact
        .plan_direct(engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = artifact
        .execute(engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = expected_rows(case);
    assert_eq!(rows(&direct), expected, "direct plan: {case}");
    assert_eq!(rows(&sql), expected, "generated SQL: {case}");
}

async fn assert_graph_paths(engine: &Engine, query: GraphQuery, case: &str) {
    let artifact = compiled_graph(compile_graph(engine, query, CompileOptions::default()).await);
    let direct = artifact
        .plan_direct(engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = artifact
        .execute(engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = expected_rows(case);
    assert_eq!(rows(&direct), expected, "direct graph: {case}");
    assert_eq!(rows(&sql), expected, "generated graph SQL: {case}");
}

fn register(engine: &mut Engine, relation: Relation, columns: Vec<ArrayRef>) {
    let schema = relation.schema.clone();
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
}

fn governed_fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, true),
        Field::new("active", DataType::Boolean, true),
        Field::new("score", DataType::Int64, true),
    ]));
    let mut relation = Relation::base("governed", schema, "private:acceptance");
    relation.semantics = Some(RelationSemantics {
        metrics: [(
            "qualified_score".into(),
            MetricDefinition {
                id: "metrics/qualified-score".into(),
                description: "Scores of active visible items".into(),
                aliases: vec![],
                function: AggregateFunction::Sum,
                field: Some("score".into()),
                distinct: false,
                source_grain: vec!["id".into()],
                compatible_dimensions: ["label".into()].into(),
                compatible_lookup_dimensions: vec![],
                sum_rollup_dimensions: None,
                row_filters: vec![GovernedFilter {
                    field: "active".into(),
                    operator: Comparison::Eq,
                    value: Literal::Boolean(true),
                }],
                result_type: DataType::Int64,
                unit: Presence::Value("points".into()),
                temporal: Presence::Missing,
                empty_behavior: EmptyBehavior::Null,
                source_refs: vec![],
            },
        )]
        .into(),
        row_policies: vec![RowPolicy {
            id: "policies/id-scope".into(),
            filters: vec![GovernedFilter {
                field: "id".into(),
                operator: Comparison::LtEq,
                value: Literal::Int64(4),
            }],
            source_refs: vec![],
        }],
        ..Default::default()
    });
    let mut engine = Engine::new();
    register(
        &mut engine,
        relation,
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])),
            Arc::new(StringArray::from(vec![
                Some("a"),
                Some("b"),
                None,
                Some("a"),
                Some("c"),
            ])),
            Arc::new(BooleanArray::from(vec![
                Some(true),
                Some(false),
                None,
                Some(true),
                Some(true),
            ])),
            Arc::new(Int64Array::from(vec![
                Some(10),
                Some(30),
                Some(20),
                None,
                Some(10),
            ])),
        ],
    );
    engine
}

#[tokio::test]
async fn governed_metric_and_row_policy_match_independent_expected_rows() {
    let engine = governed_fixture();
    let mut query = base_query("governed");
    query.requirements = vec![
        requirement(
            "qualified",
            RowOperation::Metric {
                name: "qualified_score".into(),
                alias: "qualified".into(),
                applicability: MetricApplicability {
                    required_unit: Some("points".into()),
                    required_source_grain: vec!["id".into()],
                    ..Default::default()
                },
            },
        ),
        requirement(
            "all-scores",
            RowOperation::Aggregate {
                function: AggregateFunction::Sum,
                field: Some(field("r", "score")),
                distinct: false,
                alias: "all".into(),
            },
        ),
        requirement(
            "visible-count",
            RowOperation::Aggregate {
                function: AggregateFunction::Count,
                field: None,
                distinct: false,
                alias: "count".into(),
            },
        ),
    ];
    assert_row_paths(&engine, query, "governed_metric_policy").await;
}

fn lookup_fixture(duplicate_customer: bool) -> Engine {
    let item_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("bill", DataType::Int64, true),
        Field::new("ship", DataType::Int64, true),
    ]));
    let mut items = Relation::base("items", item_schema, "memory");
    items.semantics = Some(RelationSemantics {
        relationships: ["bill", "ship"]
            .into_iter()
            .map(|role| {
                (
                    role.into(),
                    RelationshipDefinition {
                        ai_context: None,
                        id: format!("relationships/{role}"),
                        right_relation: "customers".into(),
                        role: role.into(),
                        key_pairs: vec![RelationshipKey {
                            left_field: role.into(),
                            right_field: "id".into(),
                        }],
                        null_keys_match: false,
                        cardinality: FactResolution::Unknown,
                        source_refs: vec![],
                    },
                )
            })
            .collect(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    register(
        &mut engine,
        items,
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Int64Array::from(vec![Some(10), Some(20), None])),
            Arc::new(Int64Array::from(vec![Some(20), Some(10), Some(99)])),
        ],
    );
    let customer_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("region", DataType::Utf8, true),
    ]));
    let (ids, regions) = if duplicate_customer {
        (
            vec![Some(10), Some(20), Some(10)],
            vec![Some("CH"), Some("GB"), Some("DE")],
        )
    } else {
        (vec![Some(10), Some(20)], vec![Some("CH"), Some("GB")])
    };
    register(
        &mut engine,
        Relation::base("customers", customer_schema, "memory"),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(regions)),
        ],
    );
    engine
}

fn lookup_query() -> RowQuery {
    let mut query = base_query("items");
    query.requirements.push(requirement(
        "id",
        RowOperation::Project {
            field: field("r", "id"),
            alias: "id".into(),
        },
    ));
    for role in ["bill", "ship"] {
        query.requirements.push(requirement(
            role,
            RowOperation::Lookup {
                usage: LookupUsage::Project,
                relationship: role.into(),
                role: role.into(),
                instance: format!("customer_{role}"),
                field: "region".into(),
                alias: format!("{role}_region"),
                missing: MissingMatch::Null,
            },
        ));
    }
    query.requirements.push(requirement(
        "order",
        RowOperation::Order {
            field: field("r", "id"),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        },
    ));
    query
}

#[tokio::test]
async fn lookup_roles_are_distinct_and_duplicate_dimension_keys_fail_closed() {
    assert_row_paths(
        &lookup_fixture(false),
        lookup_query(),
        "billing_shipping_roles",
    )
    .await;

    let engine = lookup_fixture(true);
    let artifact =
        compiled_row(compile_rows(&engine, lookup_query(), CompileOptions::default()).await);
    let direct = artifact
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap_err();
    let sql = artifact
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap_err();
    assert!(direct.to_string().contains("uniqueness obligation failed"));
    assert!(sql.to_string().contains("uniqueness obligation failed"));
}

fn label_fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, true),
        Field::new("score", DataType::Int64, true),
    ]));
    let mut engine = Engine::new();
    register(
        &mut engine,
        Relation::base("items", schema, "memory"),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])),
            Arc::new(StringArray::from(vec![
                Some("O'Reilly"),
                Some("b"),
                None,
                Some("O'Reilly"),
                Some("B"),
            ])),
            Arc::new(Int64Array::from(vec![
                Some(10),
                Some(30),
                Some(20),
                None,
                Some(10),
            ])),
        ],
    );
    engine
}

fn compare_id(operator: Comparison, value: i64) -> RowPredicate {
    RowPredicate::Compare {
        field: field("r", "id"),
        operator,
        value: Literal::Int64(value),
    }
}

fn set_query(operator: SetOperator) -> GraphQuery {
    let leaf = |id: &str, predicate: RowPredicate| {
        let mut query = base_query("items");
        query.requirements = vec![
            requirement(
                "label",
                RowOperation::Project {
                    field: field("r", "label"),
                    alias: format!("{id}_label"),
                },
            ),
            requirement("subset", RowOperation::Filter { predicate }),
        ];
        QueryNode {
            id: id.into(),
            source_text: format!("{id} subset"),
            operation: GraphOperation::Rows { query },
        }
    };
    GraphQuery {
        version: 1,
        nodes: vec![
            QueryNode {
                id: "set".into(),
                source_text: "bag set operation".into(),
                operation: GraphOperation::Set {
                    left: "left".into(),
                    right: "right".into(),
                    operator,
                    duplicates: Duplicates::All,
                    columns: vec![SetColumn {
                        id: "value".into(),
                        left: "label".into(),
                        right: "label".into(),
                        alias: "label".into(),
                    }],
                },
            },
            leaf("left", compare_id(Comparison::LtEq, 4)),
            leaf("right", compare_id(Comparison::GtEq, 3)),
        ],
        root: "set".into(),
        ordering: vec![GraphOrder {
            slot: "value".into(),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        }],
        limit: None,
        unresolved: vec![],
    }
}

#[tokio::test]
async fn bag_set_operations_preserve_duplicate_and_null_multiplicity() {
    let engine = label_fixture();
    assert_graph_paths(
        &engine,
        set_query(SetOperator::Intersect),
        "set_intersect_all",
    )
    .await;
    assert_graph_paths(&engine, set_query(SetOperator::Except), "set_except_all").await;
}

fn composition_fixture(composite: bool) -> Engine {
    let mut engine = Engine::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, true),
        Field::new("category", DataType::Utf8, true),
        Field::new("amount", DataType::Int64, true),
    ]));
    let mut sales = Relation::base("sales", schema.clone(), "memory");
    let key_pairs = if composite {
        vec![
            RelationshipKey {
                left_field: "region".into(),
                right_field: "region".into(),
            },
            RelationshipKey {
                left_field: "category".into(),
                right_field: "category".into(),
            },
        ]
    } else {
        vec![RelationshipKey {
            left_field: "region".into(),
            right_field: "region".into(),
        }]
    };
    sales.semantics = Some(RelationSemantics {
        relationships: [(
            "costs".into(),
            RelationshipDefinition {
                ai_context: None,
                id: "relationships/sales-costs".into(),
                right_relation: "costs".into(),
                role: "same_group".into(),
                key_pairs,
                null_keys_match: false,
                cardinality: FactResolution::Unknown,
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    let (
        sales_regions,
        sales_categories,
        sales_amounts,
        cost_regions,
        cost_categories,
        cost_amounts,
    ) = if composite {
        (
            vec![Some("A"), Some("A"), Some("A")],
            vec![Some("x"), Some("x"), Some("y")],
            vec![Some(1), Some(2), Some(3)],
            vec![Some("A"), Some("B")],
            vec![Some("x"), Some("x")],
            vec![Some(10), Some(20)],
        )
    } else {
        (
            vec![Some("A"), Some("A"), Some("B"), Some("C"), None],
            vec![Some("x"), Some("x"), Some("x"), Some("y"), Some("z")],
            vec![Some(5), Some(7), None, Some(3), Some(4)],
            vec![Some("A"), Some("A"), Some("D"), None],
            vec![Some("x"), Some("x"), Some("x"), Some("z")],
            vec![Some(2), Some(4), Some(8), Some(9)],
        )
    };
    register(
        &mut engine,
        sales,
        vec![
            Arc::new(StringArray::from(sales_regions)),
            Arc::new(StringArray::from(sales_categories)),
            Arc::new(Int64Array::from(sales_amounts)),
        ],
    );
    register(
        &mut engine,
        Relation::base("costs", schema, "memory"),
        vec![
            Arc::new(StringArray::from(cost_regions)),
            Arc::new(StringArray::from(cost_categories)),
            Arc::new(Int64Array::from(cost_amounts)),
        ],
    );
    engine
}

fn aggregate_leaf(relation: &str, groups: &[&str]) -> QueryNode {
    let mut query = base_query(relation);
    for group in groups {
        query.requirements.push(requirement(
            group,
            RowOperation::Group {
                field: field("r", group),
                alias: (*group).into(),
            },
        ));
    }
    query.requirements.push(requirement(
        "amount",
        RowOperation::Aggregate {
            function: AggregateFunction::Sum,
            field: Some(field("r", "amount")),
            distinct: false,
            alias: "amount".into(),
        },
    ));
    QueryNode {
        id: relation.into(),
        source_text: format!("{relation} independently aggregated"),
        operation: GraphOperation::Rows { query },
    }
}

fn composition_query(groups: &[&str], scalar: bool) -> GraphQuery {
    let keys = groups
        .iter()
        .map(|group| SetColumn {
            id: (*group).into(),
            left: (*group).into(),
            right: (*group).into(),
            alias: (*group).into(),
        })
        .collect();
    let relationship_value = if scalar { "" } else { "costs" };
    let role = if scalar { "" } else { "same_group" };
    let mut ordering = groups
        .iter()
        .map(|group| GraphOrder {
            slot: (*group).into(),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        })
        .collect::<Vec<_>>();
    if groups.len() == 1 {
        ordering.push(GraphOrder {
            slot: "sales_total".into(),
            direction: Direction::Desc,
            nulls: NullOrder::Last,
        });
    }
    GraphQuery {
        version: 1,
        nodes: vec![
            aggregate_leaf("sales", groups),
            aggregate_leaf("costs", groups),
            QueryNode {
                id: "composition".into(),
                source_text: "align independently aggregated facts".into(),
                operation: GraphOperation::Compose {
                    left: "sales".into(),
                    right: "costs".into(),
                    relationship_relation: if scalar { "".into() } else { "sales".into() },
                    relationship: relationship_value.into(),
                    role: role.into(),
                    domain: GroupDomain::Union,
                    null_alignment: NullAlignment::Match,
                    keys,
                    outputs: vec![
                        CompositionOutput {
                            id: "sales_total".into(),
                            side: Side::Left,
                            slot: "amount".into(),
                            alias: "sales".into(),
                            missing: MissingGroup::Zero,
                        },
                        CompositionOutput {
                            id: "cost_total".into(),
                            side: Side::Right,
                            slot: "amount".into(),
                            alias: "costs".into(),
                            missing: MissingGroup::Zero,
                        },
                    ],
                },
            },
        ],
        root: "composition".into(),
        ordering,
        limit: None,
        unresolved: vec![],
    }
}

#[tokio::test]
async fn composition_covers_missing_null_composite_key_and_global_scalar_domains() {
    assert_graph_paths(
        &composition_fixture(false),
        composition_query(&["region"], false),
        "composition_missing_and_null",
    )
    .await;
    let composite = composition_fixture(true);
    assert_graph_paths(
        &composite,
        composition_query(&["region", "category"], false),
        "composition_composite_key",
    )
    .await;
    assert_graph_paths(
        &composite,
        composition_query(&[], true),
        "composition_global_scalar",
    )
    .await;
}

fn output_compare(slot: &str, operator: Comparison, value: Literal) -> OutputPredicate {
    RowPredicate::Compare {
        field: OutputRef { slot: slot.into() },
        operator,
        value,
    }
}

#[tokio::test]
async fn aggregate_and_window_filters_execute_at_the_declared_stage() {
    let mut query = base_query("items");
    query.requirements = vec![
        requirement(
            "group",
            RowOperation::Group {
                field: field("r", "label"),
                alias: "group".into(),
            },
        ),
        requirement(
            "sum",
            RowOperation::Aggregate {
                function: AggregateFunction::Sum,
                field: Some(field("r", "score")),
                distinct: false,
                alias: "sum".into(),
            },
        ),
        requirement(
            "having",
            RowOperation::FilterOutput {
                stage: OutputFilterStage::AfterAggregate,
                predicate: output_compare("sum", Comparison::Lt, Literal::Int64(30)),
            },
        ),
        requirement(
            "rank",
            RowOperation::Window {
                window: WindowSpec {
                    function: WindowFunction::Rank,
                    input: None,
                    partition_by: vec![],
                    order_by: vec![WindowOrder {
                        input: WindowInput::Output { slot: "sum".into() },
                        direction: Direction::Desc,
                        nulls: NullOrder::Last,
                    }],
                    frame: WindowFrame::ThroughCurrentPeer,
                },
                alias: "rank".into(),
            },
        ),
        requirement(
            "qualify",
            RowOperation::FilterOutput {
                stage: OutputFilterStage::AfterWindow,
                predicate: output_compare("rank", Comparison::LtEq, Literal::UInt64(1)),
            },
        ),
    ];
    assert_row_paths(&label_fixture(), query, "aggregate_window_stage").await;
}

#[tokio::test]
async fn structurally_ambiguous_output_aliases_reject_instead_of_being_ranked() {
    let mut query = base_query("items");
    query.requirements = vec![
        requirement(
            "id",
            RowOperation::Project {
                field: field("r", "id"),
                alias: "value".into(),
            },
        ),
        requirement(
            "label",
            RowOperation::Project {
                field: field("r", "label"),
                alias: "value".into(),
            },
        ),
    ];
    let result = compile_rows(&label_fixture(), query, CompileOptions::default()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == expected_diagnostic("ambiguous_output"))
    );
}

fn competing_metric_fixture(second_unit: &str) -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("score", DataType::Int64, false),
    ]));
    let metric = |id: &str, unit: &str| MetricDefinition {
        id: format!("metrics/{id}"),
        description: format!("Competing {id} definition"),
        aliases: vec!["qualified points".into()],
        function: AggregateFunction::Sum,
        field: Some("score".into()),
        distinct: false,
        source_grain: vec!["id".into()],
        compatible_dimensions: Default::default(),
        compatible_lookup_dimensions: vec![],
        sum_rollup_dimensions: None,
        row_filters: vec![],
        result_type: DataType::Int64,
        unit: Presence::Value(unit.into()),
        temporal: Presence::Missing,
        empty_behavior: EmptyBehavior::Null,
        source_refs: vec![],
    };
    let mut relation = Relation::base("scores", schema, "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [
            ("first_metric".into(), metric("first", "points")),
            ("second_metric".into(), metric("second", second_unit)),
        ]
        .into(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    register(
        &mut engine,
        relation,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(Int64Array::from(vec![1, 2])),
        ],
    );
    engine
}

fn competing_metric_query() -> RowQuery {
    let mut query = base_query("scores");
    let mut metric = requirement(
        "metric",
        RowOperation::Metric {
            name: "first_metric".into(),
            alias: "value".into(),
            applicability: MetricApplicability {
                required_unit: Some("points".into()),
                required_source_grain: vec!["id".into()],
                ..Default::default()
            },
        },
    );
    metric.source_text = "qualified points".into();
    query.requirements.push(metric);
    query
}

#[tokio::test]
async fn applicability_resolves_competing_metrics_and_catalog_mutation_restores_ambiguity() {
    assert_row_paths(
        &competing_metric_fixture("rows"),
        competing_metric_query(),
        "competing_metric_resolved",
    )
    .await;

    let result = compile_rows(
        &competing_metric_fixture("points"),
        competing_metric_query(),
        CompileOptions::default(),
    )
    .await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == expected_diagnostic("competing_metric_ambiguous"))
    );
}

fn temporal_metric_fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("day", DataType::Date32, false),
        Field::new("score", DataType::Int64, false),
    ]));
    let mut relation = Relation::base("temporal_scores", schema, "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [(
            "monthly_score".into(),
            MetricDefinition {
                id: "metrics/monthly-score".into(),
                description: "Monthly score in the first quarter of 2024".into(),
                aliases: vec![],
                function: AggregateFunction::Sum,
                field: Some("score".into()),
                distinct: false,
                source_grain: vec!["id".into()],
                compatible_dimensions: Default::default(),
                compatible_lookup_dimensions: vec![],
                sum_rollup_dimensions: None,
                row_filters: vec![],
                result_type: DataType::Int64,
                unit: Presence::Value("points".into()),
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
    register(
        &mut engine,
        relation,
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Date32Array::from(vec![19737, 19763, 19792])),
            Arc::new(Int64Array::from(vec![1, 2, 4])),
        ],
    );
    engine
}

fn calendar_options(reference: &str, timezone: &str) -> CompileOptions {
    let mut options = CompileOptions::default();
    options.request_context = Some(RequestContext {
        reference_unix_millis: chrono::DateTime::parse_from_rfc3339(reference)
            .unwrap()
            .timestamp_millis(),
        timezone: timezone.into(),
        calendar: Calendar::Gregorian,
        origin: ContextOrigin::Caller,
    });
    options
}

fn temporal_metric_query(unit: CalendarUnit) -> RowQuery {
    let mut query = base_query("temporal_scores");
    query.requirements = vec![
        requirement(
            "period",
            RowOperation::CalendarFilter {
                field: field("r", "day"),
                period: CalendarPeriod {
                    unit,
                    offset: -1,
                    count: 1,
                },
            },
        ),
        requirement(
            "metric",
            RowOperation::Metric {
                name: "monthly_score".into(),
                alias: "score".into(),
                applicability: MetricApplicability {
                    required_unit: Some("points".into()),
                    required_source_grain: vec!["id".into()],
                    ..Default::default()
                },
            },
        ),
    ];
    query
}

#[tokio::test]
async fn metric_time_grain_and_coverage_reject_outside_the_authored_contract() {
    let engine = temporal_metric_fixture();
    let wrong_grain = compile_rows(
        &engine,
        temporal_metric_query(CalendarUnit::Day),
        calendar_options("2024-02-10T12:00:00Z", "UTC"),
    )
    .await;
    assert!(
        matches!(wrong_grain.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == expected_diagnostic("wrong_time_grain"))
    );
    let outside = compile_rows(
        &engine,
        temporal_metric_query(CalendarUnit::Month),
        calendar_options("2024-05-15T12:00:00Z", "UTC"),
    )
    .await;
    assert!(
        matches!(outside.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == expected_diagnostic("outside_coverage"))
    );
}

#[tokio::test]
async fn calendar_day_uses_half_open_bounds_across_a_dst_transition() {
    // Europe/Zurich 2024 spring transition: this local day is 23 hours.
    let start = 1_711_839_600_000_i64;
    let end = start + 23 * 60 * 60 * 1_000;
    let schema = Arc::new(Schema::new(vec![Field::new(
        "at",
        DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
        true,
    )]));
    let mut engine = Engine::new();
    register(
        &mut engine,
        Relation::base("events", schema, "memory"),
        vec![Arc::new(
            TimestampMillisecondArray::from(vec![
                Some(start - 1),
                Some(start),
                Some(end - 1),
                Some(end),
                None,
            ])
            .with_timezone("UTC"),
        )],
    );
    let mut query = base_query("events");
    query.requirements = vec![
        requirement(
            "today",
            RowOperation::CalendarFilter {
                field: field("r", "at"),
                period: CalendarPeriod {
                    unit: CalendarUnit::Day,
                    offset: 0,
                    count: 1,
                },
            },
        ),
        requirement(
            "count",
            RowOperation::Aggregate {
                function: AggregateFunction::Count,
                field: None,
                distinct: false,
                alias: "count".into(),
            },
        ),
    ];
    let artifact = compiled_row(
        compile_rows(
            &engine,
            query,
            calendar_options("2024-03-31T10:00:00Z", "Europe/Zurich"),
        )
        .await,
    );
    assert_eq!(
        artifact.sql().parameters(),
        &[
            Literal::Timestamp {
                ticks: start,
                unit: TimestampUnit::Millisecond,
                timezone: Some("UTC".into()),
            },
            Literal::Timestamp {
                ticks: end,
                unit: TimestampUnit::Millisecond,
                timezone: Some("UTC".into()),
            },
        ]
    );
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
    assert_eq!(rows(&direct), expected_rows("calendar_dst"));
    assert_eq!(rows(&sql), expected_rows("calendar_dst"));
}

struct ScriptedProvider {
    replies: Mutex<Vec<String>>,
    calls: Mutex<usize>,
}

impl ModelProvider for ScriptedProvider {
    async fn complete(&self, _messages: &[Message]) -> Result<String, ProviderError> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.replies.lock().unwrap().remove(0))
    }
}

fn scripted(replies: Vec<Value>) -> Compiler<ScriptedProvider> {
    Compiler::new(ScriptedProvider {
        replies: Mutex::new(replies.into_iter().map(|reply| reply.to_string()).collect()),
        calls: Mutex::new(0),
    })
}

fn retrieval_fixture() -> Engine {
    use semantic_catalog::{AiContext, FieldSemantics};

    let mut engine = Engine::new();
    for name in ["wide", "alternative", "distractor"] {
        let schema = Arc::new(Schema::new(vec![
            Field::new("ordinary", DataType::Int64, false),
            Field::new("rare_signal", DataType::Int64, true),
        ]));
        let mut relation = Relation::base(name, schema.clone(), "private:acceptance");
        relation.semantics = Some(RelationSemantics {
            ai_context: Some(AiContext {
                synonyms: vec!["rare concept".into()],
                ..Default::default()
            }),
            fields: [(
                "rare_signal".into(),
                FieldSemantics {
                    description: Some("Current rare signal".into()),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        });
        engine
            .register_table(
                relation,
                Arc::new(MemTable::try_new(schema, vec![vec![]]).unwrap()),
            )
            .unwrap();
    }
    engine
}

#[tokio::test]
async fn scope_and_context_bounds_fail_explicitly_and_scripted_provider_is_only_orchestration() {
    let engine = label_fixture();
    let mut denied = CompileOptions::default();
    denied.allowed_relations = Some(Default::default());
    let mut id_query = base_query("items");
    id_query.requirements.push(requirement(
        "id",
        RowOperation::Project {
            field: field("r", "id"),
            alias: "id".into(),
        },
    ));
    let result = compile_rows(&engine, id_query.clone(), denied).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == expected_diagnostic("denied_scope"))
    );

    let proposal = json!({ "status": "query", "query": id_query });
    let result = scripted(vec![proposal])
        .compile_typed(&engine, "List item identifiers", CompileOptions::default())
        .await;
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
    assert_eq!(result.record.work.model_calls, 1);

    let mut bounded = CompileOptions::default();
    bounded.selection_mode = SelectionMode::Retrieved;
    bounded.max_search_candidates = 1;
    let result = scripted(vec![])
        .compile_typed(&retrieval_fixture(), "rare signal", bounded)
        .await;
    assert!(
        matches!(result.outcome, TypedOutcome::Unresolved { diagnostic } if diagnostic.code == expected_diagnostic("bounded_context"))
    );
    assert_eq!(result.record.work.model_calls, 0);
}
