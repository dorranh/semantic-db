use std::{collections::BTreeSet, sync::Arc};

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_graph, compile_rows};
use semantic_engine::{Engine, MVP_EXECUTION_PROFILE_REVISION, QueryOptions, ReadOptions};
use semantic_plan::{
    graph::{GraphOperation, GraphQuery, QueryNode},
    typed::{
        Comparison, FieldRef, Literal, RelationInput, Requirement, RowOperation, RowPredicate,
        RowQuery,
    },
};

fn fixture() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(StringArray::from(vec!["open", "closed"])) as ArrayRef,
        ],
    )
    .unwrap();
    let mut engine = Engine::new();
    engine
        .register_table(
            Relation::base("items", schema.clone(), "fixture:items"),
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
            instance: "i".into(),
        },
        requirements: vec![
            Requirement {
                id: "filter".into(),
                source_text: "open".into(),
                operation: RowOperation::Filter {
                    predicate: RowPredicate::Compare {
                        field: FieldRef {
                            instance: "i".into(),
                            field: "label".into(),
                        },
                        operator: Comparison::Eq,
                        value: Literal::Utf8("open".into()),
                    },
                },
            },
            Requirement {
                id: "id".into(),
                source_text: "id".into(),
                operation: RowOperation::Project {
                    field: FieldRef {
                        instance: "i".into(),
                        field: "id".into(),
                    },
                    alias: "item_id".into(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn graph() -> GraphQuery {
    GraphQuery {
        version: 1,
        nodes: vec![QueryNode {
            id: "leaf".into(),
            source_text: "open item ids".into(),
            operation: GraphOperation::Rows { query: row() },
        }],
        root: "leaf".into(),
        ordering: vec![],
        limit: None,
        unresolved: vec![],
    }
}

fn scoped() -> (CompileOptions, BTreeSet<String>) {
    let allowed = BTreeSet::from(["items".to_owned()]);
    let mut options = CompileOptions::default();
    options.allowed_relations = Some(allowed.clone());
    (options, allowed)
}

#[tokio::test]
async fn row_artifact_metadata_and_all_execution_boundaries_check_current_validity() {
    let mut engine = fixture();
    let (options, allowed) = scoped();
    let result = compile_rows(&engine, row(), options).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let sql = query.sql();
    assert_eq!(sql.target(), "local:datafusion-55");
    assert_eq!(
        sql.execution_profile_revision(),
        MVP_EXECUTION_PROFILE_REVISION
    );
    assert_eq!(sql.snapshot_id(), engine.catalog().snapshot().id());
    assert_eq!(sql.parameters(), &[Literal::Utf8("open".into())]);
    assert_eq!(sql.parameter_types(), ["Utf8"]);
    assert_eq!(sql.expected_output().len(), 1);
    assert_eq!(sql.expected_output()[0].name, "item_id");
    assert_eq!(sql.expected_output()[0].data_type, "Int64");
    assert!(!sql.expected_output()[0].nullable);
    assert!(!sql.statement().contains("'open'"));

    let revoked = BTreeSet::new();
    assert_eq!(
        query.plan_direct(&engine).await.err().unwrap().code,
        "execution_scope"
    );
    assert_eq!(
        query
            .execute(&engine, QueryOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    assert_eq!(
        query
            .execute_read(&engine, ReadOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    assert_eq!(
        query
            .plan_direct_authorized(&engine, &revoked)
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    assert_eq!(
        query
            .execute_authorized(&engine, &revoked, QueryOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    assert_eq!(
        query
            .execute_read_authorized(&engine, &revoked, ReadOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    query
        .plan_direct_authorized(&engine, &allowed)
        .await
        .unwrap();

    let mut replay = query.capture_replay(128 * 1024).unwrap();
    replay.execution_profile_revision = "stale-profile".into();
    assert_eq!(
        replay
            .replay(&engine, CompileOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "replay_profile"
    );

    engine
        .create_view("later", "SELECT id FROM items")
        .await
        .unwrap();
    assert_eq!(
        query
            .plan_direct_authorized(&engine, &allowed)
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
    assert_eq!(
        query
            .execute_authorized(&engine, &allowed, QueryOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
    assert_eq!(
        query
            .execute_read_authorized(&engine, &allowed, ReadOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
    replay.execution_profile_revision = MVP_EXECUTION_PROFILE_REVISION.into();
    assert_eq!(
        replay
            .replay(&engine, CompileOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
}

#[tokio::test]
async fn graph_artifact_checks_revocation_snapshot_and_replay_profile() {
    let mut engine = fixture();
    let (options, allowed) = scoped();
    let result = compile_graph(&engine, graph(), options).await;
    let TypedOutcome::CompiledGraph { query } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    assert_eq!(
        query.sql().execution_profile_revision(),
        MVP_EXECUTION_PROFILE_REVISION
    );
    assert_eq!(query.sql().snapshot_id(), engine.catalog().snapshot().id());

    let revoked = BTreeSet::new();
    assert_eq!(
        query.plan_direct(&engine).await.err().unwrap().code,
        "execution_scope"
    );
    assert_eq!(
        query
            .execute(&engine, QueryOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    assert_eq!(
        query
            .execute_read(&engine, ReadOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    assert_eq!(
        query
            .plan_direct_authorized(&engine, &revoked)
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    assert_eq!(
        query
            .execute_authorized(&engine, &revoked, QueryOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    assert_eq!(
        query
            .execute_read_authorized(&engine, &revoked, ReadOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "execution_scope"
    );
    query
        .plan_direct_authorized(&engine, &allowed)
        .await
        .unwrap();

    let mut replay = query.capture_replay(128 * 1024).unwrap();
    replay.execution_profile_revision = "stale-profile".into();
    assert_eq!(
        replay
            .replay(&engine, CompileOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "replay_profile"
    );
    engine
        .create_view("later", "SELECT id FROM items")
        .await
        .unwrap();
    assert_eq!(
        query
            .plan_direct_authorized(&engine, &allowed)
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
    assert_eq!(
        query
            .execute_authorized(&engine, &allowed, QueryOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
    assert_eq!(
        query
            .execute_read_authorized(&engine, &allowed, ReadOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
    replay.execution_profile_revision = MVP_EXECUTION_PROFILE_REVISION.into();
    assert_eq!(
        replay
            .replay(&engine, CompileOptions::default())
            .await
            .err()
            .unwrap()
            .code,
        "snapshot_mismatch"
    );
}
