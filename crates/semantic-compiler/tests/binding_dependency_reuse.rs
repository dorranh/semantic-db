use std::{collections::BTreeSet, sync::Arc};

use datafusion::{
    arrow::{
        array::{Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{ConceptDefinition, Relation, RelationSemantics};
use semantic_compiler::typed::{
    Calendar, CompilationCacheOptions, CompilationSession, CompileOptions, ContextOrigin,
    RequestContext, TypedOutcome,
};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::graph::{GraphOperation, GraphQuery, QueryNode};
use semantic_plan::typed::*;

fn concept(id: &str, alias: &str, value: &str) -> ConceptDefinition {
    ConceptDefinition {
        id: format!("concepts/{id}"),
        description: id.into(),
        aliases: vec![alias.into()],
        alternatives: vec![],
        predicate: RowPredicate::Compare {
            field: "state".into(),
            operator: Comparison::Eq,
            value: Literal::Utf8(value.into()),
        },
        source_refs: vec![],
    }
}

fn engine(unrelated: bool, competing: bool) -> Arc<Engine> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("state", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["open", "closed"])),
        ],
    )
    .unwrap();
    let mut relation = Relation::base("items", schema.clone(), "memory:items");
    let mut concepts =
        std::collections::BTreeMap::from([("open".into(), concept("open", "live", "open"))]);
    if competing {
        concepts.insert("closed".into(), concept("closed", "live", "closed"));
    }
    relation.semantics = Some(RelationSemantics {
        concepts,
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            relation,
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
        )
        .unwrap();
    if unrelated {
        let other_schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int64, false)]));
        engine
            .register_table(
                Relation::base("other", other_schema.clone(), "memory:other"),
                Arc::new(MemTable::try_new(other_schema, vec![vec![]]).unwrap()),
            )
            .unwrap();
    }
    Arc::new(engine)
}

fn query() -> RowQuery {
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "items".into(),
            instance: "i".into(),
        },
        requirements: vec![
            Requirement {
                id: "live".into(),
                source_text: "only live items".into(),
                operation: RowOperation::ConceptFilter {
                    concept: "live".into(),
                    arguments: Default::default(),
                },
            },
            Requirement {
                id: "id".into(),
                source_text: "item identifiers".into(),
                operation: RowOperation::Project {
                    field: FieldRef {
                        instance: "i".into(),
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
async fn bound_analysis_reuses_unrelated_publication_but_not_new_alias_competitor() {
    let first_engine = engine(false, false);
    let mut session =
        CompilationSession::from_shared(first_engine, CompilationCacheOptions::default()).unwrap();
    let first = session.compile(query(), CompileOptions::default()).await;
    assert!(matches!(first.outcome, TypedOutcome::Compiled { .. }));
    assert_eq!(first.record.cache_status, "miss");

    let next_engine = engine(true, false);
    session.replace_engine(next_engine.clone());
    let second = session.compile(query(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: artifact } = second.outcome else {
        panic!(
            "expected current-snapshot compilation: {:?}",
            second.outcome
        );
    };
    assert_eq!(second.record.cache_status, "binding_hit_revalidated");
    assert_ne!(first.record.snapshot_id, second.record.snapshot_id);
    assert_eq!(
        artifact.bound().snapshot_id(),
        next_engine.catalog().snapshot().id()
    );
    let direct = artifact
        .plan_direct(&next_engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = artifact
        .execute(&next_engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    for batches in [&direct, &sql] {
        let ids = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(
            (0..ids.len()).map(|row| ids.value(row)).collect::<Vec<_>>(),
            vec![1]
        );
    }

    let mut scoped = CompileOptions::default();
    scoped.allowed_relations = Some(BTreeSet::from(["items".into()]));
    assert_eq!(
        session.compile(query(), scoped).await.record.cache_status,
        "miss"
    );

    let mut contextual = CompileOptions::default();
    contextual.request_context = Some(RequestContext {
        reference_unix_millis: 0,
        timezone: "UTC".into(),
        calendar: Calendar::Gregorian,
        origin: ContextOrigin::Caller,
    });
    assert_eq!(
        session
            .compile(query(), contextual)
            .await
            .record
            .cache_status,
        "miss"
    );

    let mut profile = CompileOptions::default();
    profile.max_nodes = 256;
    assert_eq!(
        session.compile(query(), profile).await.record.cache_status,
        "miss"
    );

    session.replace_engine(engine(true, true));
    let competitor = session.compile(query(), CompileOptions::default()).await;
    assert!(
        matches!(competitor.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "ambiguous_concept")
    );
}

fn paged_query() -> RowQuery {
    let mut request = query();
    request.requirements.push(Requirement {
        id: "order".into(),
        source_text: "order by identifier".into(),
        operation: RowOperation::Order {
            field: FieldRef {
                instance: "i".into(),
                field: "id".into(),
            },
            direction: Direction::Asc,
            nulls: NullOrder::Last,
        },
    });
    request.requirements.push(Requirement {
        id: "page".into(),
        source_text: "first two rows".into(),
        operation: RowOperation::Page {
            offset: 0,
            fetch: 2,
        },
    });
    request
}

#[tokio::test]
async fn stricter_page_cap_cannot_reuse_permissive_row_or_graph_artifact() {
    let session =
        CompilationSession::from_shared(engine(false, false), CompilationCacheOptions::default())
            .unwrap();
    let page = paged_query();
    assert!(matches!(
        session
            .compile(page.clone(), CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::Compiled { .. }
    ));
    let mut strict = CompileOptions::default();
    strict.max_page_fetch = 1;
    let result = session.compile(page.clone(), strict).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "page_limit")
    );

    let graph = GraphQuery {
        version: 1,
        nodes: vec![QueryNode {
            id: "rows".into(),
            source_text: "paged live items".into(),
            operation: GraphOperation::Rows { query: page },
        }],
        root: "rows".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    };
    assert!(matches!(
        session
            .compile_graph(graph.clone(), CompileOptions::default())
            .await
            .outcome,
        TypedOutcome::CompiledGraph { .. }
    ));
    let mut strict = CompileOptions::default();
    strict.max_page_fetch = 1;
    let result = session.compile_graph(graph, strict).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "page_limit")
    );
}
