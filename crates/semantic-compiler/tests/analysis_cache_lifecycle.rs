use std::{
    future::pending,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use semantic_catalog::{Catalog, DataType, Field, Relation, Schema};
use semantic_compiler::typed::{
    AnalysisCache, AnalysisCacheError, AnalysisCacheIdentity, AnalysisCacheLimits, LookupDependency,
};

fn snapshot() -> Arc<semantic_catalog::CatalogSnapshot> {
    Catalog::from_relations([Relation::base(
        "items",
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
        "memory",
    )])
    .unwrap()
    .snapshot()
}

fn identity(key: &str) -> AnalysisCacheIdentity {
    AnalysisCacheIdentity {
        stage: "lifecycle-test".into(),
        input_digest: key.into(),
        access_scope_revision: "tenant-a".into(),
        parameter_digest: "none".into(),
        renderer_revision: "v1".into(),
        function_revision: "v1".into(),
        acceptance_revision: "v1".into(),
    }
}

fn dependency() -> Vec<LookupDependency> {
    vec![LookupDependency::Relation {
        name: "items".into(),
    }]
}

fn cache() -> Arc<AnalysisCache<u32>> {
    Arc::new(
        AnalysisCache::new(AnalysisCacheLimits {
            max_entries: 4,
            max_bytes: 16,
            max_lookups: 1,
            max_active_keys: 1,
        })
        .unwrap(),
    )
}

#[tokio::test]
async fn aborted_builder_releases_admission_and_same_key_waiter_recovers() {
    let snapshot = snapshot();
    let cache = cache();
    let (started, entered) = tokio::sync::oneshot::channel();
    let leader = {
        let snapshot = snapshot.clone();
        let cache = cache.clone();
        tokio::spawn(async move {
            cache
                .get_or_build(&snapshot, identity("held"), move || async move {
                    started.send(()).unwrap();
                    pending::<Result<(u32, Vec<LookupDependency>, usize), &'static str>>().await
                })
                .await
        })
    };
    entered.await.unwrap();
    assert_eq!(cache.stats().active_keys, 1);

    let rejected = tokio::time::timeout(
        Duration::from_millis(100),
        cache.get_or_build(&snapshot, identity("other"), || async {
            panic!("saturated distinct-key builder must not run");
            #[allow(unreachable_code)]
            Ok::<_, &'static str>((9, dependency(), 4))
        }),
    )
    .await
    .expect("admission decision must be prompt");
    assert!(matches!(rejected, Err(AnalysisCacheError::Admission)));

    let builds = Arc::new(AtomicUsize::new(0));
    let waiter = {
        let snapshot = snapshot.clone();
        let cache = cache.clone();
        let builds = builds.clone();
        tokio::spawn(async move {
            cache
                .get_or_build(&snapshot, identity("held"), move || async move {
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, &'static str>((7, dependency(), 4))
                })
                .await
        })
    };
    tokio::task::yield_now().await;
    assert_eq!(cache.stats().active_keys, 1);
    leader.abort();
    assert!(leader.await.unwrap_err().is_cancelled());
    let recovered = tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("same-key waiter must acquire the cancelled builder's lock")
        .unwrap()
        .unwrap();
    assert_eq!(*recovered, 7);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert_eq!(cache.stats().active_keys, 0);
    assert_eq!(cache.stats().entries, 1);

    let admitted = cache
        .get_or_build(&snapshot, identity("other"), || async {
            Ok::<_, &'static str>((9, dependency(), 4))
        })
        .await
        .unwrap();
    assert_eq!(*admitted, 9);
}

#[tokio::test]
async fn failed_build_retries_without_retaining_partial_value_or_active_lock() {
    let snapshot = snapshot();
    let cache = cache();
    let failed = cache
        .get_or_build(&snapshot, identity("metadata"), || async {
            Err::<(u32, Vec<LookupDependency>, usize), _>("metadata unavailable")
        })
        .await;
    assert!(matches!(
        failed,
        Err(AnalysisCacheError::Build("metadata unavailable"))
    ));
    assert_eq!(cache.stats().entries, 0);
    assert_eq!(cache.stats().active_keys, 0);

    let retried = cache
        .get_or_build(&snapshot, identity("metadata"), || async {
            Ok::<_, &'static str>((11, dependency(), 4))
        })
        .await
        .unwrap();
    assert_eq!(*retried, 11);
    assert_eq!(cache.stats().entries, 1);
    assert_eq!(cache.stats().active_keys, 0);

    for index in 0..32 {
        let value = cache
            .get_or_build(
                &snapshot,
                identity(&format!("pressure-{index}")),
                || async move { Ok::<_, &'static str>((index, dependency(), 4)) },
            )
            .await
            .unwrap();
        assert_eq!(*value, index);
        let stats = cache.stats();
        assert!(stats.entries <= 4);
        assert!(stats.retained_bytes <= 16);
        assert_eq!(stats.active_keys, 0);
    }
    assert!(cache.stats().evictions > 0);
}
