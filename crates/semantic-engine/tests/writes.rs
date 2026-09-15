use datafusion::{
    arrow::{
        array::{Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
    },
    common::ScalarValue,
    datasource::MemTable,
};
use futures::future::BoxFuture;
use semantic_engine::*;
use semantic_runtime::staging::StagedInput;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
struct Fake {
    schema: Arc<Schema>,
    calls: Mutex<usize>,
    outcome: WriteOutcome,
}
impl WriteConnection for Fake {
    fn inspect_target<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<TargetInspection>> {
        Box::pin(async move {
            Ok(TargetInspection {
                physical_namespace: "fake".into(),
                schema: self.schema.clone(),
                revision: "v1".into(),
                resource: "physical".into(),
                domain: "commit".into(),
                unique_keys: vec![vec!["id".into()]],
                checked_eligible: true,
                supported_operations: vec![
                    "INSERT".into(),
                    "UPDATE".into(),
                    "DELETE".into(),
                    "MERGE".into(),
                ],
                atomic_writes: true,
            })
        })
    }
    fn validate_operation<'a>(&'a self, _: &'a MutationPlan) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn apply<'a>(
        &'a self,
        plan: &'a MutationPlan,
        input: &'a StagedInput,
        _: &'a [ScalarValue],
        _: Arc<QueryContext>,
    ) -> BoxFuture<'a, Result<WriteResult>> {
        Box::pin(async move {
            for batch in input.batches() {
                batch?;
            }
            *self.calls.lock().unwrap() += 1;
            Ok(WriteResult {
                outcome: self.outcome,
                operation_id: "op".into(),
                boundary: "commit".into(),
                atomic: true,
                idempotent: plan.checked,
                receipt: None,
                affected_rows: Some(input.rows as u64),
                code: None,
                message: None,
                external_observations: vec![],
            })
        })
    }
    fn begin<'a>(
        &'a self,
        _: TransactionOptions,
    ) -> BoxFuture<'a, Result<Arc<dyn ConnectorSession>>> {
        Box::pin(async { Err(semantic_runtime::failure("unsupported transaction").into()) })
    }
}
async fn setup(outcome: WriteOutcome) -> (Engine, Arc<Fake>) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("label", DataType::Utf8, true),
    ]));
    let table = Arc::new(MemTable::try_new(schema.clone(), vec![vec![]]).unwrap());
    let mut engine = Engine::new();
    engine
        .register_table(Relation::base("items", schema.clone(), "fake"), table)
        .unwrap();
    let fake = Arc::new(Fake {
        schema,
        calls: Mutex::new(0),
        outcome,
    });
    engine
        .attach_write_binding(
            "items",
            WriteBinding {
                connection: fake.clone(),
                target: "physical".into(),
                columns: BTreeMap::from([
                    ("id".into(), "id".into()),
                    ("label".into(), "label".into()),
                ]),
            },
        )
        .await
        .unwrap();
    (engine, fake)
}
const MERGE: &str = "/*leading*/ require /*modifier*/ idempotent MERGE INTO items AS t USING (SELECT $1::bigint AS id, $2::text AS label) AS s ON t.id=s.id WHEN MATCHED THEN UPDATE SET label=s.label WHEN NOT MATCHED THEN INSERT(id,label) VALUES(s.id,s.label);";
#[tokio::test]
async fn writable_aliases_and_unknown_domains_bypass_cache_but_disjoint_tables_cache() {
    let (mut engine, fake) = setup(WriteOutcome::Committed).await;
    let directory = std::env::temp_dir().join(semantic_runtime::unique_id());
    engine
        .configure_cache(CacheOptions {
            directory: directory.clone(),
            max_memory_bytes: 65536,
            max_disk_bytes: 1024 * 1024,
        })
        .unwrap();
    for (name, domain, resource, expected_cache) in [
        ("alias", "commit", "physical", false),
        ("alternate", "another_connection", "unproven", false),
        ("unrelated", "commit", "other_table", true),
    ] {
        engine
            .register_table(
                Relation::base(name, fake.schema.clone(), "fake"),
                Arc::new(MemTable::try_new(fake.schema.clone(), vec![vec![]]).unwrap()),
            )
            .unwrap();
        engine
            .attach_resource_identity(
                name,
                ResourceIdentity {
                    namespace: "fake".into(),
                    domain: Some(domain.into()),
                    resource: Some(resource.into()),
                },
            )
            .unwrap();
        let view = format!("{name}_view");
        engine
            .create_view(&view, &format!("SELECT * FROM {name}"))
            .await
            .unwrap();
        engine
            .materialize(
                &view,
                MaterializationPolicy {
                    max_age_seconds: 60,
                    max_fill_bytes: 65536,
                },
            )
            .unwrap();
        let result = engine
            .execute_read(
                &format!("SELECT * FROM {view}"),
                vec![],
                ReadOptions::default(),
            )
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(!result.report.caches.is_empty(), expected_cache, "{name}");
    }
    std::fs::remove_dir_all(directory).unwrap();
}
#[tokio::test]
async fn parser_parameters_explanation_and_outcome_preservation() {
    let (engine, fake) = setup(WriteOutcome::OutcomeUnknown).await;
    let description = engine.describe_write(MERGE, &[]).await.unwrap();
    assert_eq!(
        description.parameters,
        vec![DataType::Int64, DataType::Utf8View]
    );
    let p = engine.prepare_write(MERGE).await.unwrap();
    let explanation = p.explain().await.unwrap();
    assert!(explanation.require_idempotent);
    assert_eq!(*fake.calls.lock().unwrap(), 0);
    let result = p
        .execute(
            vec![
                ScalarValue::Int64(Some(7)),
                ScalarValue::Utf8(Some("'; DELETE FROM items; --".into())),
            ],
            WriteOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.outcome, WriteOutcome::OutcomeUnknown);
    assert_eq!(*fake.calls.lock().unwrap(), 1);
    assert!(p.execute(vec![], WriteOptions::default()).await.is_err());
    assert!(
        p.execute(
            vec![],
            WriteOptions {
                atomicity: Atomicity::BestEffort,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    for bad in [
        "BEGIN",
        "DELETE FROM items; SELECT 1",
        "REQUIRE IDEMPOTENT UPDATE items SET label='x'",
        "MERGE INTO items t USING (SELECT 1 AS id, 'x' AS label) s ON t.id=s.id WHEN MATCHED THEN DELETE",
        "REQUIRE IDEMPOTENT MERGE INTO items t USING (SELECT id,label FROM items) s ON t.id=s.id WHEN MATCHED THEN UPDATE SET label=s.label",
        "REQUIRE IDEMPOTENT MERGE INTO items t USING (SELECT random() AS id,'x' AS label) s ON t.id=s.id WHEN MATCHED THEN UPDATE SET label=s.label",
    ] {
        assert!(engine.prepare_write(bad).await.is_err(), "{bad}");
    }
    assert!(
        engine
            .prepare_write(&format!("EXPLAIN {MERGE}"))
            .await
            .unwrap()
            .execute(vec![], WriteOptions::default())
            .await
            .is_err()
    );
    assert_eq!(*fake.calls.lock().unwrap(), 1);
}
#[tokio::test]
async fn observed_completion_and_snapshot_rejection() {
    let mut e = Engine::new();
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1]))]).unwrap();
    e.register_table(
        Relation::base("source", schema.clone(), "memory"),
        Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
    )
    .unwrap();
    assert!(
        e.execute_read(
            "SELECT * FROM source",
            vec![],
            ReadOptions {
                consistency: ReadConsistency::Snapshot,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    let execution = e
        .execute_read("SELECT * FROM source", vec![], ReadOptions::default())
        .await
        .unwrap();
    let report = execution.report.clone();
    drop(execution);
    assert_eq!(report.lock().unwrap().completion, ReadCompletion::Abandoned);
    let constant = e
        .execute_read(
            "SELECT 1",
            vec![],
            ReadOptions {
                consistency: ReadConsistency::Snapshot,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(constant.report.established, ReadConsistency::Snapshot);
    e.create_view("constant", "SELECT 1").await.unwrap();
    assert!(
        e.begin_read_session(&["constant".into()], ReadSessionOptions::default())
            .await
            .is_err()
    );
    let result = e
        .execute_read("SELECT * FROM source", vec![], ReadOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(result.report.completion, ReadCompletion::Complete);
    assert_eq!(result.report.external_observations, vec!["source"]);
}
#[tokio::test]
async fn staging_spills_replays_and_rejects_exhaustion() {
    use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
    use semantic_runtime::staging::{StagedInput, StagingOptions};
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Utf8,
        false,
    )]));
    let b = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(StringArray::from(vec!["fixed input"]))],
    )
    .unwrap();
    let stream = || {
        Box::pin(RecordBatchStreamAdapter::new(
            schema.clone(),
            futures::stream::iter(vec![Ok(b.clone())]),
        )) as datafusion::physical_plan::SendableRecordBatchStream
    };
    let context = QueryContext::new(QueryOptions::default()).unwrap();
    let input = StagedInput::collect(
        stream(),
        &StagingOptions {
            memory_bytes: 0,
            disk_bytes: 4096,
        },
        &context,
    )
    .await
    .unwrap();
    assert_eq!(
        input
            .batches()
            .collect::<datafusion::error::Result<Vec<_>>>()
            .unwrap(),
        vec![b.clone()]
    );
    assert_eq!(
        input
            .batches()
            .collect::<datafusion::error::Result<Vec<_>>>()
            .unwrap(),
        vec![b.clone()]
    );
    assert!(
        StagedInput::collect(
            stream(),
            &StagingOptions {
                memory_bytes: 0,
                disk_bytes: 1
            },
            &context
        )
        .await
        .is_err()
    );
}

