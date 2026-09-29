use std::sync::{Arc, Mutex};

use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Date32Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::{
    Compiler,
    provider::{Message, ModelProvider, ProviderError},
    typed::{CompileOptions, CompiledQuery, TypedCompilation, TypedOutcome, compile_rows},
};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;
use serde_json::json;

fn fixture() -> Engine {
    let fields = vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, true),
        Field::new("active", DataType::Boolean, true),
        Field::new("score", DataType::Int64, true),
    ];
    let schema = Arc::new(Schema::new(fields));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])),
        Arc::new(StringArray::from(vec![
            Some("O'Reilly"),
            Some("b"),
            None,
            Some("O'Reilly"),
            Some("B"),
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
    ];
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    let table = MemTable::try_new(schema.clone(), vec![vec![batch]]).unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("items", schema, "private:source"),
            Arc::new(table),
        )
        .unwrap();
    engine
}
fn field(name: &str) -> FieldRef {
    FieldRef {
        instance: "r".into(),
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
fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "items".into(),
            instance: "r".into(),
        },
        requirements: vec![requirement(
            "ids",
            RowOperation::Project {
                field: field("id"),
                alias: "id".into(),
            },
        )],
        unresolved: vec![],
    }
}
fn compare(name: &str, operator: Comparison, value: Literal) -> RowPredicate {
    RowPredicate::Compare {
        field: field(name),
        operator,
        value,
    }
}
fn compiled(result: TypedCompilation) -> Box<CompiledQuery> {
    match result.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("{other:?}"),
    }
}
fn values(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    batches
        .iter()
        .flat_map(|b| {
            (0..b.num_rows()).map(|r| {
                (0..b.num_columns())
                    .map(|c| array_value_to_string(b.column(c), r).unwrap())
                    .collect()
            })
        })
        .collect()
}
async fn both_paths(engine: &Engine, query: RowQuery, expected: Vec<Vec<&str>>) {
    let expected_relations = 1 + query
        .requirements
        .iter()
        .filter(|r| {
            matches!(
                r.operation,
                RowOperation::Related { .. } | RowOperation::Lookup { .. }
            )
        })
        .count();
    let result = compile_rows(engine, query, CompileOptions::default()).await;
    assert_eq!(result.record.work.model_calls, 0);
    assert_eq!(result.record.work.relations_looked_up, expected_relations);
    assert!(result.record.artifact_digest.is_some());
    let query = compiled(result);
    let direct = query
        .plan_direct(engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let emitted = query
        .execute(engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(values(&direct), expected);
    assert_eq!(values(&emitted), expected);
}

#[tokio::test]
async fn preserves_boolean_nulls_filter_order_and_limit_across_both_backends() {
    let engine = fixture();
    let mut q = query();
    q.requirements.push(requirement(
        "active-or-unknown-score",
        RowOperation::Filter {
            predicate: RowPredicate::Any {
                predicates: vec![
                    compare("active", Comparison::Eq, Literal::Boolean(true)),
                    RowPredicate::IsNull {
                        field: field("score"),
                        negated: false,
                    },
                ],
            },
        },
    ));
    q.requirements.push(requirement(
        "ordering",
        RowOperation::Order {
            field: field("score"),
            direction: Direction::Desc,
            nulls: NullOrder::First,
        },
    ));
    q.requirements.push(requirement(
        "tie-break",
        RowOperation::Order {
            field: field("id"),
            direction: Direction::Desc,
            nulls: NullOrder::Last,
        },
    ));
    q.requirements
        .push(requirement("limit", RowOperation::Limit { count: 2 }));
    both_paths(&engine, q, vec![vec!["4"], vec!["5"]]).await;
    let mut q = query();
    q.requirements.push(requirement(
        "not-active",
        RowOperation::Filter {
            predicate: RowPredicate::Not {
                predicate: Box::new(compare("active", Comparison::Eq, Literal::Boolean(true))),
            },
        },
    ));
    both_paths(&engine, q, vec![vec!["2"]]).await;
    let mut q = query();
    q.requirements
        .push(requirement("zero", RowOperation::Limit { count: 0 }));
    both_paths(&engine, q, vec![]).await;
}

#[tokio::test]
async fn literal_parameters_case_and_duplicate_rows_are_preserved() {
    let engine = fixture();
    let mut q = query();
    q.requirements = vec![
        requirement(
            "label",
            RowOperation::Project {
                field: field("label"),
                alias: "id".into(),
            },
        ),
        requirement(
            "quote",
            RowOperation::Filter {
                predicate: compare("label", Comparison::Eq, Literal::Utf8("O'Reilly".into())),
            },
        ),
        requirement(
            "order-source-id",
            RowOperation::Order {
                field: field("id"),
                direction: Direction::Desc,
                nulls: NullOrder::Last,
            },
        ),
    ];
    let artifact = compiled(compile_rows(&engine, q.clone(), CompileOptions::default()).await);
    assert!(!artifact.sql().statement().contains("O'Reilly"));
    assert_eq!(
        artifact.sql().parameters(),
        &[Literal::Utf8("O'Reilly".into())]
    );
    both_paths(&engine, q, vec![vec!["O'Reilly"], vec!["O'Reilly"]]).await;
    let mut q = query();
    q.requirements.push(requirement(
        "case",
        RowOperation::Filter {
            predicate: compare("label", Comparison::Eq, Literal::Utf8("B".into())),
        },
    ));
    both_paths(&engine, q, vec![vec!["5"]]).await;
    let mut q = query();
    q.requirements.push(requirement(
        "explicit-unseen-value",
        RowOperation::Filter {
            predicate: compare(
                "label",
                Comparison::Eq,
                Literal::Utf8("'; DROP TABLE items; --".into()),
            ),
        },
    ));
    both_paths(&engine, q, vec![]).await;
}

#[tokio::test]
async fn authored_views_and_catalog_drift() {
    let mut engine = fixture();
    engine
        .create_view(
            "active_items",
            "SELECT id, label FROM items WHERE active = true",
        )
        .await
        .unwrap();
    let mut q = query();
    q.input.relation = "active_items".into();
    both_paths(&engine, q.clone(), vec![vec!["1"], vec!["4"], vec!["5"]]).await;
    let old = compiled(compile_rows(&engine, q, CompileOptions::default()).await);
    engine
        .create_view("new_alternative", "SELECT id FROM items")
        .await
        .unwrap();
    assert_eq!(
        old.plan_direct(&engine).await.unwrap_err().code,
        "snapshot_mismatch"
    );
    assert_eq!(
        old.execute(&engine, QueryOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
}

#[tokio::test]
async fn rejects_unbound_scopes_types_ambiguous_outputs_and_unresolved_choices() {
    let engine = fixture();
    let mutations = vec![
        ("unknown_relation", {
            let mut q = query();
            q.input.relation = "information_schema.tables".into();
            q
        }),
        ("invalid_scope", {
            let mut q = query();
            q.input.instance = "other".into();
            q
        }),
        ("unknown_field", {
            let mut q = query();
            q.requirements[0].operation = RowOperation::Project {
                field: field("missing"),
                alias: "id".into(),
            };
            q
        }),
        ("comparison_type", {
            let mut q = query();
            q.requirements.push(requirement(
                "bad-type",
                RowOperation::Filter {
                    predicate: compare("id", Comparison::Eq, Literal::Utf8("1".into())),
                },
            ));
            q
        }),
        ("invalid_requirement", {
            let mut q = query();
            q.requirements.push(q.requirements[0].clone());
            q
        }),
        ("invalid_output", {
            let mut q = query();
            q.requirements.push(requirement(
                "duplicate-alias",
                RowOperation::Project {
                    field: field("label"),
                    alias: "id".into(),
                },
            ));
            q
        }),
        ("conflicting_limits", {
            let mut q = query();
            q.requirements
                .push(requirement("limit1", RowOperation::Limit { count: 1 }));
            q.requirements
                .push(requirement("limit2", RowOperation::Limit { count: 2 }));
            q
        }),
        ("empty_boolean", {
            let mut q = query();
            q.requirements.push(requirement(
                "empty",
                RowOperation::Filter {
                    predicate: RowPredicate::All { predicates: vec![] },
                },
            ));
            q
        }),
        ("unresolved_terms", {
            let mut q = query();
            q.unresolved.push("deep".into());
            q
        }),
    ];
    for (expected, q) in mutations {
        let result = compile_rows(&engine, q, CompileOptions::default()).await;
        let error = match result.outcome {
            TypedOutcome::Rejected { diagnostic } | TypedOutcome::Unresolved { diagnostic } => {
                diagnostic
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(error.code, expected);
        assert!(result.record.artifact_digest.is_none());
    }
}

#[tokio::test]
async fn deterministic_replay_and_bounded_private_records() {
    let engine = fixture();
    let first = compile_rows(&engine, query(), CompileOptions::default()).await;
    let artifact = compiled(compile_rows(&engine, query(), CompileOptions::default()).await);
    let replay = serde_json::from_value(serde_json::to_value(artifact.intent()).unwrap()).unwrap();
    let second = compile_rows(&engine, replay, CompileOptions::default()).await;
    assert_eq!(first.record.artifact_digest, second.record.artifact_digest);
    let text = serde_json::to_string(&first.record).unwrap();
    assert!(!text.contains("private:source"));
    assert!(!text.contains("SELECT"));
    assert_eq!(
        first
            .record
            .stages
            .iter()
            .map(|s| s.stage)
            .collect::<Vec<_>>(),
        vec!["bind", "lower", "backend", "complete"]
    );
    let mut options = CompileOptions::default();
    options.max_nodes = 0;
    assert!(matches!(
        compile_rows(&engine, query(), options).await.outcome,
        TypedOutcome::Unresolved { .. }
    ));
    let options = CompileOptions::default();
    options.cancellation.cancel();
    let result = compile_rows(&engine, query(), options).await;
    assert!(
        matches!(result.outcome,TypedOutcome::Unresolved {ref diagnostic} if diagnostic.code=="cancelled")
    );
    let mut q = query();
    let mut p = compare("id", Comparison::Eq, Literal::Int64(1));
    for _ in 0..40 {
        p = RowPredicate::Not {
            predicate: Box::new(p),
        };
    }
    q.requirements
        .push(requirement("deep", RowOperation::Filter { predicate: p }));
    assert!(matches!(
        compile_rows(&engine, q, CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::Unresolved { .. }
    ));
}

struct Scripted {
    replies: Mutex<Vec<String>>,
    calls: Mutex<Vec<Vec<Message>>>,
}
impl ModelProvider for Scripted {
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.calls.lock().unwrap().push(messages.to_vec());
        Ok(self.replies.lock().unwrap().remove(0))
    }
}
fn compiler(replies: Vec<String>) -> Compiler<Scripted> {
    Compiler::new(Scripted {
        replies: Mutex::new(replies),
        calls: Mutex::new(vec![]),
    })
}

#[tokio::test]
async fn model_repairs_typed_proposals_but_never_accepts_sql_fallback() {
    let engine = fixture();
    let proposal = json!({"status":"query","query":query()}).to_string();
    let compiler = compiler(vec![
        json!({"status":"grounded","query":{"sql":"SELECT * FROM items"}}).to_string(),
        proposal,
    ]);
    let result = compiler
        .compile_typed(&engine, "List IDs", CompileOptions::default())
        .await;
    assert_eq!(result.record.work.model_calls, 2);
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
    assert!(result.record.work.model_input_bytes > result.record.work.context_bytes * 2);
    let c = compiler_typed_sql();
    assert!(matches!(
        c.compile_typed(&engine, "List IDs", CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::Rejected { .. }
    ));
}
fn compiler_typed_sql() -> Compiler<Scripted> {
    compiler(vec![
        json!({"status":"query","query":{"sql":"SELECT * FROM items"}}).to_string(),
    ])
    .with_max_repairs(0)
}

#[tokio::test]
async fn context_cutoffs_do_not_call_model_or_claim_absence() {
    let engine = fixture();
    for field_cutoff in [true, false] {
        let mut options = CompileOptions::default();
        if field_cutoff {
            options.max_context_fields = 1;
        } else {
            options.max_context_bytes = 10;
        }
        let result = compiler(vec![])
            .compile_typed(&engine, "List IDs", options)
            .await;
        assert_eq!(result.record.work.model_calls, 0);
        assert!(
            matches!(result.outcome,TypedOutcome::Unresolved {ref diagnostic} if diagnostic.code=="context_limit")
        );
    }
    for response in [
        json!({"status":"needs_clarification","phrases":["deep"],"question":"Which cutoff?"}),
        json!({"status":"unsupported","reason":"Joins require a future capability"}),
        json!({"status":"unresolved","reason":"No established mapping"}),
    ] {
        let result = compiler(vec![response.to_string()])
            .compile_typed(&engine, "Find deep items", CompileOptions::default())
            .await;
        assert_eq!(result.record.work.model_calls, 1);
        assert!(!matches!(
            result.outcome,
            TypedOutcome::Compiled { .. } | TypedOutcome::Rejected { .. }
        ));
    }
}

#[tokio::test]
async fn deadline_and_cancellation_interrupt_model_wait_without_retry() {
    struct Pending;
    impl ModelProvider for Pending {
        async fn complete(&self, _: &[Message]) -> Result<String, ProviderError> {
            std::future::pending().await
        }
    }
    let engine = fixture();
    let mut options = CompileOptions::default();
    options.timeout = std::time::Duration::from_millis(10);
    let result = Compiler::new(Pending)
        .compile_typed(&engine, "List IDs", options)
        .await;
    assert!(
        matches!(result.outcome,TypedOutcome::Unresolved {ref diagnostic} if diagnostic.code=="deadline")
    );
    let options = CompileOptions::default();
    let cancel = options.cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        cancel.cancel();
    });
    let result = Compiler::new(Pending)
        .compile_typed(&engine, "List IDs", options)
        .await;
    assert!(
        matches!(result.outcome,TypedOutcome::Unresolved {ref diagnostic} if diagnostic.code=="cancelled")
    );
    assert_eq!(result.record.work.model_calls, 1);
    assert_eq!(
        result.record.model_attempts[0].failure_code,
        Some("interrupted")
    );
    assert_eq!(result.record.token_accounting.calls_missing_input_usage, 1);
}

#[tokio::test]
async fn wide_catalog_binding_uses_indexed_fields_and_never_scans_providers() {
    #[derive(Debug)]
    struct NoScan(Arc<Schema>);
    #[async_trait::async_trait]
    impl datafusion::catalog::TableProvider for NoScan {
        fn schema(&self) -> Arc<Schema> {
            self.0.clone()
        }
        fn table_type(&self) -> datafusion::logical_expr::TableType {
            datafusion::logical_expr::TableType::Base
        }
        async fn scan(
            &self,
            _: &dyn datafusion::catalog::Session,
            _: Option<&Vec<usize>>,
            _: &[datafusion::logical_expr::Expr],
            _: Option<usize>,
        ) -> datafusion::error::Result<Arc<dyn datafusion::physical_plan::ExecutionPlan>> {
            panic!("compilation must not scan a provider")
        }
    }
    let mut engine = Engine::new();
    for i in 0..100 {
        let fields = (0..100)
            .map(|n| Field::new(format!("f{n}"), DataType::Int64, false))
            .collect::<Vec<_>>();
        let schema = Arc::new(Schema::new(fields));
        engine
            .register_table(
                Relation::base(format!("r{i}"), schema.clone(), format!("private:{i}")),
                Arc::new(NoScan(schema)),
            )
            .unwrap();
    }
    let schema = Arc::new(Schema::new(
        (0..10_000)
            .map(|n| Field::new(format!("f{n}"), DataType::Int64, false))
            .collect::<Vec<_>>(),
    ));
    engine
        .register_table(
            Relation::base("wide", schema.clone(), "private:wide"),
            Arc::new(NoScan(schema)),
        )
        .unwrap();
    // Publish before request work, which can reuse the cached handle.
    let snapshot = engine.catalog().snapshot();
    let mut q = query();
    q.input.relation = "wide".into();
    q.requirements = vec![requirement(
        "last-field",
        RowOperation::Project {
            field: field("f9999"),
            alias: "last".into(),
        },
    )];
    let result = compile_rows(&engine, q, CompileOptions::default()).await;
    assert_eq!(result.record.work.relations_looked_up, 1);
    assert_eq!(result.record.work.fields_looked_up, 1);
    assert_eq!(result.record.work.context_fields, 0);
    assert_eq!(compiled(result).bound().snapshot_id(), snapshot.id());
}

#[tokio::test]
async fn quoted_identifiers_are_ast_nodes_not_sql_fragments() {
    let name = "odd\"field; --";
    let schema = Arc::new(Schema::new(vec![Field::new(name, DataType::Int64, false)]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![42]))]).unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("items", schema.clone(), "private"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    let mut q = query();
    q.requirements = vec![requirement(
        "quoted",
        RowOperation::Project {
            field: field(name),
            alias: "also\"quoted".into(),
        },
    )];
    both_paths(&engine, q, vec![vec!["42"]]).await;
}

#[tokio::test]
async fn provider_failure_is_distinct_and_does_not_retry() {
    struct Failing;
    impl ModelProvider for Failing {
        async fn complete(&self, _: &[Message]) -> Result<String, ProviderError> {
            Err(ProviderError::Http(401))
        }
    }
    let result = Compiler::new(Failing)
        .compile_typed(&fixture(), "List IDs", CompileOptions::default())
        .await;
    assert!(matches!(
        result.outcome,
        TypedOutcome::ProviderFailure { .. }
    ));
    assert_eq!(result.record.work.model_calls, 1);
    assert_eq!(result.record.outcome, "provider_failure");
}

#[tokio::test]
async fn integer_conjunctions_keep_every_required_predicate() {
    let engine = fixture();
    let mut q = query();
    q.requirements.push(requirement(
        "score",
        RowOperation::Filter {
            predicate: compare("score", Comparison::GtEq, Literal::Int64(20)),
        },
    ));
    q.requirements.push(requirement(
        "id-range",
        RowOperation::Filter {
            predicate: RowPredicate::All {
                predicates: vec![
                    compare("id", Comparison::Gt, Literal::Int64(1)),
                    compare("id", Comparison::Lt, Literal::Int64(4)),
                ],
            },
        },
    ));
    q.requirements.push(requirement(
        "order",
        RowOperation::Order {
            field: field("id"),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        },
    ));
    both_paths(&engine, q, vec![vec!["2"], vec!["3"]]).await;
}

#[tokio::test]
async fn source_ordering_survives_shadowing_and_internal_slot_alias_collisions() {
    let engine = fixture();
    let mut q = query();
    q.requirements = vec![
        requirement(
            "display",
            RowOperation::Project {
                field: field("label"),
                alias: "id".into(),
            },
        ),
        requirement(
            "source-id",
            RowOperation::Project {
                field: field("id"),
                alias: "__semantic_slot_0".into(),
            },
        ),
        requirement(
            "order",
            RowOperation::Order {
                field: field("id"),
                direction: Direction::Desc,
                nulls: NullOrder::Last,
            },
        ),
        requirement("limit", RowOperation::Limit { count: 3 }),
    ];
    both_paths(
        &engine,
        q,
        vec![vec!["B", "5"], vec!["O'Reilly", "4"], vec!["", "3"]],
    )
    .await;
}

#[tokio::test]
async fn exact_decimal_date_and_timestamp_parameters_match_both_backends() {
    use datafusion::common::ScalarValue;
    let literals = vec![
        (Literal::Int16(7), ScalarValue::Int16(Some(7))),
        (Literal::Int32(7), ScalarValue::Int32(Some(7))),
        (
            Literal::Decimal128 {
                coefficient: "12345678901234567890123456789".into(),
                precision: 30,
                scale: 10,
            },
            ScalarValue::Decimal128(Some(12345678901234567890123456789), 30, 10),
        ),
        (Literal::Date32(20000), ScalarValue::Date32(Some(20000))),
        (
            Literal::Timestamp {
                ticks: 1720000000123456,
                unit: TimestampUnit::Microsecond,
                timezone: None,
            },
            ScalarValue::TimestampMicrosecond(Some(1720000000123456), None),
        ),
        (
            Literal::Timestamp {
                ticks: 1720000000123456789,
                unit: TimestampUnit::Nanosecond,
                timezone: Some("UTC".into()),
            },
            ScalarValue::TimestampNanosecond(Some(1720000000123456789), Some("UTC".into())),
        ),
    ];
    for (literal, scalar) in literals {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("value", scalar.data_type(), true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                ScalarValue::iter_to_array([
                    scalar.clone(),
                    ScalarValue::try_new_null(&scalar.data_type()).unwrap(),
                ])
                .unwrap(),
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
        let mut q = query();
        q.requirements.push(requirement(
            "exact filter",
            RowOperation::Filter {
                predicate: compare("value", Comparison::Eq, literal),
            },
        ));
        both_paths(&engine, q, vec![vec!["1"]]).await;
    }
}

#[tokio::test]
async fn decimal_precision_and_timestamp_timezone_are_checked_before_planning() {
    for value in [
        Literal::Decimal128 {
            coefficient: "1.25".into(),
            precision: 3,
            scale: 2,
        },
        Literal::Decimal128 {
            coefficient: "1234".into(),
            precision: 3,
            scale: 2,
        },
        Literal::Decimal128 {
            coefficient: "1".into(),
            precision: 39,
            scale: 2,
        },
        Literal::Decimal128 {
            coefficient: "1".into(),
            precision: 3,
            scale: 4,
        },
        Literal::Timestamp {
            ticks: 0,
            unit: TimestampUnit::Second,
            timezone: Some("Europe/Zurich".into()),
        },
    ] {
        let mut q = query();
        q.requirements.push(requirement(
            "invalid typed literal",
            RowOperation::Filter {
                predicate: compare("id", Comparison::Eq, value),
            },
        ));
        let result = compile_rows(&fixture(), q, CompileOptions::default()).await;
        assert!(!matches!(result.outcome, TypedOutcome::Compiled { .. }));
        assert!(
            result
                .record
                .stages
                .iter()
                .all(|stage| stage.stage != "backend")
        );
    }
}

#[tokio::test]
async fn grouping_counts_distinct_nulls_and_output_order_match_both_backends() {
    let mut q = query();
    q.requirements = vec![
        requirement(
            "group",
            RowOperation::Group {
                field: field("label"),
                alias: "label".into(),
            },
        ),
        requirement(
            "total",
            RowOperation::Aggregate {
                function: AggregateFunction::Sum,
                field: Some(field("score")),
                distinct: false,
                alias: "total".into(),
            },
        ),
        requirement(
            "rows",
            RowOperation::Aggregate {
                function: AggregateFunction::Count,
                field: None,
                distinct: false,
                alias: "rows".into(),
            },
        ),
        requirement(
            "distinct",
            RowOperation::Aggregate {
                function: AggregateFunction::Count,
                field: Some(field("score")),
                distinct: true,
                alias: "distinct".into(),
            },
        ),
        requirement(
            "filter",
            RowOperation::Filter {
                predicate: compare("id", Comparison::LtEq, Literal::Int64(4)),
            },
        ),
        requirement(
            "sort",
            RowOperation::OrderOutput {
                slot: "total".into(),
                direction: Direction::Desc,
                nulls: NullOrder::Last,
            },
        ),
    ];
    both_paths(
        &fixture(),
        q,
        vec![
            vec!["b", "30", "1", "1"],
            vec!["", "20", "1", "1"],
            vec!["O'Reilly", "10", "2", "1"],
        ],
    )
    .await;
}
#[tokio::test]
async fn aggregates_preserve_empty_input_contracts_and_group_only_queries() {
    let mut q = query();
    q.requirements = vec![
        requirement(
            "total",
            RowOperation::Aggregate {
                function: AggregateFunction::Sum,
                field: Some(field("score")),
                distinct: false,
                alias: "total".into(),
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
        requirement(
            "min",
            RowOperation::Aggregate {
                function: AggregateFunction::Min,
                field: Some(field("score")),
                distinct: false,
                alias: "min".into(),
            },
        ),
        requirement(
            "max",
            RowOperation::Aggregate {
                function: AggregateFunction::Max,
                field: Some(field("score")),
                distinct: false,
                alias: "max".into(),
            },
        ),
        requirement(
            "filter",
            RowOperation::Filter {
                predicate: compare("id", Comparison::Lt, Literal::Int64(0)),
            },
        ),
    ];
    both_paths(&fixture(), q, vec![vec!["", "0", "", ""]]).await;
    let mut q = query();
    q.requirements = vec![requirement(
        "rows",
        RowOperation::Aggregate {
            function: AggregateFunction::Count,
            field: None,
            distinct: false,
            alias: "rows".into(),
        },
    )];
    both_paths(&fixture(), q, vec![vec!["5"]]).await;
    let mut q = query();
    q.requirements = vec![
        requirement(
            "group",
            RowOperation::Group {
                field: field("active"),
                alias: "active".into(),
            },
        ),
        requirement(
            "sort",
            RowOperation::OrderOutput {
                slot: "group".into(),
                direction: Direction::Asc,
                nulls: NullOrder::Last,
            },
        ),
    ];
    both_paths(&fixture(), q, vec![vec!["false"], vec!["true"], vec![""]]).await;
}
#[tokio::test]
async fn aggregate_grain_and_output_scope_fail_closed() {
    let aggregate = || {
        requirement(
            "count",
            RowOperation::Aggregate {
                function: AggregateFunction::Count,
                field: None,
                distinct: false,
                alias: "count".into(),
            },
        )
    };
    for extra in [
        requirement(
            "ungrouped",
            RowOperation::Project {
                field: field("label"),
                alias: "label".into(),
            },
        ),
        requirement(
            "bad order",
            RowOperation::Order {
                field: field("id"),
                direction: Direction::Asc,
                nulls: NullOrder::Last,
            },
        ),
        requirement(
            "bad slot",
            RowOperation::OrderOutput {
                slot: "count_alias".into(),
                direction: Direction::Asc,
                nulls: NullOrder::Last,
            },
        ),
        requirement(
            "bad distinct",
            RowOperation::Aggregate {
                function: AggregateFunction::Count,
                field: None,
                distinct: true,
                alias: "distinct".into(),
            },
        ),
        requirement(
            "bad sum",
            RowOperation::Aggregate {
                function: AggregateFunction::Sum,
                field: Some(field("label")),
                distinct: false,
                alias: "sum".into(),
            },
        ),
    ] {
        let mut q = query();
        q.requirements = vec![aggregate(), extra];
        assert!(!matches!(
            compile_rows(&fixture(), q, CompileOptions::default())
                .await
                .outcome,
            TypedOutcome::Compiled { .. }
        ));
    }
}

async fn governed_fixture() -> Engine {
    governed_fixture_rollup(None).await
}
async fn governed_fixture_rollup(rollup: Option<std::collections::BTreeSet<String>>) -> Engine {
    governed_fixture_with_competitor(rollup, None).await
}
async fn governed_fixture_with_competitor(
    rollup: Option<std::collections::BTreeSet<String>>,
    competitor_unit: Option<&str>,
) -> Engine {
    use semantic_catalog::{
        EmptyBehavior, GovernedFilter, MetricDefinition, Presence, RelationSemantics, RowPolicy,
    };
    let mut engine = fixture();
    let mut relation = engine.catalog().relation("items").unwrap().clone();
    relation.name = "governed".into();
    relation.semantics = Some(RelationSemantics {
        metrics: [(
            "qualified_score".into(),
            MetricDefinition {
                id: "metrics/qualified-score".into(),
                description: "Scores of active items".into(),
                aliases: vec!["qualified points".into()],
                function: AggregateFunction::Sum,
                field: Some("score".into()),
                distinct: false,
                source_grain: vec!["id".into()],
                compatible_dimensions: ["label".into()].into(),
                compatible_lookup_dimensions: vec![],
                sum_rollup_dimensions: rollup.clone(),
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
            id: "policies/scope".into(),
            filters: vec![GovernedFilter {
                field: "id".into(),
                operator: Comparison::LtEq,
                value: Literal::Int64(4),
            }],
            source_refs: vec![],
        }],
        ..Default::default()
    });
    let semantics = relation.semantics.as_mut().unwrap();
    semantics.value_mappings.insert(
        "labels".into(),
        semantic_catalog::ValueMapping {
            id: "dictionaries/labels".into(),
            field: "label".into(),
            description: "Authored label aliases".into(),
            codes: [
                ("publisher".into(), "O'Reilly".into()),
                ("lowercase bee".into(), "b".into()),
            ]
            .into(),
            source_refs: vec![],
        },
    );
    semantics.metrics.insert(
        "visible_rows".into(),
        MetricDefinition {
            id: "metrics/visible-rows".into(),
            description: "Visible row count".into(),
            aliases: vec![],
            function: AggregateFunction::Count,
            field: None,
            distinct: false,
            source_grain: vec!["id".into()],
            compatible_dimensions: ["label".into()].into(),
            compatible_lookup_dimensions: vec![],
            sum_rollup_dimensions: rollup.clone(),
            row_filters: vec![],
            result_type: DataType::Int64,
            unit: Presence::Value("rows".into()),
            temporal: Presence::Missing,
            empty_behavior: EmptyBehavior::Zero,
            source_refs: vec![],
        },
    );
    semantics.ratio_metrics.insert(
        "qualified_average".into(),
        semantic_catalog::RatioDefinition {
            id: "metrics/qualified-average".into(),
            description: "Qualified points per visible row".into(),
            aliases: vec!["qualified average".into()],
            numerator: "qualified_score".into(),
            denominator: "visible_rows".into(),
            zero: ZeroDivision::Null,
            unit: Presence::Value("points/row".into()),
            source_refs: vec![],
        },
    );
    if let Some(unit) = competitor_unit {
        semantics.metrics.insert(
            "qualified_count".into(),
            MetricDefinition {
                id: "metrics/qualified-count".into(),
                description: "Count with a deliberately competing authored label".into(),
                aliases: vec!["qualified points".into()],
                function: AggregateFunction::Count,
                field: None,
                distinct: false,
                source_grain: vec!["id".into()],
                compatible_dimensions: ["label".into()].into(),
                compatible_lookup_dimensions: vec![],
                sum_rollup_dimensions: rollup,
                row_filters: vec![],
                result_type: DataType::Int64,
                unit: Presence::Value(unit.into()),
                temporal: Presence::Missing,
                empty_behavior: EmptyBehavior::Zero,
                source_refs: vec![],
            },
        );
    }
    let provider = engine
        .plan_generated_sql("SELECT * FROM items")
        .await
        .unwrap()
        .into_view();
    engine.register_table(relation, provider).unwrap();
    engine
}

async fn governed_fixture_with_duplicate_identities() -> Engine {
    let mut engine = governed_fixture().await;
    let mut duplicate = engine.catalog().relation("governed").unwrap().clone();
    duplicate.name = "other_governed".into();
    let provider = engine
        .plan_generated_sql("SELECT * FROM items")
        .await
        .unwrap()
        .into_view();
    engine.register_table(duplicate, provider).unwrap();
    engine
}

fn temporal_metric_fixture() -> Engine {
    use semantic_catalog::{
        EmptyBehavior, MetricDefinition, MetricTemporalApplicability, Presence, RelationSemantics,
    };
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("day", DataType::Date32, false),
        Field::new("score", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Date32Array::from(vec![19737, 19763, 19792])),
            Arc::new(Int64Array::from(vec![1, 2, 4])),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("temporal_scores", schema.clone(), "memory");
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
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn temporal_metric_query(unit: CalendarUnit) -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "temporal_scores".into(),
            instance: "r".into(),
        },
        requirements: vec![
            requirement(
                "period",
                RowOperation::CalendarFilter {
                    field: field("day"),
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
                    },
                },
            ),
        ],
        unresolved: vec![],
    }
}

fn temporal_options(reference: &str) -> CompileOptions {
    use semantic_compiler::typed::{Calendar, ContextOrigin, RequestContext};
    let mut options = CompileOptions::default();
    options.request_context = Some(RequestContext {
        reference_unix_millis: chrono::DateTime::parse_from_rfc3339(reference)
            .unwrap()
            .timestamp_millis(),
        timezone: "UTC".into(),
        calendar: Calendar::Gregorian,
        origin: ContextOrigin::Caller,
    });
    options
}

#[tokio::test]
async fn metric_applicability_enforces_exact_unit_grain_and_temporal_coverage() {
    let engine = temporal_metric_fixture();
    let valid = temporal_metric_query(CalendarUnit::Month);
    let result = compile_rows(
        &engine,
        valid.clone(),
        temporal_options("2024-03-15T12:00:00Z"),
    )
    .await;
    let artifact = compiled(result);
    assert_eq!(
        values(
            &artifact
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()
        ),
        vec![vec!["2".to_string()]]
    );

    let mut cases = Vec::new();
    cases.push((
        "metric_time_grain",
        temporal_metric_query(CalendarUnit::Day),
        temporal_options("2024-02-10T12:00:00Z"),
    ));
    cases.push((
        "metric_coverage",
        valid.clone(),
        temporal_options("2024-05-15T12:00:00Z"),
    ));
    let mut missing_filter = valid.clone();
    missing_filter.requirements.remove(0);
    cases.push((
        "metric_applicability",
        missing_filter,
        CompileOptions::default(),
    ));
    let mut wrong_unit = valid.clone();
    let RowOperation::Metric { applicability, .. } = &mut wrong_unit.requirements[1].operation
    else {
        unreachable!()
    };
    applicability.required_unit = Some("euros".into());
    cases.push((
        "metric_unit",
        wrong_unit,
        temporal_options("2024-03-15T12:00:00Z"),
    ));
    let mut wrong_grain = valid;
    let RowOperation::Metric { applicability, .. } = &mut wrong_grain.requirements[1].operation
    else {
        unreachable!()
    };
    applicability.required_source_grain = vec!["day".into()];
    cases.push((
        "metric_grain",
        wrong_grain,
        temporal_options("2024-03-15T12:00:00Z"),
    ));
    for (code, query, options) in cases {
        assert!(
            matches!(compile_rows(&engine, query, options).await.outcome,
                TypedOutcome::Rejected { diagnostic } if diagnostic.code == code),
            "expected {code}"
        );
    }
}

#[tokio::test]
async fn competing_metric_labels_require_identity_or_unique_exact_applicability() {
    fn alias_query(unit: Option<&str>) -> RowQuery {
        let mut metric = requirement(
            "metric",
            RowOperation::Metric {
                name: "qualified points".into(),
                alias: "value".into(),
                applicability: MetricApplicability {
                    required_unit: unit.map(str::to_owned),
                    required_source_grain: vec!["id".into()],
                },
            },
        );
        metric.source_text = "qualified points".into();
        RowQuery {
            input: RelationInput {
                relation: "governed".into(),
                instance: "r".into(),
            },
            requirements: vec![metric],
            ..query()
        }
    }

    let engine = governed_fixture_with_competitor(None, Some("rows")).await;
    assert!(
        matches!(compile_rows(&engine, alias_query(None), CompileOptions::default()).await.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "ambiguous_metric")
    );
    let mut model_selected_canonical_name = alias_query(None);
    let RowOperation::Metric { name, .. } =
        &mut model_selected_canonical_name.requirements[0].operation
    else {
        unreachable!()
    };
    *name = "qualified_score".into();
    assert!(
        matches!(compile_rows(&engine, model_selected_canonical_name, CompileOptions::default()).await.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "ambiguous_metric")
    );
    both_paths(&engine, alias_query(Some("points")), vec![vec!["10"]]).await;

    let mut by_identity = alias_query(None);
    let RowOperation::Metric { name, .. } = &mut by_identity.requirements[0].operation else {
        unreachable!()
    };
    *name = "metrics/qualified-score".into();
    by_identity.requirements[0].source_text = "metrics/qualified-score".into();
    both_paths(&engine, by_identity, vec![vec!["10"]]).await;

    assert!(
        matches!(compile_rows(&engine, alias_query(Some("unknown")), CompileOptions::default()).await.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_applicability")
    );

    // Mutation check: changing the competing contract to the requested unit
    // invalidates the formerly unique applicability decision.
    let mutated = governed_fixture_with_competitor(None, Some("points")).await;
    assert!(
        matches!(compile_rows(&mutated, alias_query(Some("points")), CompileOptions::default()).await.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "ambiguous_metric")
    );

    let duplicate_identities = governed_fixture_with_duplicate_identities().await;
    let mut duplicate_identity = alias_query(None);
    let RowOperation::Metric { name, .. } = &mut duplicate_identity.requirements[0].operation
    else {
        unreachable!()
    };
    *name = "metrics/qualified-score".into();
    duplicate_identity.requirements[0].source_text = "metric".into();
    assert!(
        matches!(compile_rows(&duplicate_identities, duplicate_identity, CompileOptions::default()).await.outcome,
            TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_identity")
    );
}

#[tokio::test]
async fn governed_metric_filters_are_local_and_policies_are_unavoidable() {
    let mut q = query();
    q.input.relation = "governed".into();
    q.requirements = vec![
        requirement(
            "metric",
            RowOperation::Metric {
                name: "qualified_score".into(),
                alias: "qualified".into(),
                applicability: Default::default(),
            },
        ),
        requirement(
            "all",
            RowOperation::Aggregate {
                function: AggregateFunction::Sum,
                field: Some(field("score")),
                distinct: false,
                alias: "all".into(),
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
    let engine = governed_fixture().await;
    both_paths(&engine, q.clone(), vec![vec!["10", "60", "4"]]).await;
    let artifact = compiled(compile_rows(&engine, q.clone(), CompileOptions::default()).await);
    let serialized = serde_json::to_value(artifact.bound()).unwrap();
    assert!(
        serialized["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|definition| definition["id"] == "functions/semantic_sum_v1")
    );
    assert_eq!(serialized["definitions"].as_array().unwrap().len(), 3);
    q.requirements.push(requirement(
        "incompatible",
        RowOperation::Group {
            field: field("active"),
            alias: "active".into(),
        },
    ));
    assert!(!matches!(
        compile_rows(&engine, q, CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::Compiled { .. }
    ));
}

fn related_fixture(null_keys_match: bool) -> Engine {
    use semantic_catalog::{
        FactResolution, RelationSemantics, RelationshipDefinition, RelationshipKey,
    };
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("score", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![Some(1), Some(2), Some(3), None])),
            Arc::new(Int64Array::from(vec![10, 20, 30, 40])),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("items", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        relationships: [(
            "orders".into(),
            RelationshipDefinition {
                ai_context: None,
                id: "relationships/customer-orders".into(),
                right_relation: "orders".into(),
                role: "purchaser".into(),
                key_pairs: vec![RelationshipKey {
                    left_field: "id".into(),
                    right_field: "customer_id".into(),
                }],
                null_keys_match,
                cardinality: FactResolution::Unknown,
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
    let schema = Arc::new(Schema::new(vec![
        Field::new("customer_id", DataType::Int64, true),
        Field::new("paid", DataType::Boolean, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![Some(1), Some(1), Some(2), None])),
            Arc::new(BooleanArray::from(vec![true, false, false, true])),
        ],
    )
    .unwrap();
    engine
        .register_table(
            Relation::base("orders", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}
fn related(mode: ExistenceMode, predicate: Option<RowPredicate>) -> Requirement {
    requirement(
        "related",
        RowOperation::Related {
            relationship: "orders".into(),
            role: "purchaser".into(),
            instance: "o".into(),
            mode,
            predicate,
        },
    )
}
#[tokio::test]
async fn existence_absence_and_nullable_keys_preserve_left_multiplicity() {
    for (null_safe, mode, expected) in [
        (false, ExistenceMode::Exists, vec![vec!["1"], vec!["2"]]),
        (false, ExistenceMode::Absent, vec![vec!["3"], vec![""]]),
        (
            true,
            ExistenceMode::Exists,
            vec![vec!["1"], vec!["2"], vec![""]],
        ),
        (true, ExistenceMode::Absent, vec![vec!["3"]]),
    ] {
        let mut q = query();
        q.requirements.push(related(mode, None));
        q.requirements.push(requirement(
            "sort",
            RowOperation::Order {
                field: field("id"),
                direction: Direction::Asc,
                nulls: NullOrder::Last,
            },
        ));
        both_paths(&related_fixture(null_safe), q, expected).await;
    }
    let mut q = query();
    q.requirements = vec![
        requirement(
            "sum",
            RowOperation::Aggregate {
                function: AggregateFunction::Sum,
                field: Some(field("score")),
                distinct: false,
                alias: "sum".into(),
            },
        ),
        related(ExistenceMode::Exists, None),
    ];
    both_paths(&related_fixture(false), q, vec![vec!["30"]]).await;
}
#[tokio::test]
async fn related_predicates_use_their_occurrence_and_roles_never_fall_back() {
    let mut q = query();
    q.requirements.push(related(
        ExistenceMode::Exists,
        Some(RowPredicate::Not {
            predicate: Box::new(RowPredicate::Compare {
                field: FieldRef {
                    instance: "o".into(),
                    field: "paid".into(),
                },
                operator: Comparison::Eq,
                value: Literal::Boolean(false),
            }),
        }),
    ));
    both_paths(&related_fixture(false), q.clone(), vec![vec!["1"]]).await;
    let RowOperation::Related { role, .. } = &mut q.requirements[1].operation else {
        unreachable!()
    };
    *role = "shipping".into();
    assert!(!matches!(
        compile_rows(
            &related_fixture(false),
            q.clone(),
            CompileOptions::default()
        )
        .await
        .outcome,
        TypedOutcome::Compiled { .. }
    ));
    let RowOperation::Related { role, .. } = &mut q.requirements[1].operation else {
        unreachable!()
    };
    *role = "purchaser".into();
    let mut options = CompileOptions::default();
    options.allowed_relations = Some(["items".into()].into());
    assert!(!matches!(
        compile_rows(&related_fixture(false), q, options)
            .await
            .outcome,
        TypedOutcome::Compiled { .. }
    ));
}

#[tokio::test]
async fn replay_is_bounded_revalidates_and_checks_pipeline_and_artifact_identity() {
    let engine = fixture();
    let compilation = compile_rows(&engine, query(), CompileOptions::default()).await;
    assert!(!compilation.record.compilation_id.is_empty());
    assert!(compilation.record.bound_digest.is_some());
    assert!(compilation.record.relational_digest.is_some());
    assert_eq!(
        compilation.record.requirement_dispositions[0].result,
        "lowered_and_verified"
    );
    let artifact = compiled(compilation);
    assert_eq!(
        artifact.capture_replay(1).unwrap_err().code,
        "capture_limit"
    );
    let bundle = artifact.capture_replay(64 * 1024).unwrap();
    let mut restored: semantic_compiler::typed::ReplayBundle =
        serde_json::from_slice(&serde_json::to_vec(&bundle).unwrap()).unwrap();
    let replay = restored
        .replay(&engine, CompileOptions::default())
        .await
        .unwrap();
    assert_eq!(
        replay.record.artifact_digest.as_deref(),
        Some(bundle.artifact_digest.as_str())
    );
    restored.proposal.requirements[0].source_text = "different requirement".into();
    assert_eq!(
        restored
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_mismatch"
    );
    restored.proposal.input.relation = "unknown".into();
    assert!(matches!(
        restored
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap()
            .outcome,
        TypedOutcome::Rejected { .. }
    ));
    restored.execution_profile_revision = "unavailable".into();
    assert_eq!(
        restored
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_profile"
    );
    restored.execution_profile_revision = semantic_engine::MVP_EXECUTION_PROFILE_REVISION.into();
    restored.pipeline_revision = "unavailable".into();
    assert_eq!(
        restored
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_version"
    );
}

#[tokio::test]
async fn scope_restricted_artifacts_require_current_execution_authorization() {
    use semantic_plan::graph::{GraphOperation, GraphQuery, QueryNode};

    let engine = fixture();
    let allowed = std::collections::BTreeSet::from(["items".to_string()]);
    let mut options = CompileOptions::default();
    options.allowed_relations = Some(allowed.clone());
    let rows = compiled(compile_rows(&engine, query(), options.clone()).await);
    assert_eq!(
        rows.execution_profile_revision(),
        semantic_engine::MVP_EXECUTION_PROFILE_REVISION
    );
    assert_eq!(
        rows.plan_direct(&engine).await.unwrap_err().code,
        "execution_scope"
    );
    rows.plan_direct_authorized(&engine, &allowed)
        .await
        .unwrap();
    assert_eq!(
        rows.plan_direct_authorized(&engine, &Default::default())
            .await
            .unwrap_err()
            .code,
        "execution_scope"
    );

    let graph = GraphQuery {
        version: 1,
        nodes: vec![QueryNode {
            id: "items".into(),
            source_text: "item IDs".into(),
            operation: GraphOperation::Rows { query: query() },
        }],
        root: "items".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    };
    let result = semantic_compiler::typed::compile_graph(&engine, graph, options).await;
    let TypedOutcome::CompiledGraph { query: graph } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    assert_eq!(
        graph.execution_profile_revision(),
        semantic_engine::MVP_EXECUTION_PROFILE_REVISION
    );
    assert_eq!(
        graph.plan_direct(&engine).await.unwrap_err().code,
        "execution_scope"
    );
    graph
        .plan_direct_authorized(&engine, &allowed)
        .await
        .unwrap();
    assert_eq!(
        graph
            .plan_direct_authorized(&engine, &Default::default())
            .await
            .unwrap_err()
            .code,
        "execution_scope"
    );
}

#[tokio::test]
async fn normal_records_digest_model_supplied_requirement_identities() {
    let mut q = query();
    q.requirements[0].id = "private-client-requirement-7f3c".into();
    let result = compile_rows(&fixture(), q, CompileOptions::default()).await;
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
    assert_eq!(
        result.record.execution_profile_revision,
        semantic_engine::MVP_EXECUTION_PROFILE_REVISION
    );
    assert_eq!(result.record.requirement_dispositions.len(), 1);
    assert!(
        result.record.requirement_dispositions[0]
            .requirement_id
            .starts_with("requirement/")
    );
    assert!(
        !serde_json::to_string(&result.record)
            .unwrap()
            .contains("private-client-requirement-7f3c")
    );
}

#[tokio::test]
async fn provider_attempts_account_for_reported_and_unknown_usage() {
    use semantic_compiler::provider::{
        CompletionMetadata, CompletionStatus, ModelCompletion, TokenUsage,
    };
    struct Reported;
    impl ModelProvider for Reported {
        async fn complete(&self, _: &[Message]) -> Result<String, ProviderError> {
            unreachable!("typed path uses envelope")
        }
        async fn complete_envelope(&self, _: &[Message]) -> Result<ModelCompletion, ProviderError> {
            Ok(ModelCompletion {
                text: Some("refusal text is private".into()),
                status: CompletionStatus::Refused,
                metadata: CompletionMetadata {
                    usage: TokenUsage {
                        input_tokens: Some(100),
                        output_tokens: Some(5),
                        cached_input_tokens: Some(40),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            })
        }
    }
    let result = Compiler::new(Reported)
        .compile_typed(&fixture(), "IDs", CompileOptions::default())
        .await;
    assert!(matches!(
        result.outcome,
        TypedOutcome::ProviderFailure { .. }
    ));
    assert_eq!(
        result.record.model_attempts[0].status,
        Some(CompletionStatus::Refused)
    );
    assert_eq!(result.record.token_accounting.reported_input_tokens, 100);
    assert_eq!(
        result.record.token_accounting.reported_cached_input_tokens,
        40
    );
    assert_eq!(result.record.token_accounting.calls_missing_output_usage, 0);
    assert!(
        !serde_json::to_string(&result.record)
            .unwrap()
            .contains("refusal text")
    );
}

#[tokio::test]
async fn compilation_cache_is_bounded_scoped_and_coalesces_identical_work() {
    use semantic_compiler::typed::{CompilationCacheOptions, CompilationSession};
    let engine = fixture();
    let session = CompilationSession::new(
        &engine,
        CompilationCacheOptions {
            max_entries: 1,
            max_bytes: 1024 * 1024,
            max_concurrent: 2,
        },
    )
    .unwrap();
    let (first, second) = tokio::join!(
        session.compile(query(), CompileOptions::default()),
        session.compile(query(), CompileOptions::default())
    );
    assert!(matches!(first.outcome, TypedOutcome::Compiled { .. }));
    assert!(matches!(second.outcome, TypedOutcome::Compiled { .. }));
    assert_eq!(first.record.artifact_digest, second.record.artifact_digest);
    assert_eq!(session.stats().misses, 1);
    assert_eq!(session.stats().hits, 1);
    assert_ne!(first.record.compilation_id, second.record.compilation_id);
    let mut scoped = CompileOptions::default();
    scoped.allowed_relations = Some(Default::default());
    assert!(matches!(
        session.compile(query(), scoped).await.outcome,
        TypedOutcome::Rejected { .. }
    ));
    let mut changed = query();
    changed.requirements[0].source_text = "alternate request".into();
    assert!(matches!(
        session
            .compile(changed, CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::Compiled { .. }
    ));
    assert_eq!(session.stats().evictions, 1);
    assert_eq!(session.stats().entries, 1);
    assert!(session.stats().retained_bytes <= 1024 * 1024);
    let options = CompileOptions::default();
    options.cancellation.cancel();
    assert!(matches!(
        session.compile(query(), options).await.outcome,
        TypedOutcome::Unresolved { .. }
    ));
}
#[tokio::test]
async fn cache_retention_failure_does_not_change_semantic_outcome() {
    use semantic_compiler::typed::{CompilationCacheOptions, CompilationSession};
    let engine = fixture();
    let session = CompilationSession::new(
        &engine,
        CompilationCacheOptions {
            max_entries: 1,
            max_bytes: 1,
            max_concurrent: 1,
        },
    )
    .unwrap();
    let result = session.compile(query(), CompileOptions::default()).await;
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
    assert_eq!(session.stats().entries, 0);
    assert_eq!(
        result.record.cache_status,
        "miss_artifact_exceeds_retention_budget"
    );
}

#[tokio::test]
async fn ratios_aggregate_before_dividing_with_declared_precision_and_zero_behavior() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("label", DataType::Utf8, false),
        Field::new("n", DataType::Int64, true),
        Field::new("d", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["a", "a", "b", "c"])),
            Arc::new(Int64Array::from(vec![Some(1), Some(0), Some(10), None])),
            Arc::new(Int64Array::from(vec![1, 2, 0, 0])),
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
    for zero in [ZeroDivision::Null, ZeroDivision::Zero] {
        let mut q = query();
        q.requirements = vec![
            requirement(
                "group",
                RowOperation::Group {
                    field: field("label"),
                    alias: "label".into(),
                },
            ),
            requirement(
                "ratio",
                RowOperation::Ratio {
                    numerator: AggregateOperand {
                        function: AggregateFunction::Sum,
                        field: Some(field("n")),
                        distinct: false,
                    },
                    denominator: AggregateOperand {
                        function: AggregateFunction::Sum,
                        field: Some(field("d")),
                        distinct: false,
                    },
                    zero,
                    alias: "ratio".into(),
                },
            ),
            requirement(
                "order",
                RowOperation::OrderOutput {
                    slot: "ratio".into(),
                    direction: Direction::Desc,
                    nulls: NullOrder::Last,
                },
            ),
            requirement(
                "tie",
                RowOperation::OrderOutput {
                    slot: "group".into(),
                    direction: Direction::Asc,
                    nulls: NullOrder::Last,
                },
            ),
        ];
        both_paths(
            &engine,
            q,
            vec![
                vec!["a", "0.333333333333333333"],
                vec![
                    "b",
                    if zero == ZeroDivision::Null {
                        ""
                    } else {
                        "0.000000000000000000"
                    },
                ],
                vec!["c", ""],
            ],
        )
        .await;
    }
    let mut q = query();
    q.requirements = vec![requirement(
        "average",
        RowOperation::Ratio {
            numerator: AggregateOperand {
                function: AggregateFunction::Sum,
                field: Some(field("score")),
                distinct: false,
            },
            denominator: AggregateOperand {
                function: AggregateFunction::Count,
                field: Some(field("score")),
                distinct: false,
            },
            zero: ZeroDivision::Null,
            alias: "average".into(),
        },
    )];
    both_paths(&fixture(), q, vec![vec!["17.500000000000000000"]]).await;
}

#[tokio::test]
async fn governed_ratios_retain_component_contracts_and_retrieval_closure() {
    use semantic_compiler::typed::SelectionMode;
    let engine = governed_fixture().await;
    let mut q = query();
    q.input.relation = "governed".into();
    q.requirements = vec![requirement(
        "ratio",
        RowOperation::Metric {
            name: "qualified_average".into(),
            alias: "average".into(),
            applicability: Default::default(),
        },
    )];
    both_paths(&engine, q.clone(), vec![vec!["2.500000000000000000"]]).await;
    let mut options = CompileOptions::default();
    options.selection_mode = SelectionMode::Retrieved;
    options.small_relation_fields = 0;
    options.initial_fields_per_relation = 0;
    let result = compiler(vec![json!({"status":"query","query":q}).to_string()])
        .compile_typed(&engine, "qualified average", options)
        .await;
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
    let selected = result.record.contexts[0]
        .included
        .iter()
        .find(|r| r.reference.id == "governed")
        .unwrap();
    assert_eq!(selected.fields, vec!["active", "id", "score"]);
    assert!(!selected.field_inventory_complete);
    assert!(
        result
            .record
            .definition_refs
            .iter()
            .any(|r| r.id == "metrics/qualified-average")
    );
    assert!(
        result
            .record
            .definition_refs
            .iter()
            .any(|r| r.id == "functions/semantic_ratio_i64_v1")
    );
}

fn window_order(input: WindowInput, direction: Direction) -> WindowOrder {
    WindowOrder {
        input,
        direction,
        nulls: NullOrder::Last,
    }
}
fn window_field(name: &str) -> WindowInput {
    WindowInput::Field { field: field(name) }
}
#[tokio::test]
async fn windows_preserve_peer_groups_nulls_and_partition_contracts() {
    let engine = fixture();
    let mut q = query();
    for (id, function, input, frame, partition_by) in [
        (
            "rank",
            WindowFunction::Rank,
            None,
            WindowFrame::ThroughCurrentPeer,
            vec![],
        ),
        (
            "dense",
            WindowFunction::DenseRank,
            None,
            WindowFrame::ThroughCurrentPeer,
            vec![],
        ),
        (
            "running",
            WindowFunction::Sum,
            Some(window_field("score")),
            WindowFrame::ThroughCurrentPeer,
            vec![],
        ),
        (
            "partition_count",
            WindowFunction::Count,
            None,
            WindowFrame::EntirePartition,
            vec![window_field("active")],
        ),
    ] {
        q.requirements.push(requirement(
            id,
            RowOperation::Window {
                window: WindowSpec {
                    function,
                    input,
                    frame,
                    partition_by,
                    order_by: vec![window_order(window_field("score"), Direction::Asc)],
                },
                alias: id.into(),
            },
        ));
    }
    q.requirements.push(requirement(
        "sort",
        RowOperation::Order {
            field: field("id"),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        },
    ));
    both_paths(
        &engine,
        q,
        vec![
            vec!["1", "1", "1", "20", "3"],
            vec!["2", "4", "3", "70", "1"],
            vec!["3", "3", "2", "40", "1"],
            vec!["4", "5", "4", "70", "3"],
            vec!["5", "1", "1", "20", "3"],
        ],
    )
    .await;
}

#[tokio::test]
async fn windows_run_after_aggregation_and_before_final_fetch() {
    let engine = fixture();
    let q = RowQuery {
        requirements: vec![
            requirement(
                "label",
                RowOperation::Group {
                    field: field("label"),
                    alias: "label".into(),
                },
            ),
            requirement(
                "total",
                RowOperation::Aggregate {
                    function: AggregateFunction::Sum,
                    field: Some(field("score")),
                    distinct: false,
                    alias: "total".into(),
                },
            ),
            requirement(
                "rank",
                RowOperation::Window {
                    window: WindowSpec {
                        function: WindowFunction::Rank,
                        input: None,
                        partition_by: vec![],
                        order_by: vec![window_order(
                            WindowInput::Output {
                                slot: "total".into(),
                            },
                            Direction::Desc,
                        )],
                        frame: WindowFrame::ThroughCurrentPeer,
                    },
                    alias: "rank".into(),
                },
            ),
            requirement(
                "sort",
                RowOperation::OrderOutput {
                    slot: "rank".into(),
                    direction: Direction::Asc,
                    nulls: NullOrder::Last,
                },
            ),
            requirement("limit", RowOperation::Limit { count: 2 }),
        ],
        ..query()
    };
    both_paths(
        &engine,
        q.clone(),
        vec![vec!["b", "30", "1"], vec!["", "20", "2"]],
    )
    .await;
    let mut invalid = q;
    if let RowOperation::Window { window, .. } = &mut invalid.requirements[2].operation {
        window.order_by[0].input = window_field("score");
    }
    let result = compile_rows(&engine, invalid, CompileOptions::default()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "window_grain")
    );
}

#[tokio::test]
async fn windows_reject_undefined_order_nesting_and_cross_grain_inputs() {
    let engine = fixture();
    for (function, input, order_by, code) in [
        (WindowFunction::Rank, None, vec![], "window_order"),
        (
            WindowFunction::Sum,
            Some(WindowInput::Output { slot: "ids".into() }),
            vec![window_order(window_field("id"), Direction::Asc)],
            "window_grain",
        ),
        (
            WindowFunction::Rank,
            Some(window_field("score")),
            vec![window_order(window_field("id"), Direction::Asc)],
            "window_contract",
        ),
    ] {
        let mut q = query();
        q.requirements.push(requirement(
            "w",
            RowOperation::Window {
                window: WindowSpec {
                    function,
                    input,
                    partition_by: vec![],
                    order_by,
                    frame: WindowFrame::ThroughCurrentPeer,
                },
                alias: "w".into(),
            },
        ));
        let result = compile_rows(&engine, q, CompileOptions::default()).await;
        assert!(
            matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == code)
        );
    }
}

#[tokio::test]
async fn calendar_filters_pin_context_use_half_open_bounds_and_replay_across_clock_changes() {
    use datafusion::arrow::{array::TimestampMillisecondArray, datatypes::TimeUnit};
    use semantic_compiler::typed::{
        Calendar, CompilationCacheOptions, CompilationSession, ContextOrigin, RequestContext,
    };
    // Zurich's 2024 spring transition: March 31 is a 23-hour local day.
    let start = 1711839600000i64;
    let end = start + 23 * 60 * 60 * 1000;
    let schema = Arc::new(Schema::new(vec![Field::new(
        "at",
        DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
        true,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
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
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("events", schema.clone(), "local"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    let q = RowQuery {
        version: 1,
        input: RelationInput {
            relation: "events".into(),
            instance: "r".into(),
        },
        requirements: vec![
            requirement(
                "today",
                RowOperation::CalendarFilter {
                    field: field("at"),
                    period: CalendarPeriod {
                        unit: CalendarUnit::Day,
                        offset: 0,
                        count: 1,
                    },
                },
            ),
            requirement(
                "n",
                RowOperation::Aggregate {
                    function: AggregateFunction::Count,
                    field: None,
                    distinct: false,
                    alias: "n".into(),
                },
            ),
        ],
        unresolved: vec![],
    };
    let mut options = CompileOptions::default();
    options.request_context = Some(RequestContext {
        reference_unix_millis: start + 12 * 60 * 60 * 1000,
        timezone: "Europe/Zurich".into(),
        calendar: Calendar::Gregorian,
        origin: ContextOrigin::Caller,
    });
    let session = CompilationSession::new(&engine, CompilationCacheOptions::default()).unwrap();
    let first = compiled(session.compile(q.clone(), options.clone()).await);
    let direct = first
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let emitted = first
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(values(&direct), vec![vec!["2"]]);
    assert_eq!(values(&emitted), values(&direct));
    assert_eq!(
        first.sql().parameters(),
        &[
            Literal::Timestamp {
                ticks: start,
                unit: TimestampUnit::Millisecond,
                timezone: Some("UTC".into())
            },
            Literal::Timestamp {
                ticks: end,
                unit: TimestampUnit::Millisecond,
                timezone: Some("UTC".into())
            }
        ]
    );
    let capture = first.capture_replay(64 * 1024).unwrap();
    let replay = capture
        .replay(&engine, CompileOptions::default())
        .await
        .unwrap();
    assert!(matches!(replay.outcome, TypedOutcome::Compiled { .. }));
    options
        .request_context
        .as_mut()
        .unwrap()
        .reference_unix_millis = end + 1000;
    assert_eq!(
        capture
            .replay(&engine, options.clone())
            .await
            .unwrap_err()
            .code,
        "replay_context"
    );
    let next = compiled(session.compile(q.clone(), options).await);
    assert_ne!(first.sql().parameters(), next.sql().parameters());
    assert_eq!(session.stats().misses, 2);
    let missing = compile_rows(&engine, q, CompileOptions::default()).await;
    assert!(
        matches!(missing.outcome, TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "unresolved_time_context")
    );
}

fn traced_intent() -> IntentQuery {
    let mut q = query();
    q.requirements[0].source_text = "IDs".into();
    IntentQuery {
        query: q,
        evidence: RequestEvidence {
            version: 1,
            request_id: "request-1".into(),
            original_request: "Montrér IDs".into(),
            requirement_spans: [("ids".into(), vec![RequestSpan { start: 9, end: 12 }])].into(),
            unresolved_alternatives: vec![],
        },
    }
}
#[tokio::test]
async fn intent_retains_exact_request_and_validates_utf8_spans_without_claiming_completeness() {
    use semantic_compiler::typed::compile_intent;
    let engine = fixture();
    let intent = traced_intent();
    let result = compile_intent(&engine, intent.clone(), CompileOptions::default()).await;
    assert!(result.record.request_spans_validated);
    assert!(result.record.request_digest.is_some());
    assert!(result.record.relational_digest.is_some());
    let record = serde_json::to_string(&result.record).unwrap();
    assert!(!record.contains("Montrér"));
    let artifact = compiled(result);
    let replay = artifact.capture_replay(64 * 1024).unwrap();
    assert_eq!(replay.request_evidence.as_ref().unwrap(), &intent.evidence);
    assert!(
        replay
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap()
            .record
            .request_spans_validated
    );
    for (code, evidence) in [
        ("request_span", {
            let mut e = intent.evidence.clone();
            e.requirement_spans.get_mut("ids").unwrap()[0].start = 6;
            e
        }),
        ("request_span", {
            let mut e = intent.evidence.clone();
            e.requirement_spans.get_mut("ids").unwrap()[0].end = 1000;
            e
        }),
        ("request_coverage", {
            let mut e = intent.evidence.clone();
            e.requirement_spans.clear();
            e
        }),
        ("request_span", {
            let mut e = intent.evidence.clone();
            e.original_request = "Montrér xyz".into();
            e
        }),
    ] {
        let result = compile_intent(
            &engine,
            IntentQuery {
                query: intent.query.clone(),
                evidence,
            },
            CompileOptions::default(),
        )
        .await;
        assert!(
            matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == code)
        );
    }
    let mut unresolved = intent.clone();
    unresolved
        .evidence
        .unresolved_alternatives
        .push("another definition".into());
    assert!(matches!(
        compile_intent(&engine, unresolved, CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::Unresolved { .. }
    ));
    let model = compiler(vec![
        serde_json::to_string(&TypedProposal::Intent {
            query: intent.query,
            evidence: intent.evidence.clone(),
        })
        .unwrap(),
    ]);
    let result = model
        .compile_typed(
            &engine,
            &intent.evidence.original_request,
            CompileOptions::default(),
        )
        .await;
    assert!(result.record.request_spans_validated);
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
}

fn lookup_fixture(duplicate: bool, null_keys_match: bool) -> Engine {
    use semantic_catalog::{
        FactResolution, RelationSemantics, RelationshipDefinition, RelationshipKey,
    };
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("bill", DataType::Int64, true),
        Field::new("ship", DataType::Int64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Int64Array::from(vec![Some(10), Some(20), None])),
            Arc::new(Int64Array::from(vec![Some(20), Some(10), Some(99)])),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("items", schema.clone(), "memory");
    relation.semantics = Some(RelationSemantics {
        metrics: [(
            "id_total".into(),
            semantic_catalog::MetricDefinition {
                id: "metrics/id-total".into(),
                description: "Test measure".into(),
                aliases: vec![],
                function: AggregateFunction::Sum,
                field: Some("id".into()),
                distinct: false,
                source_grain: vec!["id".into()],
                compatible_dimensions: Default::default(),
                compatible_lookup_dimensions: vec![semantic_catalog::MetricLookupDimension {
                    relationship: "bill".into(),
                    field: "region".into(),
                    missing: MissingMatch::Null,
                }],
                sum_rollup_dimensions: None,
                row_filters: vec![],
                result_type: DataType::Int64,
                unit: semantic_catalog::Presence::Missing,
                temporal: semantic_catalog::Presence::Missing,
                empty_behavior: semantic_catalog::EmptyBehavior::Null,
                source_refs: vec![],
            },
        )]
        .into(),
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
                        null_keys_match,
                        cardinality: FactResolution::Unknown,
                        source_refs: vec![],
                    },
                )
            })
            .collect(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("region", DataType::Utf8, true),
    ]));
    let mut ids = vec![Some(10), Some(20), None];
    let mut regions = vec![Some("CH"), Some("GB"), Some("unknown")];
    if duplicate {
        ids.push(Some(10));
        regions.push(Some("DE"));
    }
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(regions)),
        ],
    )
    .unwrap();
    engine
        .register_table(
            Relation::base("customers", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}
fn lookup_query(missing: MissingMatch) -> RowQuery {
    let mut q = query();
    for role in ["bill", "ship"] {
        q.requirements.push(requirement(
            role,
            RowOperation::Lookup {
                usage: LookupUsage::Project,
                relationship: role.into(),
                role: role.into(),
                instance: format!("c_{role}"),
                field: "region".into(),
                alias: format!("{role}_region"),
                missing,
            },
        ));
    }
    q.requirements.push(requirement(
        "sort",
        RowOperation::Order {
            field: field("id"),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        },
    ));
    q
}
#[tokio::test]
async fn lookups_keep_billing_and_shipping_roles_separate_with_explicit_missing_matches() {
    both_paths(
        &lookup_fixture(false, false),
        lookup_query(MissingMatch::Null),
        vec![
            vec!["1", "CH", "GB"],
            vec!["2", "GB", "CH"],
            vec!["3", "", ""],
        ],
    )
    .await;
    both_paths(
        &lookup_fixture(false, true),
        lookup_query(MissingMatch::Null),
        vec![
            vec!["1", "CH", "GB"],
            vec!["2", "GB", "CH"],
            vec!["3", "unknown", ""],
        ],
    )
    .await;
    both_paths(
        &lookup_fixture(false, false),
        lookup_query(MissingMatch::Exclude),
        vec![vec!["1", "CH", "GB"], vec!["2", "GB", "CH"]],
    )
    .await;
}
#[tokio::test]
async fn duplicate_dimension_keys_fail_during_execution_never_fan_out_or_choose_a_value() {
    let engine = lookup_fixture(true, false);
    let result = compile_rows(
        &engine,
        lookup_query(MissingMatch::Null),
        CompileOptions::default(),
    )
    .await;
    assert!(
        result
            .record
            .definition_refs
            .iter()
            .any(|r| r.id == "functions/semantic_assert_single_v1")
    );
    let q = compiled(result);
    let direct = q
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap_err();
    let emitted = q
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap_err();
    assert!(direct.to_string().contains("uniqueness obligation failed"));
    assert!(emitted.to_string().contains("uniqueness obligation failed"));
}

#[tokio::test]
async fn bounded_model_capture_replays_repairs_and_does_not_change_compilation() {
    use semantic_compiler::typed::{CaptureLimits, RecordingProvider, TranscriptProvider};
    let engine = fixture();
    let replies = vec![
        "not JSON".into(),
        serde_json::to_string(&TypedProposal::Query { query: query() }).unwrap(),
    ];
    let provider = RecordingProvider::new(
        Scripted {
            replies: Mutex::new(replies.clone()),
            calls: Mutex::new(vec![]),
        },
        CaptureLimits::default(),
    );
    let recorder = provider.recorder();
    let original = Compiler::new(provider)
        .compile_typed(&engine, "secret request IDs", CompileOptions::default())
        .await;
    let digest = original.record.artifact_digest.clone().unwrap();
    let transcript = recorder.snapshot();
    assert!(transcript.is_complete());
    assert_eq!(transcript.retained_calls(), 2);
    assert!(!format!("{transcript:?}").contains("secret request"));
    assert!(
        !serde_json::to_string(&original.record)
            .unwrap()
            .contains("secret request")
    );
    let provider = Arc::new(TranscriptProvider::new(transcript.clone(), 1024 * 1024).unwrap());
    let replay = Compiler::new(provider.clone())
        .compile_typed(&engine, "secret request IDs", CompileOptions::default())
        .await;
    provider.verify_consumed().unwrap();
    assert_eq!(replay.record.artifact_digest, Some(digest.clone()));
    assert_eq!(replay.record.work.model_calls, 2);
    let wrong = Arc::new(TranscriptProvider::new(transcript, 1024 * 1024).unwrap());
    let mismatched = Compiler::new(wrong.clone())
        .compile_typed(&engine, "changed request", CompileOptions::default())
        .await;
    assert!(matches!(
        mismatched.outcome,
        TypedOutcome::ProviderFailure { .. }
    ));
    assert_eq!(wrong.verify_consumed().unwrap_err().code, "replay_mismatch");
    let provider = RecordingProvider::new(
        Scripted {
            replies: Mutex::new(replies),
            calls: Mutex::new(vec![]),
        },
        CaptureLimits {
            max_bytes: 64,
            max_calls: 1,
        },
    );
    let recorder = provider.recorder();
    let limited = Compiler::new(provider)
        .compile_typed(&engine, "secret request IDs", CompileOptions::default())
        .await;
    assert_eq!(limited.record.artifact_digest, Some(digest));
    let capture = recorder.snapshot();
    assert!(!capture.is_complete());
    assert_eq!(capture.omitted_calls, 2);
    assert!(TranscriptProvider::new(capture, 1024).is_err());
}

#[tokio::test]
async fn aggregate_metrics_include_cache_reuse_rejections_and_usage_without_a_trace_subscriber() {
    use semantic_compiler::{
        provider::{CompletionMetadata, CompletionStatus, ModelCompletion, TokenUsage},
        typed::{CompilationCacheOptions, CompilationSession, CompilerMetrics},
    };
    struct Reported;
    impl ModelProvider for Reported {
        async fn complete(&self, _: &[Message]) -> Result<String, ProviderError> {
            unreachable!("envelope path")
        }
        async fn complete_envelope(&self, _: &[Message]) -> Result<ModelCompletion, ProviderError> {
            Ok(ModelCompletion {
                text: Some(
                    serde_json::to_string(&TypedProposal::Query { query: query() }).unwrap(),
                ),
                status: CompletionStatus::Complete,
                metadata: CompletionMetadata {
                    usage: TokenUsage {
                        input_tokens: Some(8),
                        output_tokens: Some(4),
                        cached_input_tokens: Some(3),
                        reasoning_tokens: Some(1),
                        total_tokens: Some(12),
                    },
                    ..Default::default()
                },
            })
        }
    }
    let engine = fixture();
    let metrics = Arc::new(CompilerMetrics::default());
    let mut options = CompileOptions::default();
    options.metrics = Some(metrics.clone());
    assert!(matches!(
        Compiler::new(Reported)
            .compile_typed(&engine, "IDs", options.clone())
            .await
            .outcome,
        TypedOutcome::Compiled { .. }
    ));
    let session = CompilationSession::new(&engine, CompilationCacheOptions::default()).unwrap();
    let (a, b) = tokio::join!(
        session.compile(query(), options.clone()),
        session.compile(query(), options.clone())
    );
    assert!(matches!(a.outcome, TypedOutcome::Compiled { .. }));
    assert!(matches!(b.outcome, TypedOutcome::Compiled { .. }));
    let mut denied = options.clone();
    denied.allowed_relations = Some(Default::default());
    assert!(matches!(
        session.compile(query(), denied).await.outcome,
        TypedOutcome::Rejected { .. }
    ));
    options.cancellation.cancel();
    assert!(matches!(
        compile_rows(&engine, query(), options).await.outcome,
        TypedOutcome::Unresolved { .. }
    ));
    let total = metrics.snapshot();
    assert_eq!(total.completed, 5);
    assert_eq!(total.outcomes["compiled"], 3);
    assert_eq!(total.outcomes["rejected"], 1);
    assert_eq!(total.outcomes["unresolved"], 1);
    assert_eq!(total.cache_hits, 1);
    assert_eq!(
        (
            total.model_calls,
            total.reported_input_tokens,
            total.reported_output_tokens
        ),
        (1, 8, 4)
    );
    assert_eq!(
        (
            total.reported_cached_input_tokens,
            total.reported_reasoning_tokens
        ),
        (3, 1)
    );
    assert_eq!(total.calls_missing_input_usage, 0);
    assert_eq!(total.latency_buckets.iter().sum::<u128>(), total.completed);
}

fn output_compare(slot: &str, operator: Comparison, value: Literal) -> OutputPredicate {
    RowPredicate::Compare {
        field: OutputRef { slot: slot.into() },
        operator,
        value,
    }
}

#[tokio::test]
async fn output_filters_preserve_aggregate_window_stage_and_exact_rank_literals() {
    let engine = fixture();
    let mut q = RowQuery {
        requirements: vec![
            requirement(
                "group",
                RowOperation::Group {
                    field: field("label"),
                    alias: "group".into(),
                },
            ),
            requirement(
                "sum",
                RowOperation::Aggregate {
                    function: AggregateFunction::Sum,
                    field: Some(field("score")),
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
                        order_by: vec![window_order(
                            WindowInput::Output { slot: "sum".into() },
                            Direction::Desc,
                        )],
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
        ],
        ..query()
    };
    // Filtering the 30 group before rank makes the 20 group rank first. Moving
    // either predicate across the window would change this result.
    both_paths(&engine, q.clone(), vec![vec!["", "20", "1"]]).await;
    q.requirements[4].operation = RowOperation::FilterOutput {
        stage: OutputFilterStage::AfterWindow,
        predicate: output_compare("rank", Comparison::Gt, Literal::UInt64(u64::MAX)),
    };
    both_paths(&engine, q.clone(), vec![]).await;
    // Requirement order is not a staging instruction.
    q.requirements[4].operation = RowOperation::FilterOutput {
        stage: OutputFilterStage::AfterWindow,
        predicate: output_compare("rank", Comparison::Eq, Literal::UInt64(1)),
    };
    q.requirements.swap(2, 4);
    both_paths(&engine, q.clone(), vec![vec!["", "20", "1"]]).await;
    q.requirements[4].operation = RowOperation::FilterOutput {
        stage: OutputFilterStage::AfterAggregate,
        predicate: output_compare("rank", Comparison::Eq, Literal::UInt64(1)),
    };
    assert!(
        matches!(compile_rows(&engine, q, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "output_filter_scope")
    );

    let mut q = query();
    q.requirements.push(requirement(
        "bad",
        RowOperation::FilterOutput {
            stage: OutputFilterStage::AfterAggregate,
            predicate: output_compare("ids", Comparison::Eq, Literal::Int64(1)),
        },
    ));
    assert!(
        matches!(compile_rows(&engine, q, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "filter_stage")
    );
}

#[tokio::test]
async fn metric_windows_require_additive_grain_and_mergeable_state() {
    let q = RowQuery {
        input: RelationInput {
            relation: "governed".into(),
            instance: "r".into(),
        },
        requirements: vec![
            requirement(
                "label",
                RowOperation::Group {
                    field: field("label"),
                    alias: "label".into(),
                },
            ),
            requirement(
                "metric",
                RowOperation::Metric {
                    name: "qualified_score".into(),
                    alias: "score".into(),
                    applicability: Default::default(),
                },
            ),
            requirement(
                "rollup",
                RowOperation::Window {
                    window: WindowSpec {
                        function: WindowFunction::Sum,
                        input: Some(WindowInput::Output {
                            slot: "metric".into(),
                        }),
                        partition_by: vec![],
                        order_by: vec![],
                        frame: WindowFrame::EntirePartition,
                    },
                    alias: "total".into(),
                },
            ),
            requirement(
                "sort",
                RowOperation::OrderOutput {
                    slot: "label".into(),
                    direction: Direction::Asc,
                    nulls: NullOrder::Last,
                },
            ),
        ],
        ..query()
    };
    let engine = governed_fixture().await;
    assert!(
        matches!(compile_rows(&engine, q.clone(), CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_rollup")
    );
    let engine = governed_fixture_rollup(Some(["label".into()].into())).await;
    both_paths(
        &engine,
        q.clone(),
        vec![
            vec!["O'Reilly", "10", "10"],
            vec!["b", "", "10"],
            vec!["", "", "10"],
        ],
    )
    .await;
    for metric in [
        RowOperation::Metric {
            name: "qualified_average".into(),
            alias: "average".into(),
            applicability: Default::default(),
        },
        RowOperation::Aggregate {
            function: AggregateFunction::Count,
            field: Some(field("id")),
            distinct: true,
            alias: "distinct".into(),
        },
    ] {
        let mut invalid = q.clone();
        invalid.requirements[1].operation = metric;
        assert!(
            matches!(compile_rows(&engine, invalid, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_rollup")
        );
    }
    let engine = governed_fixture_rollup(Some(Default::default())).await;
    assert!(
        matches!(compile_rows(&engine, q.clone(), CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_rollup")
    );
    let mut same_grain = q;
    if let RowOperation::Window { window, .. } = &mut same_grain.requirements[2].operation {
        window.partition_by = vec![WindowInput::Output {
            slot: "label".into(),
        }];
    }
    both_paths(
        &engine,
        same_grain,
        vec![
            vec!["O'Reilly", "10", "10"],
            vec!["b", "", ""],
            vec!["", "", ""],
        ],
    )
    .await;
}

#[tokio::test]
async fn authored_value_mappings_bind_exact_codes_and_keep_parameter_provenance() {
    let engine = governed_fixture().await;
    let mut q = query();
    q.input.relation = "governed".into();
    q.requirements.push(requirement(
        "selection",
        RowOperation::Filter {
            predicate: RowPredicate::CompareMapped {
                field: field("label"),
                operator: Comparison::Eq,
                mapping: "labels".into(),
                phrase: "publisher".into(),
            },
        },
    ));
    q.requirements.push(requirement(
        "sort",
        RowOperation::Order {
            field: field("id"),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        },
    ));
    both_paths(&engine, q.clone(), vec![vec!["1"], vec!["4"]]).await;
    let result = compile_rows(&engine, q.clone(), CompileOptions::default()).await;
    assert!(
        result
            .record
            .definition_refs
            .iter()
            .any(|r| r.id == "dictionaries/labels")
    );
    assert_eq!(result.record.work.mapped_value_bytes, "O'Reilly".len());
    let normal_record = serde_json::to_string(&result.record).unwrap();
    assert!(!normal_record.contains("publisher"));
    assert!(!normal_record.contains("O'Reilly"));
    let artifact = compiled(result);
    assert!(
        serde_json::to_string(artifact.bound())
            .unwrap()
            .contains("publisher")
    );
    assert!(
        artifact
            .sql()
            .parameters()
            .contains(&Literal::Utf8("O'Reilly".into()))
    );
    let mut options = CompileOptions::default();
    options.selection_mode = semantic_compiler::typed::SelectionMode::Retrieved;
    let response = serde_json::to_string(&TypedProposal::Query { query: q.clone() }).unwrap();
    let result = compiler(vec![response.clone(), response])
        .compile_typed(&engine, "publisher", options)
        .await;
    assert!(matches!(result.outcome, TypedOutcome::Compiled { .. }));
    for phrase in ["Publisher", "unlisted"] {
        if let RowOperation::Filter {
            predicate: RowPredicate::CompareMapped { phrase: value, .. },
        } = &mut q.requirements[1].operation
        {
            *value = phrase.into();
        }
        assert!(
            matches!(compile_rows(&engine, q.clone(), CompileOptions::default()).await.outcome, TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "unresolved_value")
        );
    }
    q.requirements[1].operation = RowOperation::Filter {
        predicate: RowPredicate::CompareMapped {
            field: field("id"),
            operator: Comparison::Eq,
            mapping: "labels".into(),
            phrase: "publisher".into(),
        },
    };
    assert!(
        matches!(compile_rows(&engine, q, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "value_mapping_scope")
    );
}

#[tokio::test]
async fn grouped_lookups_preserve_fact_grain_missing_matches_and_metric_dimension_contracts() {
    let engine = lookup_fixture(false, false);
    let mut q = query();
    q.requirements = vec![
        requirement(
            "region",
            RowOperation::Lookup {
                relationship: "bill".into(),
                role: "bill".into(),
                instance: "billing".into(),
                field: "region".into(),
                alias: "region".into(),
                missing: MissingMatch::Null,
                usage: LookupUsage::Group,
            },
        ),
        requirement(
            "total",
            RowOperation::Metric {
                name: "id_total".into(),
                alias: "total".into(),
                applicability: Default::default(),
            },
        ),
        requirement(
            "sort",
            RowOperation::OrderOutput {
                slot: "region".into(),
                direction: Direction::Asc,
                nulls: NullOrder::Last,
            },
        ),
    ];
    both_paths(
        &engine,
        q.clone(),
        vec![vec!["CH", "1"], vec!["GB", "2"], vec!["", "3"]],
    )
    .await;
    let duplicate = lookup_fixture(true, false);
    let artifact = compiled(compile_rows(&duplicate, q.clone(), CompileOptions::default()).await);
    assert!(
        artifact
            .plan_direct(&duplicate)
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
    assert!(
        artifact
            .execute(&duplicate, QueryOptions::default())
            .await
            .unwrap()
            .collect()
            .await
            .is_err()
    );
    let mut explicit = q.clone();
    explicit.requirements[1].operation = RowOperation::Aggregate {
        function: AggregateFunction::Sum,
        field: Some(field("id")),
        distinct: false,
        alias: "total".into(),
    };
    if let RowOperation::Lookup { missing, .. } = &mut explicit.requirements[0].operation {
        *missing = MissingMatch::Exclude;
    }
    both_paths(&engine, explicit, vec![vec!["CH", "1"], vec!["GB", "2"]]).await;
    if let RowOperation::Lookup { missing, .. } = &mut q.requirements[0].operation {
        *missing = MissingMatch::Exclude;
    }
    assert!(
        matches!(compile_rows(&engine, q, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "metric_dimensions")
    );
}

#[tokio::test]
async fn query_graph_sets_preserve_duplicates_nulls_parameter_slots_and_requirement_coverage() {
    use semantic_plan::graph::*;
    let engine = fixture();
    let mut left = query();
    left.requirements = vec![
        requirement(
            "label",
            RowOperation::Project {
                field: field("label"),
                alias: "left_label".into(),
            },
        ),
        requirement(
            "subset",
            RowOperation::Filter {
                predicate: compare("id", Comparison::LtEq, Literal::Int64(4)),
            },
        ),
    ];
    let mut right = left.clone();
    right.requirements[0].operation = RowOperation::Project {
        field: field("label"),
        alias: "right_label".into(),
    };
    right.requirements[1].operation = RowOperation::Filter {
        predicate: compare("id", Comparison::GtEq, Literal::Int64(3)),
    };
    for (operator, duplicates, expected) in [
        (
            SetOperator::Union,
            Duplicates::All,
            vec!["B", "O'Reilly", "O'Reilly", "O'Reilly", "b", "", ""],
        ),
        (
            SetOperator::Union,
            Duplicates::Distinct,
            vec!["B", "O'Reilly", "b", ""],
        ),
        (
            SetOperator::Intersect,
            Duplicates::All,
            vec!["O'Reilly", ""],
        ),
        (
            SetOperator::Intersect,
            Duplicates::Distinct,
            vec!["O'Reilly", ""],
        ),
        (SetOperator::Except, Duplicates::All, vec!["O'Reilly", "b"]),
        (SetOperator::Except, Duplicates::Distinct, vec!["b"]),
    ] {
        let q = GraphQuery {
            version: 1,
            nodes: vec![
                QueryNode {
                    id: "set".into(),
                    source_text: "explicit set operation".into(),
                    operation: GraphOperation::Set {
                        left: "left".into(),
                        right: "right".into(),
                        operator,
                        duplicates,
                        columns: vec![SetColumn {
                            id: "value".into(),
                            left: "label".into(),
                            right: "label".into(),
                            alias: "label".into(),
                        }],
                    },
                },
                QueryNode {
                    id: "left".into(),
                    source_text: "left subset".into(),
                    operation: GraphOperation::Rows {
                        query: left.clone(),
                    },
                },
                QueryNode {
                    id: "right".into(),
                    source_text: "right subset".into(),
                    operation: GraphOperation::Rows {
                        query: right.clone(),
                    },
                },
            ],
            root: "set".into(),
            ordering: vec![GraphOrder {
                slot: "value".into(),
                direction: Direction::Asc,
                nulls: NullOrder::Last,
            }],
            limit: None,
            unresolved: vec![],
        };
        let result =
            semantic_compiler::typed::compile_graph(&engine, q.clone(), CompileOptions::default())
                .await;
        let TypedOutcome::CompiledGraph { query: artifact } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
        assert_eq!(
            artifact.sql().parameters(),
            &[Literal::Int64(4), Literal::Int64(3)]
        );
        let expected: Vec<Vec<String>> = expected.into_iter().map(|s| vec![s.into()]).collect();
        assert_eq!(
            values(
                &artifact
                    .plan_direct(&engine)
                    .await
                    .unwrap()
                    .collect()
                    .await
                    .unwrap()
            ),
            expected
        );
        assert_eq!(
            values(
                &artifact
                    .execute(&engine, QueryOptions::default())
                    .await
                    .unwrap()
                    .collect()
                    .await
                    .unwrap()
            ),
            expected
        );
        assert!(
            result
                .record
                .requirement_dispositions
                .iter()
                .all(|r| r.requirement_id.starts_with("requirement/"))
        );
        assert!(
            !serde_json::to_string(&result.record)
                .unwrap()
                .contains("subset")
        );
        let mut disconnected = q.clone();
        disconnected.root = "left".into();
        assert!(
            matches!(semantic_compiler::typed::compile_graph(&engine, disconnected, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_coverage")
        );
        let mut cycle = q;
        if let GraphOperation::Set { left, .. } = &mut cycle.nodes[0].operation {
            *left = "set".into();
        }
        cycle.nodes.remove(1);
        assert!(
            matches!(semantic_compiler::typed::compile_graph(&engine, cycle, CompileOptions::default()).await.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_cycle")
        );
    }
}

fn composition_fixture() -> Engine {
    let mut engine = Engine::new();
    for (name, regions, amounts) in [
        (
            "sales",
            vec![Some("A"), Some("A"), Some("B"), Some("C"), None],
            vec![Some(5), Some(7), None, Some(3), Some(4)],
        ),
        (
            "costs",
            vec![Some("A"), Some("A"), Some("D"), None],
            vec![Some(2), Some(4), Some(8), Some(9)],
        ),
    ] {
        let schema = Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, true),
            Field::new("amount", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(regions)),
                Arc::new(Int64Array::from(amounts)),
            ],
        )
        .unwrap();
        let mut relation = Relation::base(name, schema.clone(), "memory");
        if name == "sales" {
            relation.semantics = Some(semantic_catalog::RelationSemantics {
                relationships: [(
                    "regional_costs".into(),
                    semantic_catalog::RelationshipDefinition {
                        ai_context: None,
                        id: "relationships/regional-costs".into(),
                        right_relation: "costs".into(),
                        role: "same_region".into(),
                        key_pairs: vec![semantic_catalog::RelationshipKey {
                            left_field: "region".into(),
                            right_field: "region".into(),
                        }],
                        null_keys_match: false,
                        cardinality: semantic_catalog::FactResolution::Unknown,
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
fn composition_query(
    domain: semantic_plan::graph::GroupDomain,
    null_alignment: semantic_plan::graph::NullAlignment,
) -> semantic_plan::graph::GraphQuery {
    use semantic_plan::graph::*;
    let mut nodes: Vec<_> = ["sales", "costs"]
        .into_iter()
        .map(|name| QueryNode {
            id: name.into(),
            source_text: format!("{name} by region"),
            operation: GraphOperation::Rows {
                query: RowQuery {
                    input: RelationInput {
                        relation: name.into(),
                        instance: "r".into(),
                    },
                    requirements: vec![
                        requirement(
                            "group",
                            RowOperation::Group {
                                field: field("region"),
                                alias: "region".into(),
                            },
                        ),
                        requirement(
                            "amount",
                            RowOperation::Aggregate {
                                function: AggregateFunction::Sum,
                                field: Some(field("amount")),
                                distinct: false,
                                alias: "amount".into(),
                            },
                        ),
                    ],
                    ..query()
                },
            },
        })
        .collect();
    nodes.push(QueryNode {
        id: "comparison".into(),
        source_text: "compare by region, missing groups zero".into(),
        operation: GraphOperation::Compose {
            left: "sales".into(),
            right: "costs".into(),
            relationship_relation: "sales".into(),
            relationship: "regional_costs".into(),
            role: "same_region".into(),
            domain,
            null_alignment,
            keys: vec![SetColumn {
                id: "region".into(),
                left: "group".into(),
                right: "group".into(),
                alias: "region".into(),
            }],
            outputs: vec![
                CompositionOutput {
                    id: "sales".into(),
                    side: Side::Left,
                    slot: "amount".into(),
                    alias: "sales".into(),
                    missing: MissingGroup::Zero,
                },
                CompositionOutput {
                    id: "costs".into(),
                    side: Side::Right,
                    slot: "amount".into(),
                    alias: "costs".into(),
                    missing: MissingGroup::Zero,
                },
            ],
        },
    });
    GraphQuery {
        version: 1,
        nodes,
        root: "comparison".into(),
        ordering: vec![
            GraphOrder {
                slot: "region".into(),
                direction: Direction::Asc,
                nulls: NullOrder::Last,
            },
            GraphOrder {
                slot: "sales".into(),
                direction: Direction::Desc,
                nulls: NullOrder::Last,
            },
        ],
        limit: None,
        unresolved: vec![],
    }
}
#[tokio::test]
async fn fact_composition_aggregates_before_alignment_and_distinguishes_missing_from_null_groups() {
    use semantic_plan::graph::*;
    let engine = composition_fixture();
    for (domain, nulls, expected) in [
        (
            GroupDomain::Union,
            NullAlignment::Match,
            vec![
                vec!["A", "12", "6"],
                vec!["B", "", "0"],
                vec!["C", "3", "0"],
                vec!["D", "0", "8"],
                vec!["", "4", "9"],
            ],
        ),
        (
            GroupDomain::Intersection,
            NullAlignment::Match,
            vec![vec!["A", "12", "6"], vec!["", "4", "9"]],
        ),
        (
            GroupDomain::Left,
            NullAlignment::Match,
            vec![
                vec!["A", "12", "6"],
                vec!["B", "", "0"],
                vec!["C", "3", "0"],
                vec!["", "4", "9"],
            ],
        ),
        (
            GroupDomain::Right,
            NullAlignment::Match,
            vec![
                vec!["A", "12", "6"],
                vec!["D", "0", "8"],
                vec!["", "4", "9"],
            ],
        ),
        (
            GroupDomain::Union,
            NullAlignment::NeverMatch,
            vec![
                vec!["A", "12", "6"],
                vec!["B", "", "0"],
                vec!["C", "3", "0"],
                vec!["D", "0", "8"],
                vec!["", "4", "0"],
                vec!["", "0", "9"],
            ],
        ),
    ] {
        let result = semantic_compiler::typed::compile_graph(
            &engine,
            composition_query(domain, nulls),
            CompileOptions::default(),
        )
        .await;
        assert!(
            result
                .record
                .definition_refs
                .iter()
                .any(|r| r.id == "relationships/regional-costs")
        );
        let TypedOutcome::CompiledGraph { query } = result.outcome else {
            panic!("{:?}", result.outcome)
        };
        assert_eq!(
            values(
                &query
                    .plan_direct(&engine)
                    .await
                    .unwrap()
                    .collect()
                    .await
                    .unwrap()
            ),
            expected
        );
        assert_eq!(
            values(
                &query
                    .execute(&engine, QueryOptions::default())
                    .await
                    .unwrap()
                    .collect()
                    .await
                    .unwrap()
            ),
            expected
        );
    }
    let mut invalid = composition_query(GroupDomain::Union, NullAlignment::Match);
    if let GraphOperation::Compose { role, .. } = &mut invalid.nodes[2].operation {
        *role = "different_role".into();
    }
    assert!(
        matches!(semantic_compiler::typed::compile_graph(&engine,invalid,CompileOptions::default()).await.outcome,TypedOutcome::Rejected{diagnostic} if diagnostic.code=="composition_relationship")
    );
}

#[tokio::test]
async fn model_graphs_share_context_scope_and_generated_ctes_never_capture_catalog_names() {
    use semantic_plan::graph::*;
    let mut engine = fixture();
    engine
        .create_view("__semantic_graph_0", "SELECT 999::bigint AS id")
        .await
        .unwrap();
    let mut second = query();
    second.input.relation = "__semantic_graph_0".into();
    let q = GraphQuery {
        version: 1,
        nodes: vec![
            QueryNode {
                id: "a".into(),
                source_text: "item IDs".into(),
                operation: GraphOperation::Rows { query: query() },
            },
            QueryNode {
                id: "b".into(),
                source_text: "special IDs".into(),
                operation: GraphOperation::Rows { query: second },
            },
            QueryNode {
                id: "all".into(),
                source_text: "combine all IDs".into(),
                operation: GraphOperation::Set {
                    left: "a".into(),
                    right: "b".into(),
                    operator: SetOperator::Union,
                    duplicates: Duplicates::All,
                    columns: vec![SetColumn {
                        id: "id".into(),
                        left: "ids".into(),
                        right: "ids".into(),
                        alias: "id".into(),
                    }],
                },
            },
        ],
        root: "all".into(),
        ordering: vec![GraphOrder {
            slot: "id".into(),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        }],
        limit: None,
        unresolved: vec![],
    };
    let response = serde_json::to_string(&TypedProposal::Graph { query: q.clone() }).unwrap();
    let result = compiler(vec![response.clone()])
        .compile_typed(
            &engine,
            "Combine all item and special IDs",
            CompileOptions::default(),
        )
        .await;
    assert_eq!(result.record.work.model_calls, 1);
    let TypedOutcome::CompiledGraph { query: compiled } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let expected = vec![
        vec!["1"],
        vec!["2"],
        vec!["3"],
        vec!["4"],
        vec!["5"],
        vec!["999"],
    ];
    assert_eq!(
        values(
            &compiled
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()
        ),
        expected
    );
    assert_eq!(
        values(
            &compiled
                .plan_direct(&engine)
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()
        ),
        expected
    );
    let mut options = CompileOptions::default();
    options.allowed_relations = Some(["items".into()].into());
    let result = compiler(vec![response])
        .compile_typed(&engine, "Combine IDs", options)
        .await;
    assert!(
        matches!(result.outcome,TypedOutcome::Rejected{diagnostic} if diagnostic.code=="access_scope")
    );
    let mut exponential = q;
    exponential.ordering.clear();
    for i in 0..12 {
        let previous = exponential.root.clone();
        let id = format!("double_{i}");
        exponential.nodes.push(QueryNode {
            id: id.clone(),
            source_text: "duplicate all rows".into(),
            operation: GraphOperation::Set {
                left: previous.clone(),
                right: previous,
                operator: SetOperator::Union,
                duplicates: Duplicates::All,
                columns: vec![SetColumn {
                    id: "id".into(),
                    left: "id".into(),
                    right: "id".into(),
                    alias: "id".into(),
                }],
            },
        });
        exponential.root = id;
    }
    let result =
        semantic_compiler::typed::compile_graph(&engine, exponential, CompileOptions::default())
            .await;
    assert!(
        matches!(result.outcome,TypedOutcome::Unresolved{diagnostic} if diagnostic.code=="graph_expansion_limit")
    );
}

#[tokio::test]
async fn graph_replay_is_bounded_pinned_and_revalidates_host_scope_and_artifact_identity() {
    use semantic_plan::graph::{GroupDomain, NullAlignment};
    let engine = composition_fixture();
    let compiled = semantic_compiler::typed::compile_graph(
        &engine,
        composition_query(GroupDomain::Union, NullAlignment::Match),
        CompileOptions::default(),
    )
    .await;
    let expected = compiled.record.artifact_digest.clone();
    let TypedOutcome::CompiledGraph { query } = compiled.outcome else {
        panic!("{:?}", compiled.outcome)
    };
    assert!(query.capture_replay(8).is_err());
    let bundle = query.capture_replay(128 * 1024).unwrap();
    let encoded = serde_json::to_vec(&bundle).unwrap();
    let restored: semantic_compiler::typed::GraphReplayBundle =
        serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        restored
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap()
            .record
            .artifact_digest,
        expected
    );
    let mut tampered = restored.clone();
    tampered.proposal.limit = Some(1);
    assert_eq!(
        tampered
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_mismatch"
    );
    let mut options = CompileOptions::default();
    options.allowed_relations = Some(["sales".into()].into());
    let scoped = restored.replay(&engine, options).await.unwrap();
    assert!(
        matches!(scoped.outcome,TypedOutcome::Rejected{diagnostic} if diagnostic.code=="access_scope")
    );
    tampered = restored;
    tampered.execution_profile_revision = "old".into();
    assert_eq!(
        tampered
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_profile"
    );
    tampered.execution_profile_revision = semantic_engine::MVP_EXECUTION_PROFILE_REVISION.into();
    tampered.pipeline_revision = "old".into();
    assert_eq!(
        tampered
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_version"
    );
}

fn traced_graph_intent() -> semantic_plan::graph::GraphIntentQuery {
    use semantic_plan::graph::*;
    let request = "Montrér IDs twice ascending first two";
    let leaf = |id: &str| {
        let mut q = query();
        q.requirements[0].id = id.into();
        q.requirements[0].source_text = "IDs".into();
        q
    };
    let q = GraphQuery {
        version: 1,
        nodes: vec![
            QueryNode {
                id: "a/b".into(),
                source_text: "IDs".into(),
                operation: GraphOperation::Rows { query: leaf("c") },
            },
            QueryNode {
                id: "a".into(),
                source_text: "IDs".into(),
                operation: GraphOperation::Rows { query: leaf("b/c") },
            },
            QueryNode {
                id: "twice".into(),
                source_text: "twice".into(),
                operation: GraphOperation::Set {
                    left: "a/b".into(),
                    right: "a".into(),
                    operator: SetOperator::Union,
                    duplicates: Duplicates::All,
                    columns: vec![SetColumn {
                        id: "id".into(),
                        left: "c".into(),
                        right: "b/c".into(),
                        alias: "id".into(),
                    }],
                },
            },
        ],
        root: "twice".into(),
        ordering: vec![GraphOrder {
            slot: "id".into(),
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        }],
        limit: Some(2),
        unresolved: vec![],
    };
    let requirements = [
        (GraphRequirementRef::Node { node: "a/b".into() }, "IDs"),
        (
            GraphRequirementRef::Leaf {
                node: "a/b".into(),
                requirement: "c".into(),
            },
            "IDs",
        ),
        (GraphRequirementRef::Node { node: "a".into() }, "IDs"),
        (
            GraphRequirementRef::Leaf {
                node: "a".into(),
                requirement: "b/c".into(),
            },
            "IDs",
        ),
        (
            GraphRequirementRef::Node {
                node: "twice".into(),
            },
            "twice",
        ),
        (
            GraphRequirementRef::Output {
                node: "twice".into(),
                slot: "id".into(),
            },
            "IDs",
        ),
        (GraphRequirementRef::Order { index: 0 }, "ascending"),
        (GraphRequirementRef::Limit, "first two"),
    ]
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
    .collect();
    GraphIntentQuery {
        query: q,
        evidence: GraphRequestEvidence {
            version: 1,
            request_id: "graph-request".into(),
            original_request: request.into(),
            requirements,
            unresolved_alternatives: vec![],
        },
    }
}

#[tokio::test]
async fn graph_intent_covers_scoped_requirements_order_limit_and_replays_exact_evidence() {
    use semantic_compiler::typed::{compile_graph, compile_graph_intent};
    use semantic_plan::graph::*;
    let engine = fixture();
    let intent = traced_graph_intent();
    let result = compile_graph_intent(&engine, intent.clone(), CompileOptions::default()).await;
    assert!(
        result.record.request_spans_validated,
        "{:?}",
        result.outcome
    );
    assert!(result.record.request_digest.is_some());
    assert!(result.record.relational_digest.is_some());
    assert_eq!(result.record.work.graph_nodes_visited, 3);
    assert_eq!(result.record.work.graph_edges_visited, 2);
    assert_eq!(result.record.work.graph_outputs_checked, 1);
    let recorded: std::collections::BTreeSet<_> = result
        .record
        .requirement_dispositions
        .iter()
        .map(|d| d.requirement_id.clone())
        .collect();
    assert_eq!(recorded.len(), intent.evidence.requirements.len());
    assert_eq!(recorded.len(), result.record.requirement_dispositions.len());
    assert!(recorded.iter().all(|id| id.starts_with("requirement/")));
    assert!(
        result
            .record
            .requirement_dispositions
            .iter()
            .all(|d| d.result == "lowered_and_verified")
    );
    let record_json = serde_json::to_string(&result.record).unwrap();
    for sensitive in ["Montrér", "a/b", "b/c"] {
        assert!(!record_json.contains(sensitive));
    }
    let digest = result.record.artifact_digest;
    let TypedOutcome::CompiledGraph { query: artifact } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let expected_rows = vec![vec!["1".to_string()], vec!["1".to_string()]];
    assert_eq!(
        values(
            &artifact
                .plan_direct(&engine)
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()
        ),
        expected_rows
    );
    assert_eq!(
        values(
            &artifact
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()
        ),
        expected_rows
    );
    assert!(artifact.capture_replay(8).is_err());
    let capture = artifact.capture_replay(64 * 1024).unwrap();
    let restored: semantic_compiler::typed::GraphReplayBundle =
        serde_json::from_slice(&serde_json::to_vec(&capture).unwrap()).unwrap();
    assert_eq!(restored.request_evidence.as_ref(), Some(&intent.evidence));
    let replay = restored
        .replay(&engine, CompileOptions::default())
        .await
        .unwrap();
    assert!(replay.record.request_spans_validated);
    assert_eq!(replay.record.artifact_digest, digest);
    let mut options = CompileOptions::default();
    let mut changed = intent.evidence.clone();
    changed.request_id = "changed".into();
    options.graph_request_evidence = Some(changed);
    assert_eq!(
        restored.replay(&engine, options).await.unwrap_err().code,
        "replay_evidence"
    );
    let mut changed = restored.clone();
    changed
        .request_evidence
        .as_mut()
        .unwrap()
        .original_request
        .push('!');
    assert_eq!(
        changed
            .replay(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_mismatch"
    );
    let legacy = compile_graph(&engine, intent.query.clone(), CompileOptions::default()).await;
    assert!(matches!(legacy.outcome, TypedOutcome::CompiledGraph { .. }));
    assert!(!legacy.record.request_spans_validated);
    assert!(legacy.record.request_digest.is_none());
    assert_eq!(
        legacy.record.requirement_dispositions.len(),
        intent.evidence.requirements.len()
    );
    let model = compiler(vec![
        serde_json::to_string(&TypedProposal::GraphIntent {
            query: intent.query,
            evidence: intent.evidence.clone(),
        })
        .unwrap(),
    ]);
    let result = model
        .compile_typed(
            &engine,
            &intent.evidence.original_request,
            CompileOptions::default(),
        )
        .await;
    assert!(
        matches!(result.outcome, TypedOutcome::CompiledGraph { .. }),
        "{:?}",
        result.outcome
    );
    assert!(result.record.request_spans_validated);
    assert_eq!(result.record.artifact_digest, digest);
    // Escaping is injective even when input IDs already contain escape-like text.
    assert_ne!(
        GraphRequirementRef::Node { node: "a/b".into() }.record_id(),
        GraphRequirementRef::Node {
            node: "a~1b".into()
        }
        .record_id()
    );
}

#[tokio::test]
async fn graph_intent_rejects_incomplete_or_orphan_evidence_and_invalid_spans() {
    use semantic_compiler::typed::compile_graph_intent;
    use semantic_plan::graph::*;
    let engine = fixture();
    let original = traced_graph_intent();
    let mut cases = Vec::new();
    for target in [
        GraphRequirementRef::Order { index: 0 },
        GraphRequirementRef::Limit,
        GraphRequirementRef::Node {
            node: "twice".into(),
        },
        GraphRequirementRef::Output {
            node: "twice".into(),
            slot: "id".into(),
        },
    ] {
        let mut intent = original.clone();
        intent
            .evidence
            .requirements
            .retain(|entry| entry.target != target);
        cases.push(("request_coverage", intent));
    }
    let mut intent = original.clone();
    intent.query.limit = None; // Removing the operation leaves an orphan mandatory clause.
    cases.push(("request_coverage", intent));
    let mut intent = original.clone();
    intent.evidence.requirements[0].target = intent.evidence.requirements[1].target.clone();
    cases.push(("request_coverage", intent));
    let mut intent = original.clone();
    intent.evidence.requirements[0].target = GraphRequirementRef::Node {
        node: "missing".into(),
    };
    cases.push(("request_coverage", intent));
    let mut intent = original.clone();
    intent.evidence.requirements[0].source_spans.clear();
    cases.push(("request_coverage", intent));
    for span in [
        RequestSpan { start: 6, end: 8 },
        RequestSpan {
            start: 0,
            end: 1000,
        },
        RequestSpan { start: 8, end: 9 },
    ] {
        let mut intent = original.clone();
        intent.evidence.requirements[0].source_spans = vec![span];
        cases.push(("request_span", intent));
    }
    let mut intent = original.clone();
    intent.query.nodes[0].source_text = "different".into();
    cases.push(("request_span", intent));
    let mut intent = original.clone();
    intent.evidence.version = 2;
    cases.push(("request_evidence", intent));
    for (code, intent) in cases {
        let result = compile_graph_intent(&engine, intent, CompileOptions::default()).await;
        assert!(
            matches!(&result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == code),
            "expected {code}, got {:?}",
            result.outcome
        );
        assert!(!result.record.request_spans_validated);
    }
    let mut unresolved = original.clone();
    unresolved
        .evidence
        .unresolved_alternatives
        .push("different group domain".into());
    assert!(
        matches!(compile_graph_intent(&engine, unresolved, CompileOptions::default()).await.outcome,
        TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "unresolved_alternatives")
    );
    let mut options = CompileOptions::default();
    options.max_nodes = original.evidence.requirements.len() - 1;
    assert!(
        matches!(compile_graph_intent(&engine, original.clone(), options).await.outcome,
        TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "work_limit")
    );
    let mut options = CompileOptions::default();
    options.max_input_bytes = 32;
    assert!(
        matches!(compile_graph_intent(&engine, original.clone(), options).await.outcome,
        TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "input_limit")
    );
    // Evidence of one IR kind cannot silently disappear into another compiler.
    let mut options = CompileOptions::default();
    options.graph_request_evidence = Some(original.evidence.clone());
    assert!(
        matches!(compile_rows(&engine, query(), options).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_evidence")
    );
    let mut options = CompileOptions::default();
    options.request_evidence = Some(traced_intent().evidence);
    assert!(
        matches!(compile_graph_intent(&engine, original, options).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "graph_evidence")
    );
}

#[tokio::test]
async fn graph_intent_repairs_preserve_host_evidence_and_clear_previous_attempt_guarantees() {
    let engine = fixture();
    let intent = traced_graph_intent();
    let mut invalid = intent.clone();
    invalid.query.ordering[0].slot = "unavailable".into();
    let model = compiler(vec![
        serde_json::to_string(&TypedProposal::GraphIntent {
            query: invalid.query,
            evidence: invalid.evidence,
        })
        .unwrap(),
        serde_json::to_string(&TypedProposal::Graph {
            query: intent.query.clone(),
        })
        .unwrap(),
    ]);
    let result = model
        .compile_typed(
            &engine,
            &intent.evidence.original_request,
            CompileOptions::default(),
        )
        .await;
    assert!(
        matches!(result.outcome, TypedOutcome::CompiledGraph { .. }),
        "{:?}",
        result.outcome
    );
    assert_eq!(result.record.work.model_calls, 2);
    assert!(!result.record.request_spans_validated);
    assert!(result.record.request_digest.is_none());

    let mut changed = intent.evidence.clone();
    changed.request_id = "model-replacement".into();
    let model = compiler(vec![
        serde_json::to_string(&TypedProposal::GraphIntent {
            query: intent.query.clone(),
            evidence: changed,
        })
        .unwrap(),
        serde_json::to_string(&TypedProposal::Graph {
            query: intent.query.clone(),
        })
        .unwrap(),
    ]);
    let mut options = CompileOptions::default();
    options.graph_request_evidence = Some(intent.evidence.clone());
    let result = model
        .compile_typed(&engine, &intent.evidence.original_request, options)
        .await;
    assert!(
        matches!(result.outcome, TypedOutcome::CompiledGraph { .. }),
        "{:?}",
        result.outcome
    );
    assert_eq!(result.record.work.model_calls, 2);
    assert!(result.record.request_spans_validated);

    let mut wrong_request = intent.evidence.clone();
    wrong_request.original_request.push('!');
    let model = compiler(vec![
        serde_json::to_string(&TypedProposal::GraphIntent {
            query: intent.query.clone(),
            evidence: wrong_request,
        })
        .unwrap(),
        serde_json::to_string(&TypedProposal::GraphIntent {
            query: intent.query,
            evidence: intent.evidence.clone(),
        })
        .unwrap(),
    ]);
    let result = model
        .compile_typed(
            &engine,
            &intent.evidence.original_request,
            CompileOptions::default(),
        )
        .await;
    assert!(
        matches!(result.outcome, TypedOutcome::CompiledGraph { .. }),
        "{:?}",
        result.outcome
    );
    assert_eq!(result.record.work.model_calls, 2);
    assert!(result.record.request_spans_validated);
}
