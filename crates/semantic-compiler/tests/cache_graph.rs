use std::sync::Arc;

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
    CompilationCacheOptions, CompilationSession, CompileOptions, TypedOutcome,
};
use semantic_engine::Engine;
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
        version: 1,
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
fn intent() -> GraphIntentQuery {
    GraphIntentQuery {
        query: graph(),
        evidence: GraphRequestEvidence {
            version: 1,
            request_id: "cache-graph".into(),
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
#[tokio::test]
async fn graph_cache_coalesces_and_rechecks_request_limits() {
    let engine = fixture();
    let session = CompilationSession::new(&engine, CompilationCacheOptions::default()).unwrap();
    let (first, second) = tokio::join!(
        session.compile_graph(graph(), CompileOptions::default()),
        session.compile_graph(graph(), CompileOptions::default()),
    );
    assert!(matches!(first.outcome, TypedOutcome::CompiledGraph { .. }));
    assert!(matches!(second.outcome, TypedOutcome::CompiledGraph { .. }));
    assert_eq!(session.stats().misses, 1);
    assert_eq!(session.stats().hits, 1);
    assert_ne!(first.record.compilation_id, second.record.compilation_id);
    assert_eq!(first.record.artifact_digest, second.record.artifact_digest);
    assert!(
        first
            .record
            .requirement_dispositions
            .iter()
            .all(|item| item.result == "lowered_and_verified")
    );
    assert!(
        second
            .record
            .requirement_dispositions
            .iter()
            .all(|item| item.result == "reused_validated")
    );
    let mut tight = CompileOptions::default();
    tight.max_sql_bytes = 1;
    assert!(
        matches!(session.compile_graph(graph(), tight).await.outcome, TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "sql_limit")
    );
    assert_eq!(session.stats().hits, 2);
}
#[tokio::test]
async fn graph_and_row_share_lru_but_scope_and_intent_do_not_reuse_artifacts() {
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
    assert!(matches!(
        session
            .compile_graph(graph(), CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::CompiledGraph { .. }
    ));
    let traced = session
        .compile_graph_intent(intent(), CompileOptions::default())
        .await;
    assert!(traced.record.request_spans_validated);
    assert!(matches!(traced.outcome, TypedOutcome::CompiledGraph { .. }));
    assert_eq!(session.stats().misses, 2);
    assert_eq!(session.stats().evictions, 1);
    let mut scoped = CompileOptions::default();
    scoped.allowed_relations = Some(["items".into()].into());
    let compiled = session.compile_graph(graph(), scoped.clone()).await;
    let TypedOutcome::CompiledGraph { query } = compiled.outcome else {
        panic!("{:?}", compiled.outcome)
    };
    assert_eq!(
        query.plan_direct(&engine).await.unwrap_err().code,
        "execution_scope"
    );
    assert!(
        query
            .plan_direct_authorized(&engine, &scoped.allowed_relations.unwrap())
            .await
            .is_ok()
    );
    assert_eq!(session.stats().entries, 1);
    assert!(matches!(
        session
            .compile(row(), CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::Compiled { .. }
    ));
    assert_eq!(session.stats().entries, 1);
    assert!(session.stats().retained_bytes <= 1024 * 1024);
}
