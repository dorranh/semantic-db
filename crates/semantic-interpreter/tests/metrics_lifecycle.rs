use std::{
    collections::BTreeMap,
    future::pending,
    sync::{Arc, Mutex},
    time::Duration,
};

use datafusion::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{
    CompilationCacheOptions, CompilationSession, CompileOptions, CompilerMetrics, TypedOutcome,
    compile_graph, compile_graph_intent, compile_intent, compile_rows, compile_semantic,
};
use semantic_engine::Engine;
use semantic_interpreter::{
    Interpreter,
    provider::{Message, ModelProvider, ProviderError},
    typed::{InterpretOptions, InterpreterMetrics},
};
use semantic_plan::{graph::*, typed::*};

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1, 2]))]).unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("items", schema.clone(), "memory"),
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn row() -> RowQuery {
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "items".into(),
            instance: "r".into(),
        },
        requirements: vec![Requirement {
            id: "id".into(),
            source_text: "IDs".into(),
            operation: RowOperation::Project {
                field: FieldRef {
                    instance: "r".into(),
                    field: "id".into(),
                },
                alias: "id".into(),
            },
        }],
        unresolved: vec![],
    }
}

fn graph() -> GraphQuery {
    GraphQuery {
        version: 1,
        nodes: vec![QueryNode {
            id: "items".into(),
            source_text: "IDs".into(),
            operation: GraphOperation::Rows { query: row() },
        }],
        root: "items".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    }
}

fn graph_intent() -> GraphIntentQuery {
    GraphIntentQuery {
        query: graph(),
        evidence: GraphRequestEvidence {
            version: 1,
            request_id: "metrics-lifecycle".into(),
            original_request: "IDs".into(),
            requirements: vec![
                GraphRequirementEvidence {
                    target: GraphRequirementRef::Node {
                        node: "items".into(),
                    },
                    source_spans: vec![RequestSpan { start: 0, end: 3 }],
                },
                GraphRequirementEvidence {
                    target: GraphRequirementRef::Leaf {
                        node: "items".into(),
                        requirement: "id".into(),
                    },
                    source_spans: vec![RequestSpan { start: 0, end: 3 }],
                },
            ],
            unresolved_alternatives: vec![],
        },
    }
}

fn row_intent() -> IntentQuery {
    IntentQuery {
        query: row(),
        evidence: RequestEvidence {
            version: 1,
            request_id: "metrics-lifecycle-row".into(),
            original_request: "IDs".into(),
            requirement_spans: BTreeMap::from([(
                "id".into(),
                vec![RequestSpan { start: 0, end: 3 }],
            )]),
            unresolved_alternatives: vec![],
        },
    }
}

fn interpret_options(
    compiler: CompileOptions,
    metrics: &Arc<InterpreterMetrics>,
) -> InterpretOptions {
    let mut options = InterpretOptions::default();
    options.compiler = compiler;
    options.metrics = Some(metrics.clone());
    options
}

fn options(metrics: &Arc<CompilerMetrics>) -> CompileOptions {
    let mut options = CompileOptions::default();
    options.metrics = Some(metrics.clone());
    options
}

struct Unsupported;
impl ModelProvider for Unsupported {
    async fn complete(&self, _messages: &[Message]) -> Result<String, ProviderError> {
        Ok(r#"{"status":"unsupported","reason":"No requested calculation"}"#.into())
    }
}

#[tokio::test]
async fn one_logical_request_is_counted_once_on_each_public_compile_route() {
    let engine = fixture();
    let metrics = Arc::new(CompilerMetrics::default());
    let interpreter_metrics = Arc::new(InterpreterMetrics::default());
    let session = CompilationSession::new(&engine, CompilationCacheOptions::default()).unwrap();
    let _ = compile_rows(&engine, row(), options(&metrics)).await;
    let _ = compile_semantic(&engine, row(), options(&metrics)).await;
    let _ = compile_intent(&engine, row_intent(), options(&metrics)).await;
    let _ = compile_graph(&engine, graph(), options(&metrics)).await;
    let _ = compile_graph_intent(&engine, graph_intent(), options(&metrics)).await;
    let _ = session.compile(row(), options(&metrics)).await;
    let _ = session.compile_graph(graph(), options(&metrics)).await;
    let _ = session
        .compile_graph_intent(graph_intent(), options(&metrics))
        .await;
    let _ = Interpreter::new(Unsupported)
        .compile_typed(
            &engine,
            "IDs",
            interpret_options(options(&metrics), &interpreter_metrics),
        )
        .await;
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.admitted, 8);
    assert_eq!(snapshot.completed, 8);
    let interpreted = interpreter_metrics.snapshot();
    assert_eq!(interpreted.admitted, 1);
    assert_eq!(interpreted.completed, 1);
    assert_eq!(interpreted.model_calls, 1);
    assert_eq!(interpreted.outcomes["unsupported"], 1);
    assert_eq!(snapshot.in_flight, 0);
    assert!(snapshot.peak_in_flight >= 1);
    assert_eq!(snapshot.abandoned, 0);
}

