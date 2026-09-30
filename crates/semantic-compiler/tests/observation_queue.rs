use std::sync::Arc;

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{
    CompileDiagnostic, CompileOptions, CompilerMetrics, ObservationOutcome, ObservationQueue,
    TypedOutcome, compile_graph, compile_rows, diagnostic_details,
};
use semantic_engine::Engine;
use semantic_plan::graph::{GraphOperation, GraphQuery, QueryNode};
use semantic_plan::typed::{FieldRef, RelationInput, Requirement, RowOperation, RowQuery};

const SECRET: &str = "private_customer_token_739";

#[test]
fn diagnostic_debug_omits_sensitive_message() {
    let diagnostic = CompileDiagnostic {
        code: "invalid_proposal".into(),
        message: SECRET.into(),
        details: diagnostic_details("invalid_proposal"),
    };
    let debug = format!("{diagnostic:?}");
    assert!(debug.contains("invalid_proposal"));
    assert!(!debug.contains(SECRET));
}

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let provider = Arc::new(MemTable::try_new(schema.clone(), vec![vec![]]).unwrap());
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base(SECRET, schema, "secret:/credentials/path"),
            provider,
        )
        .unwrap();
    engine
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: SECRET.into(),
            instance: "r".into(),
        },
        requirements: vec![Requirement {
            id: "sensitive_requirement".into(),
            source_text: "my private amount".into(),
            operation: RowOperation::Project {
                field: FieldRef {
                    instance: "r".into(),
                    field: "value".into(),
                },
                alias: "value".into(),
            },
        }],
        unresolved: vec![],
    }
}

#[tokio::test]
async fn direct_artifact_debug_omits_request_sql_and_output_names() {
    let engine = fixture();
    let mut row = query();
    let RowOperation::Project { alias, .. } = &mut row.requirements[0].operation else {
        unreachable!()
    };
    *alias = SECRET.into();
    let compiled = compile_rows(&engine, row.clone(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: artifact } = compiled.outcome else {
        panic!("expected row artifact: {:?}", compiled.outcome)
    };
    let rendered = format!(
        "{artifact:?} {:?} {:?} {:?} {:?}",
        artifact.bound(),
        artifact.relational(),
        artifact.sql(),
        artifact.sql().expected_output()[0]
    );
    for sensitive in [
        SECRET,
        "sensitive_requirement",
        "my private amount",
        "credentials",
    ] {
        assert!(
            !rendered.contains(sensitive),
            "artifact Debug leaked {sensitive}"
        );
    }

    let graph = GraphQuery {
        version: 1,
        nodes: vec![QueryNode {
            id: "sensitive_node".into(),
            source_text: "my private graph".into(),
            operation: GraphOperation::Rows { query: row },
        }],
        root: "sensitive_node".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    };
    let compiled = compile_graph(&engine, graph, CompileOptions::default()).await;
    let TypedOutcome::CompiledGraph { query: artifact } = compiled.outcome else {
        panic!("expected graph artifact: {:?}", compiled.outcome)
    };
    let capture = artifact.capture_replay(64 * 1024).unwrap();
    let rendered = format!("{artifact:?} {:?} {capture:?}", artifact.sql());
    for sensitive in [
        SECRET,
        "sensitive_node",
        "sensitive_requirement",
        "my private graph",
    ] {
        assert!(
            !rendered.contains(sensitive),
            "graph Debug leaked {sensitive}"
        );
    }
}

#[tokio::test]
async fn optional_queue_preserves_compilation_and_never_emits_sensitive_text() {
    let engine = fixture();
    let baseline = compile_rows(&engine, query(), CompileOptions::default()).await;
    assert!(matches!(baseline.outcome, TypedOutcome::Compiled { .. }));

    let (queue, mut receiver) = ObservationQueue::new(1).unwrap();
    let metrics = Arc::new(CompilerMetrics::with_observation_queue(queue.clone()));
    let mut options = CompileOptions::default();
    options.metrics = Some(metrics.clone());
    let observed = compile_rows(&engine, query(), options.clone()).await;
    assert!(matches!(observed.outcome, TypedOutcome::Compiled { .. }));
    assert_eq!(
        observed.record.artifact_digest,
        baseline.record.artifact_digest
    );
    let event = receiver.try_recv().expect("first observation");
    assert_eq!(event.outcome, ObservationOutcome::Compiled);
    let rendered = format!(
        "{event:?} {} {queue:?} {observed:?}",
        serde_json::to_string(&event).unwrap()
    );
    for sensitive in [
        SECRET,
        "sensitive_requirement",
        "my private amount",
        "credentials",
    ] {
        assert!(
            !rendered.contains(sensitive),
            "observation leaked {sensitive}"
        );
    }

    // Stop consuming; the next successful compilation must not await the sink.
    let second = compile_rows(&engine, query(), options.clone()).await;
    let third = compile_rows(&engine, query(), options).await;
    assert!(matches!(second.outcome, TypedOutcome::Compiled { .. }));
    assert!(matches!(third.outcome, TypedOutcome::Compiled { .. }));
    assert_eq!(
        third.record.artifact_digest,
        baseline.record.artifact_digest
    );
    assert_eq!(metrics.snapshot().dropped_observations, 1);
}

#[tokio::test]
async fn concurrent_observations_are_bounded_and_disconnection_is_nonblocking() {
    assert!(ObservationQueue::new(0).is_err());
    assert!(ObservationQueue::new(65_537).is_err());
    let engine = Arc::new(fixture());
    let (queue, receiver) = ObservationQueue::new(1).unwrap();
    drop(receiver);
    let metrics = Arc::new(CompilerMetrics::with_observation_queue(queue));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let engine = engine.clone();
        let metrics = metrics.clone();
        tasks.push(tokio::spawn(async move {
            let mut options = CompileOptions::default();
            options.metrics = Some(metrics);
            compile_rows(&engine, query(), options).await
        }));
    }
    for task in tasks {
        assert!(matches!(
            task.await.unwrap().outcome,
            TypedOutcome::Compiled { .. }
        ));
    }
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.completed, 8);
    assert_eq!(snapshot.dropped_observations, 8);
}
