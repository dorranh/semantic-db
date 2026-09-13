use datafusion::{
    arrow::{
        array::{Int64Array, RecordBatch},
        datatypes::{DataType, Field, Schema},
    },
    physical_plan::{SendableRecordBatchStream, stream::RecordBatchStreamAdapter},
    prelude::SessionContext,
};
use futures::stream;
use semantic_materialization::*;
use semantic_runtime::{QueryContext, QueryOptions, SourceDescriptor, failure, unique_id};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn descriptor(scope: &str) -> SourceDescriptor {
    SourceDescriptor {
        scope: "test".into(),
        schema_revision: "v1".into(),
        authorization_scope: scope.into(),
        revision: "1".into(),
    }
}
fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new("x", DataType::Int64, false)]))
}
fn batch(value: i64) -> RecordBatch {
    RecordBatch::try_new(schema(), vec![Arc::new(Int64Array::from(vec![value]))]).unwrap()
}
fn fetch(value: i64) -> SendableRecordBatchStream {
    Box::pin(RecordBatchStreamAdapter::new(
        schema(),
        stream::iter(vec![Ok(batch(value))]),
    ))
}
fn context() -> Arc<QueryContext> {
    QueryContext::new(QueryOptions::default()).unwrap()
}
async fn values(value: Materialized) -> Vec<i64> {
    let session = SessionContext::new();
    session.register_table("cached", value.provider).unwrap();
    session
        .sql("SELECT x FROM cached")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap()
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect()
}
struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn manager(memory: usize) -> (Temp, Arc<MaterializationManager>) {
    let temp = Temp(std::env::temp_dir().join(format!("semantic-cache-{}", unique_id())));
    let manager = MaterializationManager::new(CacheOptions {
        directory: temp.0.clone(),
        max_memory_bytes: memory,
        max_disk_bytes: 1024 * 1024,
    })
    .unwrap();
    (temp, manager)
}
fn policy() -> MaterializationPolicy {
    MaterializationPolicy {
        max_age_seconds: 60,
        max_fill_bytes: 65536,
    }
}

#[tokio::test]
async fn publication_restart_authorization_and_pinned_generation() {
    for memory in [0, 65536] {
        let (_temp, manager) = manager(memory);
        let first = manager
            .resolve(
                &descriptor("reader-a"),
                &policy(),
                schema(),
                &context(),
                None,
                || Box::pin(async { Ok(fetch(1)) }),
            )
            .await
            .unwrap();
        let pinned = first.provider.clone();
        let restarted = MaterializationManager::new(manager.options().clone()).unwrap();
        let second = restarted
            .resolve(
                &descriptor("reader-a"),
                &policy(),
                schema(),
                &context(),
                None,
                || Box::pin(async { panic!("fresh disk cache must avoid source reads") }),
            )
            .await
            .unwrap();
        assert_eq!(first.manifest.generation, second.manifest.generation);
        assert_eq!(values(second).await, vec![1]);
        let other = manager
            .resolve(
                &descriptor("reader-b"),
                &policy(),
                schema(),
                &context(),
                None,
                || Box::pin(async { Ok(fetch(2)) }),
            )
            .await
            .unwrap();
        assert_ne!(first.manifest.key, other.manifest.key);
        assert_eq!(values(other).await, vec![2]);
        manager
            .invalidate(&first.manifest.key, &context())
            .await
            .unwrap();
        let fresh = manager
            .resolve(
                &descriptor("reader-a"),
                &policy(),
                schema(),
                &context(),
                None,
                || Box::pin(async { Ok(fetch(3)) }),
            )
            .await
            .unwrap();
        assert_ne!(first.manifest.generation, fresh.manifest.generation);
        assert_eq!(values(fresh).await, vec![3]);
        assert_eq!(
            values(Materialized {
                provider: pinned,
                manifest: first.manifest
            })
            .await,
            vec![1]
        );
    }
}
#[tokio::test]
async fn concurrent_fill_is_shared_and_failed_refresh_never_publishes() {
    let (_temp, manager) = manager(65536);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];
    for _ in 0..4 {
        let manager = manager.clone();
        let calls = calls.clone();
        handles.push(tokio::spawn(async move {
            manager
                .resolve(
                    &descriptor("a"),
                    &policy(),
                    schema(),
                    &context(),
                    None,
                    move || {
                        Box::pin(async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            Ok(fetch(7))
                        })
                    },
                )
                .await
                .unwrap()
        }));
    }
    let mut generations = vec![];
    for handle in handles {
        generations.push(handle.await.unwrap().manifest.generation);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(generations.iter().all(|g| g == &generations[0]));
    manager
        .invalidate(&descriptor("a").cache_key(), &context())
        .await
        .unwrap();
    let error = manager
        .resolve(
            &descriptor("a"),
            &policy(),
            schema(),
            &context(),
            None,
            || {
                Box::pin(async {
                    Ok(Box::pin(RecordBatchStreamAdapter::new(
                        schema(),
                        stream::iter(vec![Ok(batch(8)), Err(failure("late source failure"))]),
                    )) as SendableRecordBatchStream)
                })
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("late source failure"));
    assert!(manager.status().unwrap().is_empty());
    let cancelled = context();
    cancelled.cancel();
    assert!(
        manager
            .resolve(
                &descriptor("a"),
                &policy(),
                schema(),
                &cancelled,
                None,
                || Box::pin(async { Ok(fetch(9)) })
            )
            .await
            .is_err()
    );
    assert!(manager.status().unwrap().is_empty());
}
