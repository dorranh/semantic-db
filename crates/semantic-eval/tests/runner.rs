use semantic_eval::*;
fn tiny() -> Dataset {
    Dataset::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny/manifest.json"),
    )
    .unwrap()
}
fn output() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "semantic-eval-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
#[test]
fn artifact_loading_and_repeated_selection() {
    let d = tiny();
    assert_eq!(d.cases.len(), 1);
    assert!(select_cases(&d, &["duplicates".into(), "duplicates".into()]).is_err());
    assert!(select_cases(&d, &["unknown".into()]).is_err());
    assert_eq!(select_cases(&d, &[]).unwrap().len(), 1);
}
#[tokio::test]
async fn standalone_csv_executes_and_reports() {
    let d = tiny();
    let path = output();
    let r = run(
        &d,
        RunOptions {
            interface: Interface::Sql,
            artifacts: path.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(r.success(), "{r:?}");
    assert!(r.report_path.exists());
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test]
async fn setup_failure_marks_every_required_attempt_incomplete() {
    let mut d = tiny();
    d.manifest.project = "missing.yaml".into();
    let path = output();
    let r = run(
        &d,
        RunOptions {
            interface: Interface::Both,
            repetitions: 2,
            artifacts: path.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(!r.success());
    assert!(!r.complete);
    assert_eq!(r.cases.len(), 4);
    assert!(r.finalized);
    assert_eq!(r.expected_case_attempts, 4);
    assert_eq!(r.completed_case_attempts, 4);
    assert!(r.cases.iter().all(|c| !c.passed && c.incomplete));
    assert!(r.setup_error.is_some());
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test]
async fn filtered_release_is_rejected() {
    let d = tiny();
    assert!(
        run(
            &d,
            RunOptions {
                release: true,
                cases: vec!["duplicates".into()],
                repetitions: 3,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
}
struct Scripted(&'static str);
impl semantic_interpreter::provider::ModelProvider for Scripted {
    async fn complete(
        &self,
        messages: &[semantic_interpreter::provider::Message],
    ) -> std::result::Result<String, semantic_interpreter::provider::ProviderError> {
        assert!(
            !messages
                .iter()
                .any(|m| m.content.contains("SELECT id FROM samples ORDER BY id"))
        );
        assert!(
            !messages
                .iter()
                .any(|m| m.content.contains("distinguishing_rows"))
        );
        Ok(self.0.into())
    }
}
#[tokio::test]
async fn scripted_clarification_is_not_a_result_pass() {
    let d = tiny();
    let path = output();
    let r = run_with_provider(
        &d,
        RunOptions {
            interface: Interface::Ask,
            artifacts: path.clone(),
            ..Default::default()
        },
        Scripted(
            r#"{"status":"needs_clarification","phrases":["sample"],"question":"Which sample?"}"#,
        ),
    )
    .await
    .unwrap();
    assert!(!r.success());
    assert_eq!(r.cases.len(), 1);
    assert!(!r.cases[0].passed);
    assert!(r.cases[0].compilation.is_some());
    std::fs::remove_dir_all(path).unwrap();
}
struct FailedProvider;
impl semantic_interpreter::provider::ModelProvider for FailedProvider {
    async fn complete(
        &self,
        _: &[semantic_interpreter::provider::Message],
    ) -> std::result::Result<String, semantic_interpreter::provider::ProviderError> {
        Err(semantic_interpreter::provider::ProviderError::Transport)
    }
}
#[tokio::test]
async fn provider_failure_is_incomplete() {
    let d = tiny();
    let path = output();
    let r = run_with_provider(
        &d,
        RunOptions {
            interface: Interface::Ask,
            artifacts: path.clone(),
            ..Default::default()
        },
        FailedProvider,
    )
    .await
    .unwrap();
    assert!(!r.complete);
    assert!(r.cases[0].incomplete);
    assert_eq!(r.cases[0].outcome, "provider_failure");
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test]
async fn filtered_development_passes_but_has_no_full_coverage() {
    let d = tiny();
    let path = output();
    let r = run(
        &d,
        RunOptions {
            interface: Interface::Sql,
            cases: vec!["duplicates".into()],
            artifacts: path.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(r.success(), "{r:?}");
    assert!(!r.full_coverage);
    std::fs::remove_dir_all(path).unwrap();
}
struct OwnedScript(String);
impl semantic_interpreter::provider::ModelProvider for OwnedScript {
    async fn complete(
        &self,
        _: &[semantic_interpreter::provider::Message],
    ) -> std::result::Result<String, semantic_interpreter::provider::ProviderError> {
        Ok(self.0.clone())
    }
}
fn row_proposal() -> serde_json::Value {
    serde_json::json!({"version":1,"input":{"relation":"samples","instance":"s"},"requirements":[{"id":"filter","source_text":"greater than three","operation":{"kind":"filter","predicate":{"kind":"compare","field":{"instance":"s","field":"id"},"operator":"gt","value":{"type":"int64","value":3}}}},{"id":"id","source_text":"IDs","operation":{"kind":"project","field":{"instance":"s","field":"id"},"alias":"id"}},{"id":"sort","source_text":"ascending","operation":{"kind":"order","field":{"instance":"s","field":"id"},"direction":"asc","nulls":"last"}}],"unresolved":[]})
}
#[tokio::test]
async fn typed_row_and_graph_execute_bound_values() {
    for graph in [false, true] {
        let mut d = tiny();
        d.cases[0].question =
            "List sample IDs greater than three in ascending order, retaining duplicates.".into();
        if let Expected::Result { rows, .. } = d.expectations.get_mut("duplicates").unwrap() {
            rows.remove(0);
        }
        let proposal = if graph {
            serde_json::json!({"status":"graph","query":{"version":1,"nodes":[{"id":"rows","source_text":"sample IDs greater than three","operation":{"kind":"rows","query":row_proposal()}}],"root":"rows","ordering":[{"slot":"id","direction":"asc","nulls":"last"}],"limit":null,"unresolved":[]}})
        } else {
            serde_json::json!({"status":"query","query":row_proposal()})
        };
        let path = output();
        let report = run_with_provider(
            &d,
            RunOptions {
                interface: Interface::Ask,
                context: ContextMode::Full,
                artifacts: path.clone(),
                ..Default::default()
            },
            OwnedScript(proposal.to_string()),
        )
        .await
        .unwrap();
        assert!(report.success(), "graph={graph}: {report:?}");
        assert_eq!(
            report.cases[0].actual.as_ref().unwrap().rows,
            vec![vec![serde_json::json!("7")], vec![serde_json::json!("7")]]
        );
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[tokio::test]
async fn explicit_artifacts_inside_bundle_rejected_before_mutation() {
    let d = tiny();
    let nested = d.root.join("runtime-must-not-exist");
    assert!(
        run(
            &d,
            RunOptions {
                interface: Interface::Sql,
                artifacts: nested.clone(),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert!(!nested.exists());
    d.verify_digest().unwrap();
}

#[tokio::test]
async fn sql_selection_without_reference_cannot_pass_vacuously() {
    let mut d = tiny();
    d.cases[0].sql = None;
    let path = output();
    assert!(
        run(
            &d,
            RunOptions {
                interface: Interface::Sql,
                cases: vec!["duplicates".into()],
                artifacts: path.clone(),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert!(!path.exists());
    assert!(
        run(
            &d,
            RunOptions {
                interface: Interface::Sql,
                artifacts: path.clone(),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert!(!path.exists());
}

struct RateLimitedProvider;
impl semantic_interpreter::provider::ModelProvider for RateLimitedProvider {
    async fn complete(
        &self,
        _: &[semantic_interpreter::provider::Message],
    ) -> std::result::Result<String, semantic_interpreter::provider::ProviderError> {
        Err(semantic_interpreter::provider::ProviderError::Http(429))
    }
}
#[tokio::test]
async fn provider_http_status_is_retained_without_response_or_credentials() {
    let d = tiny();
    let path = output();
    let report = run_with_provider(
        &d,
        RunOptions {
            interface: Interface::Ask,
            artifacts: path.clone(),
            ..Default::default()
        },
        RateLimitedProvider,
    )
    .await
    .unwrap();
    assert!(!report.complete);
    assert!(!report.cases[0].passed);
    assert_eq!(report.cases[0].provider_errors.len(), 1);
    assert!(report.cases[0].provider_errors[0].contains("HTTP 429"));
    let persisted: RunReport =
        serde_json::from_slice(&std::fs::read(&report.report_path).unwrap()).unwrap();
    assert!(persisted.cases[0].provider_errors[0].contains("HTTP 429"));
    std::fs::remove_dir_all(path).unwrap();
}

struct TimestampedProvider(std::sync::Arc<std::sync::Mutex<Vec<std::time::Instant>>>);
impl semantic_interpreter::provider::ModelProvider for TimestampedProvider {
    async fn complete(
        &self,
        _: &[semantic_interpreter::provider::Message],
    ) -> std::result::Result<String, semantic_interpreter::provider::ProviderError> {
        self.0.lock().unwrap().push(std::time::Instant::now());
        Err(semantic_interpreter::provider::ProviderError::Http(429))
    }
}
#[tokio::test]
async fn model_call_pacing_is_shared_across_cases_and_preserves_each_failure() {
    let mut dataset = tiny();
    let mut second = dataset.cases[0].clone();
    second.id = "second_attempt".into();
    dataset.expectations.insert(
        second.id.clone(),
        dataset.expectations["duplicates"].clone(),
    );
    dataset.cases.push(second);
    dataset.manifest.required_paired_cases = 2;
    let starts = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let path = output();
    let report = run_with_provider(
        &dataset,
        RunOptions {
            interface: Interface::Ask,
            model_request_interval_millis: 40,
            artifacts: path.clone(),
            ..Default::default()
        },
        TimestampedProvider(starts.clone()),
    )
    .await
    .unwrap();
    let timestamps = starts.lock().unwrap();
    assert_eq!(
        timestamps.len(),
        2,
        "each failure remains one attempt; pacing must not retry transport errors"
    );
    // Allow one millisecond for the small gap from the gate to the provider timestamp.
    assert!(timestamps[1].duration_since(timestamps[0]) >= std::time::Duration::from_millis(39));
    assert!(!report.complete);
    assert_eq!(report.model_request_interval_millis, 40);
    assert_eq!(report.cases.len(), 2);
    assert!(
        report
            .cases
            .iter()
            .all(|case| !case.passed && case.incomplete && case.provider_errors.len() == 1)
    );
    assert!(report.cases[1].latency_millis >= 39);
    let mut serialized = serde_json::to_value(&report).unwrap();
    serialized
        .as_object_mut()
        .unwrap()
        .remove("model_request_interval_millis");
    let old_report: RunReport = serde_json::from_value(serialized).unwrap();
    assert_eq!(old_report.model_request_interval_millis, 0);
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test]
async fn excessive_model_pacing_is_rejected_before_setup() {
    let dataset = tiny();
    let path = output();
    assert!(
        run(
            &dataset,
            RunOptions {
                interface: Interface::Ask,
                model_request_interval_millis: 60_001,
                artifacts: path.clone(),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert!(!path.exists());
    assert_eq!(RunOptions::default().model_request_interval_millis, 0);
}

#[tokio::test]
async fn pacing_wait_respects_case_deadline_without_starting_or_retrying_call() {
    let mut dataset = tiny();
    let mut second = dataset.cases[0].clone();
    second.id = "second_attempt".into();
    dataset.expectations.insert(
        second.id.clone(),
        dataset.expectations["duplicates"].clone(),
    );
    dataset.cases.push(second);
    dataset.manifest.required_paired_cases = 2;
    let starts = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let path = output();
    let report = run_with_provider(
        &dataset,
        RunOptions {
            interface: Interface::Ask,
            timeout_seconds: 1,
            model_request_interval_millis: 2000,
            artifacts: path.clone(),
            ..Default::default()
        },
        TimestampedProvider(starts.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        starts.lock().unwrap().len(),
        1,
        "the second provider call must not start after its pacing wait exceeds the deadline"
    );
    assert!(!report.complete);
    assert!(!report.success());
    assert_eq!(report.cases.len(), 2);
    assert!(report.cases[0].provider_errors[0].contains("HTTP 429"));
    assert!(report.cases[1].provider_errors.is_empty());
    assert!(report.cases[1].incomplete);
    assert!(!report.cases[1].passed);
    assert!(
        report.cases[1]
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("deadline")
    );
    std::fs::remove_dir_all(path).unwrap();
}

struct BlockingSecond {
    calls: std::sync::atomic::AtomicUsize,
    entered: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
    response: String,
}
impl semantic_interpreter::provider::ModelProvider for BlockingSecond {
    async fn complete(
        &self,
        _: &[semantic_interpreter::provider::Message],
    ) -> std::result::Result<String, semantic_interpreter::provider::ProviderError> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(self.response.clone())
    }
}
#[tokio::test]
async fn persisted_running_report_cannot_pass_before_every_attempt_finishes() {
    let mut dataset = tiny();
    dataset.cases[0].question =
        "List sample IDs greater than three in ascending order, retaining duplicates.".into();
    if let Expected::Result { rows, .. } = dataset.expectations.get_mut("duplicates").unwrap() {
        rows.remove(0);
    }
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let provider = BlockingSecond {
        calls: std::sync::atomic::AtomicUsize::new(0),
        entered: entered.clone(),
        release: release.clone(),
        response: serde_json::json!({"status":"query","query":row_proposal()}).to_string(),
    };
    let path = output();
    let artifacts = path.clone();
    let task = tokio::spawn(async move {
        run_with_provider(
            &dataset,
            RunOptions {
                interface: Interface::Ask,
                context: ContextMode::Full,
                repetitions: 2,
                artifacts,
                ..Default::default()
            },
            provider,
        )
        .await
        .unwrap()
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
        .await
        .unwrap();
    let dir = std::fs::read_dir(&path)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let snapshot: RunReport =
        serde_json::from_slice(&std::fs::read(dir.join("report.json")).unwrap()).unwrap();
    assert_eq!(snapshot.expected_case_attempts, 2);
    assert_eq!(snapshot.completed_case_attempts, 1);
    assert!(snapshot.cases[0].passed);
    assert!(!snapshot.finalized && !snapshot.complete && !snapshot.success());
    release.notify_one();
    let mut report = task.await.unwrap();
    assert!(
        report.finalized && report.complete && report.success(),
        "{report:?}"
    );
    assert_eq!(report.completed_case_attempts, 2);
    report.cases[1].repetition = report.cases[0].repetition;
    assert!(
        !report.success(),
        "duplicate identities cannot satisfy coverage"
    );
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test]
async fn output_budget_is_independent_of_execution_admission_and_old_reports_are_nonterminal() {
    let path = output();
    let report = run(
        &tiny(),
        RunOptions {
            interface: Interface::Sql,
            artifacts: path.clone(),
            max_bytes: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        report.execution_budgets.max_decoded_bytes,
        semantic_engine::QueryOptions::default().max_decoded_bytes
    );
    assert_eq!(report.output_max_bytes, 1);
    assert!(report.finalized && !report.complete && !report.success());
    let mut legacy = serde_json::to_value(&report).unwrap();
    legacy.as_object_mut().unwrap().remove("finalized");
    assert!(
        !serde_json::from_value::<RunReport>(legacy)
            .unwrap()
            .success()
    );
    std::fs::remove_dir_all(path).unwrap();
    assert!(
        run(
            &tiny(),
            RunOptions {
                max_decoded_bytes: Some(0),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn artifact_execution_capacity_and_explicit_overrides_are_recorded() {
    let mut dataset = tiny();
    dataset.manifest.execution = Some(ExecutionLimits {
        max_requests: Some(1024),
        max_decoded_bytes: Some(128 * 1024 * 1024),
        max_remote_bytes: None,
    });
    for override_requests in [None, Some(512)] {
        let path = output();
        let report = run(
            &dataset,
            RunOptions {
                interface: Interface::Sql,
                artifacts: path.clone(),
                max_requests: override_requests,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(report.success(), "{report:?}");
        assert_eq!(
            report.execution_budgets.max_remote_requests,
            override_requests.unwrap_or(1024)
        );
        assert_eq!(
            report.execution_budgets.max_decoded_bytes,
            128 * 1024 * 1024
        );
        assert_eq!(
            report.execution_budgets.max_remote_bytes,
            semantic_engine::QueryOptions::default().max_remote_bytes
        );
        assert_eq!(report.output_max_bytes, 32 * 1024 * 1024);
        std::fs::remove_dir_all(path).unwrap();
    }
    let path = output();
    let report = run(
        &dataset,
        RunOptions {
            interface: Interface::Sql,
            artifacts: path.clone(),
            public_interfaces: true,
            cli_binary: Some("binary-must-not-be-launched".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(!report.success());
    assert_eq!(report.public_checks.len(), 3);
    assert!(report.public_checks.iter().all(|check| {
        !check.passed
            && check
                .diagnostic
                .as_deref()
                .unwrap()
                .contains("budget forwarding")
    }));
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn malformed_execution_capacity_fails_artifact_loading_offline() {
    let dataset = tiny();
    let path = output();
    std::fs::create_dir_all(path.join("expected")).unwrap();
    for name in [
        "samples.csv",
        "cases.json",
        "semantic-db.yaml",
        "expected/duplicates.json",
    ] {
        std::fs::copy(dataset.root.join(name), path.join(name)).unwrap();
    }
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dataset.root.join("manifest.json")).unwrap())
            .unwrap();
    for limits in [
        serde_json::json!({"max_requests":0}),
        serde_json::json!({"max_requests":1_000_001}),
        serde_json::json!({"max_decoded_bytes":0}),
        serde_json::json!({"max_remote_bytes":1u64<<41}),
        serde_json::json!({"unknown":1}),
    ] {
        let mut manifest = original.clone();
        manifest["execution"] = limits;
        std::fs::write(
            path.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(Dataset::load(path.join("manifest.json")).is_err());
    }
    let mut manifest = original;
    manifest["execution"] = serde_json::json!({"max_requests":1024});
    std::fs::write(
        path.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert_eq!(
        Dataset::load(path.join("manifest.json"))
            .unwrap()
            .manifest
            .execution
            .unwrap()
            .max_requests,
        Some(1024)
    );
    std::fs::remove_dir_all(path).unwrap();
}
