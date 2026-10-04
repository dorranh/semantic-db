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
    EmptyBehavior, FactResolution, MetricDefinition, Presence, Relation, RelationSemantics,
    RelationshipDefinition, RelationshipKey, Unit,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_graph};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::{graph::*, typed::*};

fn fixture() -> Engine {
    fixture_with(vec![Some(2), Some(4)])
}
fn fixture_with(spend: Vec<Option<i64>>) -> Engine {
    let mut engine = Engine::new();
    for (name, values) in [("revenue", vec![Some(10), Some(20)]), ("spend", spend)] {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "amount",
            DataType::Int64,
            true,
        )]));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))]).unwrap();
        engine
            .register_table(
                Relation::base(name, schema.clone(), "memory"),
                Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
            )
            .unwrap();
    }
    engine
}
fn leaf(name: &str) -> QueryNode {
    QueryNode {
        id: name.into(),
        source_text: format!("sum {name}"),
        operation: GraphOperation::Rows {
            query: RowQuery {
                version: 1,
                input: RelationInput {
                    relation: name.into(),
                    instance: "r".into(),
                },
                requirements: vec![Requirement {
                    id: "sum".into(),
                    source_text: format!("sum {name}"),
                    operation: RowOperation::Aggregate {
                        function: AggregateFunction::Sum,
                        field: Some(FieldRef {
                            instance: "r".into(),
                            field: "amount".into(),
                        }),
                        distinct: false,
                        alias: "amount".into(),
                    },
                }],
                unresolved: vec![],
            },
        },
    }
}
fn query() -> GraphQuery {
    GraphQuery {
        version: 1,
        nodes: vec![
            leaf("revenue"),
            leaf("spend"),
            QueryNode {
                id: "aligned".into(),
                source_text: "align totals".into(),
                operation: GraphOperation::Compose {
                    left: "revenue".into(),
                    right: "spend".into(),
                    relationship_relation: "".into(),
                    relationship: "".into(),
                    role: "".into(),
                    domain: GroupDomain::Union,
                    null_alignment: NullAlignment::Match,
                    keys: vec![],
                    outputs: vec![
                        CompositionOutput {
                            id: "revenue".into(),
                            side: Side::Left,
                            slot: "sum".into(),
                            alias: "revenue".into(),
                            missing: MissingGroup::Null,
                        },
                        CompositionOutput {
                            id: "spend".into(),
                            side: Side::Right,
                            slot: "sum".into(),
                            alias: "spend".into(),
                            missing: MissingGroup::Null,
                        },
                    ],
                },
            },
            QueryNode {
                id: "ratio".into(),
                source_text: "revenue divided by spend".into(),
                operation: GraphOperation::Calculate {
                    input: "aligned".into(),
                    passthrough: vec![GraphProjection {
                        id: "amount".into(),
                        slot: "revenue".into(),
                        alias: "revenue".into(),
                    }],
                    ratios: vec![GraphRatio {
                        id: "ratio".into(),
                        numerator: "revenue".into(),
                        denominator: "spend".into(),
                        required_unit: None,
                        zero: ZeroDivision::Null,
                        alias: "ratio".into(),
                    }],
                },
            },
            QueryNode {
                id: "positive".into(),
                source_text: "only ratios above four".into(),
                operation: GraphOperation::Filter {
                    input: "ratio".into(),
                    predicate: RowPredicate::Compare {
                        field: OutputRef {
                            slot: "ratio".into(),
                        },
                        operator: Comparison::Gt,
                        value: Literal::Decimal128 {
                            coefficient: "4000000000000000000".into(),
                            precision: 38,
                            scale: 18,
                        },
                    },
                },
            },
        ],
        root: "positive".into(),
        ordering: vec![GraphOrder {
            slot: "ratio".into(),
            direction: Direction::Desc,
            nulls: NullOrder::Last,
        }],
        limit: Some(1),
        unresolved: vec![],
    }
}
fn rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| {
                (0..batch.num_columns())
                    .map(|column| {
                        array_value_to_string(batch.column(column).as_ref(), row).unwrap()
                    })
                    .collect()
            })
        })
        .collect()
}
#[tokio::test]
async fn scalar_composition_calculates_filters_and_replays_with_backend_parity() {
    let engine = fixture();
    let result = compile_graph(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query: artifact } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let direct = rows(
        &artifact
            .plan_direct(&engine)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
    );
    let sql = rows(
        &artifact
            .execute(&engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
    );
    assert_eq!(direct, sql);
    assert_eq!(direct.len(), 1);
    assert_eq!(direct[0][0], "30");
    assert!(direct[0][1].starts_with('5'));
    let bundle = artifact.capture_replay(128 * 1024).unwrap();
    let replay = bundle
        .replay(&engine, CompileOptions::default())
        .await
        .unwrap();
    assert!(matches!(replay.outcome, TypedOutcome::CompiledGraph { .. }));
}
#[tokio::test]
async fn calculation_rejects_wrong_types_duplicate_outputs_and_cycles() {
    let engine = fixture();
    let mut wrong = query();
    if let GraphOperation::Calculate { ratios, .. } = &mut wrong.nodes[3].operation {
        ratios[0].denominator = "missing".into();
    }
    assert!(
        matches!(compile_graph(&engine, wrong, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_slot")
    );
    let mut duplicate = query();
    if let GraphOperation::Calculate { ratios, .. } = &mut duplicate.nodes[3].operation {
        ratios[0].alias = "revenue".into();
    }
    assert!(
        matches!(compile_graph(&engine, duplicate, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_output")
    );
    let mut cycle = query();
    cycle.nodes = vec![cycle.nodes[3].clone()];
    cycle.root = "ratio".into();
    if let GraphOperation::Calculate { input, .. } = &mut cycle.nodes[0].operation {
        *input = "ratio".into();
    }
    assert!(
        matches!(compile_graph(&engine, cycle, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_cycle")
    );
}

#[tokio::test]
async fn zero_and_null_denominators_keep_exact_ratio_contract() {
    for (spend, zero, expected) in [
        (vec![Some(0)], ZeroDivision::Null, ""),
        (vec![Some(0)], ZeroDivision::Zero, "0"),
        (vec![None], ZeroDivision::Zero, ""),
    ] {
        let engine = fixture_with(spend);
        let mut graph = query();
        graph.nodes.pop();
        graph.root = "ratio".into();
        graph.limit = None;
        if let GraphOperation::Calculate { ratios, .. } = &mut graph.nodes[3].operation {
            ratios[0].zero = zero;
        }
        let result = compile_graph(&engine, graph, CompileOptions::default()).await;
        let TypedOutcome::CompiledGraph { query: artifact } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
        let direct = rows(
            &artifact
                .plan_direct(&engine)
                .await
                .unwrap()
                .collect()
                .await
                .unwrap(),
        );
        let sql = rows(
            &artifact
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .unwrap(),
        );
        assert_eq!(direct, sql);
        assert_eq!(direct.len(), 1);
        assert!(direct[0][1].starts_with(expected));
        if expected.is_empty() {
            assert_eq!(direct[0][1], "");
        }
    }
}

#[tokio::test]
async fn graph_intent_requires_calculation_outputs_and_filter_node_evidence() {
    let engine = fixture();
    let graph = query();
    let request = "sum revenue, sum spend, align totals, revenue divided by spend, only ratios above four, sort and take one";
    let entries = [
        (
            GraphRequirementRef::Node {
                node: "revenue".into(),
            },
            "sum revenue",
        ),
        (
            GraphRequirementRef::Leaf {
                node: "revenue".into(),
                requirement: "sum".into(),
            },
            "sum revenue",
        ),
        (
            GraphRequirementRef::Node {
                node: "spend".into(),
            },
            "sum spend",
        ),
        (
            GraphRequirementRef::Leaf {
                node: "spend".into(),
                requirement: "sum".into(),
            },
            "sum spend",
        ),
        (
            GraphRequirementRef::Node {
                node: "aligned".into(),
            },
            "align totals",
        ),
        (
            GraphRequirementRef::Output {
                node: "aligned".into(),
                slot: "revenue".into(),
            },
            "align totals",
        ),
        (
            GraphRequirementRef::Output {
                node: "aligned".into(),
                slot: "spend".into(),
            },
            "align totals",
        ),
        (
            GraphRequirementRef::Node {
                node: "ratio".into(),
            },
            "revenue divided by spend",
        ),
        (
            GraphRequirementRef::Output {
                node: "ratio".into(),
                slot: "amount".into(),
            },
            "revenue divided by spend",
        ),
        (
            GraphRequirementRef::Output {
                node: "ratio".into(),
                slot: "ratio".into(),
            },
            "revenue divided by spend",
        ),
        (
            GraphRequirementRef::Node {
                node: "positive".into(),
            },
            "only ratios above four",
        ),
        (GraphRequirementRef::Order { index: 0 }, "sort and take one"),
        (GraphRequirementRef::Limit, "sort and take one"),
    ];
    let evidence = GraphRequestEvidence {
        version: 1,
        request_id: "ratio-request".into(),
        original_request: request.into(),
        requirements: entries
            .into_iter()
            .map(|(target, text)| {
                let start = request.find(text).unwrap();
                GraphRequirementEvidence {
                    target,
                    source_spans: vec![RequestSpan {
                        start,
                        end: start + text.len(),
                    }],
                }
            })
            .collect(),
        unresolved_alternatives: vec![],
    };
    let accepted = semantic_compiler::typed::compile_graph_intent(
        &engine,
        GraphIntentQuery {
            query: graph.clone(),
            evidence: evidence.clone(),
        },
        CompileOptions::default(),
    )
    .await;
    assert!(
        accepted.record.request_spans_validated,
        "{:?}",
        accepted.outcome
    );
    assert!(matches!(
        accepted.outcome,
        TypedOutcome::CompiledGraph { .. }
    ));
    let mut removed = evidence;
    removed.requirements.retain(|item| {
        item.target
            != GraphRequirementRef::Output {
                node: "ratio".into(),
                slot: "ratio".into(),
            }
    });
    assert!(
        matches!(semantic_compiler::typed::compile_graph_intent(&engine, GraphIntentQuery { query: graph, evidence: removed }, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "request_coverage")
    );
}

fn grouped_fixture() -> Engine {
    let mut engine = Engine::new();
    for (name, products, amounts) in [
        (
            "revenue",
            vec!["X", "X", "Y", "Z"],
            vec![Some(10), Some(20), None, Some(7)],
        ),
        (
            "spend",
            vec!["X", "X", "Y", "W"],
            vec![Some(2), Some(4), Some(0), Some(9)],
        ),
    ] {
        let schema = Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("amount", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["A"; products.len()])),
                Arc::new(StringArray::from(products)),
                Arc::new(Int64Array::from(amounts)),
            ],
        )
        .unwrap();
        let mut relation = Relation::base(name, schema.clone(), "memory");
        if name == "revenue" {
            relation.semantics = Some(RelationSemantics {
                relationships: [(
                    "same_group".into(),
                    RelationshipDefinition {
                        ai_context: None,
                        id: "relationships/same-group".into(),
                        right_relation: "spend".into(),
                        role: "same_group".into(),
                        key_pairs: vec![
                            RelationshipKey {
                                left_field: "region".into(),
                                right_field: "region".into(),
                            },
                            RelationshipKey {
                                left_field: "product".into(),
                                right_field: "product".into(),
                            },
                        ],
                        null_keys_match: false,
                        cardinality: FactResolution::Unknown,
                        source_refs: vec![],
                    },
                )]
                .into(),
                ..Default::default()
            });
        }
        engine
            .register_table(
                relation,
                Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
            )
            .unwrap();
    }
    engine
}
fn grouped_leaf(name: &str) -> QueryNode {
    QueryNode {
        id: name.into(),
        source_text: format!("{name} by region and product"),
        operation: GraphOperation::Rows {
            query: RowQuery {
                version: 1,
                input: RelationInput {
                    relation: name.into(),
                    instance: "r".into(),
                },
                requirements: ["region", "product"]
                    .into_iter()
                    .map(|field| Requirement {
                        id: field.into(),
                        source_text: field.into(),
                        operation: RowOperation::Group {
                            field: FieldRef {
                                instance: "r".into(),
                                field: field.into(),
                            },
                            alias: field.into(),
                        },
                    })
                    .chain(std::iter::once(Requirement {
                        id: "amount".into(),
                        source_text: "sum amount".into(),
                        operation: RowOperation::Aggregate {
                            function: AggregateFunction::Sum,
                            field: Some(FieldRef {
                                instance: "r".into(),
                                field: "amount".into(),
                            }),
                            distinct: false,
                            alias: "amount".into(),
                        },
                    }))
                    .collect(),
                unresolved: vec![],
            },
        },
    }
}
#[tokio::test]
async fn composite_alignment_distinguishes_missing_groups_from_present_null_and_zero() {
    let engine = grouped_fixture();
    let mut graph = query();
    graph.nodes[0] = grouped_leaf("revenue");
    graph.nodes[1] = grouped_leaf("spend");
    if let GraphOperation::Compose {
        relationship_relation,
        relationship,
        role,
        keys,
        outputs,
        ..
    } = &mut graph.nodes[2].operation
    {
        *relationship_relation = "revenue".into();
        *relationship = "same_group".into();
        *role = "same_group".into();
        *keys = ["region", "product"]
            .into_iter()
            .map(|field| SetColumn {
                id: field.into(),
                left: field.into(),
                right: field.into(),
                alias: field.into(),
            })
            .collect();
        outputs[0].slot = "amount".into();
        outputs[1].slot = "amount".into();
        outputs[0].missing = MissingGroup::Zero;
        outputs[1].missing = MissingGroup::Zero;
    }
    if let GraphOperation::Calculate { passthrough, .. } = &mut graph.nodes[3].operation {
        *passthrough = vec![
            GraphProjection {
                id: "region".into(),
                slot: "region".into(),
                alias: "region".into(),
            },
            GraphProjection {
                id: "product".into(),
                slot: "product".into(),
                alias: "product".into(),
            },
            GraphProjection {
                id: "revenue".into(),
                slot: "revenue".into(),
                alias: "revenue".into(),
            },
        ];
    }
    graph.nodes.pop();
    graph.root = "ratio".into();
    graph.limit = None;
    graph.ordering = vec![GraphOrder {
        slot: "product".into(),
        direction: Direction::Asc,
        nulls: NullOrder::Last,
    }];
    let result = compile_graph(&engine, graph, CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query: artifact } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let direct = rows(
        &artifact
            .plan_direct(&engine)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
    );
    let sql = rows(
        &artifact
            .execute(&engine, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
    );
    assert_eq!(direct, sql);
    assert_eq!(direct.len(), 4);
    assert_eq!(direct[0][1], "W");
    assert_eq!(direct[0][2], "0");
    assert_eq!(direct[1][1], "X");
    assert_eq!(direct[1][2], "30");
    assert!(direct[1][3].starts_with('5'));
    assert_eq!(direct[2][1], "Y");
    assert_eq!(direct[2][2], "");
    assert_eq!(direct[2][3], "");
    assert_eq!(direct[3][1], "Z");
    assert_eq!(direct[3][2], "7");
    assert_eq!(direct[3][3], "");
}

fn governed_fixture(revenue_unit: Presence<Unit>, spend_unit: Presence<Unit>) -> Engine {
    let mut engine = Engine::new();
    for (name, value, unit) in [("revenue", 30, revenue_unit), ("spend", 6, spend_unit)] {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "amount",
            DataType::Int64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(vec![value]))],
        )
        .unwrap();
        let mut relation = Relation::base(name, schema.clone(), "memory");
        relation.semantics = Some(RelationSemantics {
            metrics: [(
                "total".into(),
                MetricDefinition {
                    id: format!("metrics/{name}-total"),
                    description: format!("{name} total"),
                    aliases: vec![],
                    function: AggregateFunction::Sum,
                    field: Some("amount".into()),
                    distinct: false,
                    source_grain: semantic_catalog::SourceGrain {
                        entity: None,
                        keys: vec![semantic_catalog::GrainKey {
                            relation: name.into(),
                            field: "amount".into(),
                        }],
                    },
                    compatible_dimensions: Default::default(),
                    compatible_lookup_dimensions: vec![],
                    sum_rollup_dimensions: None,
                    state: None,
                    row_filters: vec![],
                    result_type: DataType::Int64,
                    unit,
                    temporal: Presence::Missing,
                    empty_behavior: EmptyBehavior::Null,
                    source_refs: vec![],
                },
            )]
            .into(),
            ..Default::default()
        });
        engine
            .register_table(
                relation,
                Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
            )
            .unwrap();
    }
    engine
}

fn governed_ratio_graph(required_unit: Option<Unit>) -> GraphQuery {
    let mut graph = query();
    graph.nodes.pop();
    graph.root = "ratio".into();
    graph.limit = None;
    for node in &mut graph.nodes[..2] {
        let metric_id = format!("metrics/{}-total", node.id);
        let GraphOperation::Rows { query } = &mut node.operation else {
            unreachable!()
        };
        query.requirements[0].operation = RowOperation::Metric {
            name: metric_id,
            alias: "amount".into(),
            applicability: MetricApplicability::default(),
        };
    }
    let GraphOperation::Calculate { ratios, .. } = &mut graph.nodes[3].operation else {
        unreachable!()
    };
    ratios[0].required_unit = required_unit;
    graph
}

#[tokio::test]
async fn governed_units_produce_checked_quotient_without_guessing_unknowns() {
    for (left, right, expected) in [
        (
            Presence::Value(Unit::Currency { code: "USD".into() }),
            Presence::Value(Unit::Currency { code: "USD".into() }),
            Some(Unit::Dimensionless),
        ),
        (
            Presence::Value(Unit::Currency { code: "USD".into() }),
            Presence::Value(Unit::Currency { code: "EUR".into() }),
            Some(Unit::Quotient {
                numerator: Box::new(Unit::Currency { code: "USD".into() }),
                denominator: Box::new(Unit::Currency { code: "EUR".into() }),
            }),
        ),
        (
            Presence::Missing,
            Presence::Value(Unit::Currency { code: "USD".into() }),
            None,
        ),
    ] {
        let engine = governed_fixture(left, right);
        let graph = governed_ratio_graph(None);
        let result = compile_graph(&engine, graph, CompileOptions::default()).await;
        let TypedOutcome::CompiledGraph { query: artifact } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
        match (&artifact.slot_meaning("ratio").unwrap().unit, expected) {
            (
                FactResolution::Known {
                    value: Presence::Value(actual),
                    ..
                },
                Some(expected),
            ) => assert_eq!(actual, &expected),
            (FactResolution::Unknown, None) => {}
            (actual, expected) => panic!("unexpected unit {actual:?}, expected {expected:?}"),
        }
    }
}

#[tokio::test]
async fn required_ratio_unit_rejects_authored_mismatch_or_unknown() {
    let required = Unit::Dimensionless;
    let matched = governed_fixture(
        Presence::Value(Unit::Currency { code: "USD".into() }),
        Presence::Value(Unit::Currency { code: "USD".into() }),
    );
    assert!(matches!(
        compile_graph(
            &matched,
            governed_ratio_graph(Some(required.clone())),
            CompileOptions::default()
        )
        .await
        .outcome,
        TypedOutcome::CompiledGraph { .. }
    ));
    for (numerator, denominator) in [
        (
            Presence::Value(Unit::Currency { code: "USD".into() }),
            Presence::Value(Unit::Currency { code: "EUR".into() }),
        ),
        (
            Presence::Missing,
            Presence::Value(Unit::Currency { code: "USD".into() }),
        ),
    ] {
        let engine = governed_fixture(numerator, denominator);
        assert!(matches!(
            compile_graph(
                &engine,
                governed_ratio_graph(Some(required.clone())),
                CompileOptions::default()
            )
            .await
            .outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_ratio_unit"
        ));
    }
}

#[tokio::test]
async fn metric_request_distinguishes_currency_from_same_named_unit() {
    let engine = governed_fixture(
        Presence::Value(Unit::Currency { code: "USD".into() }),
        Presence::Value(Unit::Currency { code: "USD".into() }),
    );
    for (requested, accepted) in [
        (Unit::Currency { code: "USD".into() }, true),
        (Unit::Named { id: "USD".into() }, false),
    ] {
        let mut graph = governed_ratio_graph(None);
        let GraphOperation::Rows { query } = &mut graph.nodes[0].operation else {
            unreachable!()
        };
        let RowOperation::Metric { applicability, .. } = &mut query.requirements[0].operation
        else {
            unreachable!()
        };
        applicability.required_unit = Some(requested);
        let result = compile_graph(&engine, graph, CompileOptions::default()).await;
        if accepted {
            assert!(matches!(result.outcome, TypedOutcome::CompiledGraph { .. }));
        } else {
            assert!(matches!(
                result.outcome,
                TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_unit"
            ));
        }
    }
}

#[tokio::test]
async fn projection_preserves_only_complete_unique_original_group_keys() {
    for mode in ["complete", "duplicate", "missing", "global"] {
        for calculate in [false, true] {
            let global = mode == "global";
            let engine = if global { fixture() } else { grouped_fixture() };
            let mut graph = query();
            graph.nodes.truncate(3);
            graph.root = "aligned".into();
            graph.ordering.clear();
            graph.limit = None;
            if !global {
                graph.nodes[0] = grouped_leaf("revenue");
                graph.nodes[1] = grouped_leaf("spend");
            }
            let columns = if global {
                vec![GraphProjection {
                    id: "total".into(),
                    slot: "sum".into(),
                    alias: "total".into(),
                }]
            } else {
                let mut columns = vec![GraphProjection {
                    id: "total".into(),
                    slot: "amount".into(),
                    alias: "total".into(),
                }];
                if mode != "missing" {
                    columns.extend([
                        GraphProjection {
                            id: "key_a".into(),
                            slot: "region".into(),
                            alias: "key_a".into(),
                        },
                        GraphProjection {
                            id: "key_b".into(),
                            slot: if mode == "duplicate" {
                                "region"
                            } else {
                                "product"
                            }
                            .into(),
                            alias: "key_b".into(),
                        },
                    ]);
                }
                columns
            };
            graph.nodes.insert(
                2,
                QueryNode {
                    id: "selected".into(),
                    source_text: "select totals and keys".into(),
                    operation: if calculate {
                        GraphOperation::Calculate {
                            input: "revenue".into(),
                            passthrough: columns,
                            ratios: vec![GraphRatio {
                                id: "check_ratio".into(),
                                numerator: if global { "sum" } else { "amount" }.into(),
                                denominator: if global { "sum" } else { "amount" }.into(),
                                required_unit: None,
                                zero: ZeroDivision::Null,
                                alias: "check_ratio".into(),
                            }],
                        }
                    } else {
                        GraphOperation::Project {
                            input: "revenue".into(),
                            columns,
                        }
                    },
                },
            );
            let GraphOperation::Compose {
                left,
                relationship_relation,
                relationship,
                role,
                keys,
                outputs,
                ..
            } = &mut graph.nodes[3].operation
            else {
                unreachable!()
            };
            *left = "selected".into();
            outputs[0].slot = "total".into();
            if !global {
                *relationship_relation = "revenue".into();
                *relationship = "same_group".into();
                *role = "same_group".into();
                outputs[1].slot = "amount".into();
                *keys = if mode == "missing" {
                    vec![]
                } else {
                    [("key_a", "region"), ("key_b", "product")]
                        .into_iter()
                        .map(|(left, right)| SetColumn {
                            id: right.into(),
                            left: left.into(),
                            right: right.into(),
                            alias: right.into(),
                        })
                        .collect()
                };
            }
            let result = compile_graph(&engine, graph, CompileOptions::default()).await;
            if matches!(mode, "duplicate" | "missing") {
                assert!(
                    matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "composition_grain"),
                    "mode={mode}"
                );
            } else {
                let TypedOutcome::CompiledGraph { query } = result.outcome else {
                    panic!("mode={mode}: {:?}", result.outcome)
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
                assert_eq!(direct[0].schema(), sql[0].schema());
                assert_eq!(
                    direct.iter().map(RecordBatch::num_rows).sum::<usize>(),
                    sql.iter().map(RecordBatch::num_rows).sum::<usize>()
                );
            }
        }
    }
}