#[derive(Debug)]
struct LateFailure {
    schema: Arc<Schema>,
}
impl datafusion::physical_plan::streaming::PartitionStream for LateFailure {
    fn schema(&self) -> &Arc<Schema> {
        &self.schema
    }
    fn execute(
        &self,
        _: Arc<datafusion::execution::TaskContext>,
    ) -> datafusion::physical_plan::SendableRecordBatchStream {
        let b = RecordBatch::try_new(
            self.schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1])),
                Arc::new(StringArray::from(vec!["partial"])),
            ],
        )
        .unwrap();
        Box::pin(
            datafusion::physical_plan::stream::RecordBatchStreamAdapter::new(
                self.schema.clone(),
                futures::stream::iter(vec![
                    Ok(b),
                    Err(semantic_runtime::failure("late source failure")),
                ]),
            ),
        )
    }
}
#[tokio::test]
async fn late_failure_cannot_publish_or_become_complete() {
    use futures::StreamExt;
    let (mut engine, fake) = setup(WriteOutcome::Committed).await;
    let source = Arc::new(
        datafusion::catalog::streaming::StreamingTable::try_new(
            fake.schema.clone(),
            vec![Arc::new(LateFailure {
                schema: fake.schema.clone(),
            })],
        )
        .unwrap(),
    );
    engine
        .register_table(
            Relation::base("source", fake.schema.clone(), "fault"),
            source,
        )
        .unwrap();
    engine
        .attach_resource_identity(
            "source",
            ResourceIdentity {
                namespace: "fault".into(),
                domain: None,
                resource: None,
            },
        )
        .unwrap();
    let p=engine.prepare_write("REQUIRE IDEMPOTENT MERGE INTO items t USING (SELECT id,label FROM source) s ON t.id=s.id WHEN MATCHED THEN UPDATE SET label=s.label").await.unwrap();
    assert!(p.execute(vec![], WriteOptions::default()).await.is_err());
    assert_eq!(*fake.calls.lock().unwrap(), 0);
    let mut execution = engine
        .execute_read("SELECT * FROM source", vec![], ReadOptions::default())
        .await
        .unwrap();
    assert!(execution.stream.next().await.unwrap().is_ok());
    assert!(execution.stream.next().await.unwrap().is_err());
    assert_eq!(
        execution.report.lock().unwrap().completion,
        ReadCompletion::Failed
    );
    assert!(execution.stream.next().await.is_none());
    assert_eq!(
        execution.report.lock().unwrap().completion,
        ReadCompletion::Failed
    );
    assert!(execution.collect().await.is_err());
    let execution = engine
        .execute_read("SELECT * FROM source", vec![], ReadOptions::default())
        .await
        .unwrap();
    let report = execution.report.clone();
    execution.cancel();
    drop(execution);
    assert_eq!(report.lock().unwrap().completion, ReadCompletion::Cancelled);
}