struct PendingProvider {
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

struct MultiPending {
    entered: tokio::sync::mpsc::UnboundedSender<()>,
}
impl ModelProvider for MultiPending {
    async fn complete(&self, _messages: &[Message]) -> Result<String, ProviderError> {
        self.entered.send(()).unwrap();
        pending().await
    }
}

#[tokio::test]
async fn concurrent_pending_requests_show_live_and_peak_counts() {
    let engine = Arc::new(fixture());
    let metrics = Arc::new(InterpreterMetrics::default());
    let (entered, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let compiler = Arc::new(Interpreter::new(MultiPending { entered }));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let engine = engine.clone();
        let metrics = metrics.clone();
        let compiler = compiler.clone();
        tasks.push(tokio::spawn(async move {
            let mut options = InterpretOptions::default();
            options.metrics = Some(metrics);
            compiler.compile_typed(&engine, "IDs", options).await
        }));
    }
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("both providers must be entered")
            .unwrap();
    }
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.admitted, 2);
    assert_eq!(snapshot.in_flight, 2);
    assert_eq!(snapshot.peak_in_flight, 2);
    assert_eq!(snapshot.completed, 0);
    for task in tasks {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.in_flight, 0);
    assert_eq!(snapshot.abandoned, 2);
}
impl ModelProvider for PendingProvider {
    async fn complete(&self, _messages: &[Message]) -> Result<String, ProviderError> {
        let sender = { self.entered.lock().unwrap().take() };
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
        pending().await
    }
}

#[tokio::test]
async fn cancellation_deadline_abort_and_panic_leave_an_accurate_gauge() {
    let engine = Arc::new(fixture());
    let metrics = Arc::new(CompilerMetrics::default());
    let interpreter_metrics = Arc::new(InterpreterMetrics::default());

    let cancelled = options(&metrics);
    cancelled.cancellation.cancel();
    let result = compile_rows(&engine, row(), cancelled).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "cancelled")
    );

    let (entered, _receiver) = tokio::sync::oneshot::channel();
    let compiler = Arc::new(Interpreter::new(PendingProvider {
        entered: Mutex::new(Some(entered)),
    }));
    let mut deadline = options(&metrics);
    deadline.timeout = Duration::from_millis(20);
    let result = compiler
        .compile_typed(
            &engine,
            "IDs",
            interpret_options(deadline, &interpreter_metrics),
        )
        .await;
    assert!(
        matches!(result.outcome, TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "deadline")
    );

    let (entered, receiver) = tokio::sync::oneshot::channel();
    let compiler = Arc::new(Interpreter::new(PendingProvider {
        entered: Mutex::new(Some(entered)),
    }));
    let task = {
        let engine = engine.clone();
        let metrics = metrics.clone();
        let interpreter_metrics = interpreter_metrics.clone();
        tokio::spawn(async move {
            compiler
                .compile_typed(
                    &engine,
                    "IDs",
                    interpret_options(options(&metrics), &interpreter_metrics),
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), receiver)
        .await
        .expect("provider must be entered")
        .unwrap();
    assert_eq!(interpreter_metrics.snapshot().in_flight, 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(interpreter_metrics.snapshot().in_flight, 0);

    struct Panics;
    impl ModelProvider for Panics {
        async fn complete(&self, _messages: &[Message]) -> Result<String, ProviderError> {
            panic!("test provider panic")
        }
    }
    let task = {
        let engine = engine.clone();
        let metrics = metrics.clone();
        let interpreter_metrics = interpreter_metrics.clone();
        tokio::spawn(async move {
            Interpreter::new(Panics)
                .compile_typed(
                    &engine,
                    "IDs",
                    interpret_options(options(&metrics), &interpreter_metrics),
                )
                .await
        })
    };
    assert!(task.await.unwrap_err().is_panic());
    let compiler_snapshot = metrics.snapshot();
    assert_eq!(compiler_snapshot.admitted, 1);
    assert_eq!(compiler_snapshot.completed, 1);
    assert_eq!(compiler_snapshot.cancelled, 1);
    let snapshot = interpreter_metrics.snapshot();
    assert_eq!(snapshot.admitted, 3);
    assert_eq!(snapshot.completed, 1);
    assert_eq!(snapshot.deadline_exceeded, 1);
    assert_eq!(snapshot.abandoned, 2);
    assert_eq!(snapshot.in_flight, 0);
}
