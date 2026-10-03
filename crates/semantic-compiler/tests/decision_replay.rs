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
use semantic_compiler::typed::{CompileOptions, StageReplayOutcome, TypedOutcome, compile_rows};
use semantic_engine::Engine;
use semantic_plan::typed::*;

#[test]
fn replay_divergence_debug_omits_untrusted_stage_and_digest_text() {
    let outcome = StageReplayOutcome::Diverged {
        stage: "private-stage-name".into(),
        expected_digest: "private-expected".into(),
        actual_digest: "private-actual".into(),
    };
    let rendered = format!("{outcome:?}");
    assert!(rendered.contains("Diverged"));
    for sensitive in ["private-stage-name", "private-expected", "private-actual"] {
        assert!(!rendered.contains(sensitive));
    }
}

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
fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "items".into(),
            instance: "r".into(),
        },
        requirements: vec![Requirement {
            id: "ids".into(),
            source_text: "private-ID-phrase".into(),
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
#[tokio::test]
async fn stage_replay_matches_and_reports_first_divergence() {
    let engine = fixture();
    let result = compile_rows(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: artifact } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let capture = artifact.capture_stage_replay(128 * 1024, true).unwrap();
    assert_eq!(capture.version, 2);
    assert_eq!(capture.stage_trace.as_ref().unwrap().stages.len(), 4);
    assert!(
        capture.stage_trace.as_ref().unwrap().stages[1]
            .before_ir
            .is_some()
    );
    let decision = &capture.stage_trace.as_ref().unwrap().decisions[0];
    assert_eq!(decision.requirement_id, "ids");
    assert!(!decision.rule.is_empty());
    assert_eq!(decision.preconditions.len(), 2);
    assert!(!format!("{capture:?}").contains("private-ID-phrase"));
    let mut hostile = capture.clone();
    let trace = hostile.stage_trace.as_mut().unwrap();
    trace.stages[0].stage = "private-stage-name".into();
    trace.stages[0].after_digest = "private-digest".into();
    trace.decisions[0].rule = "private-rule".into();
    let rendered = format!("{:?} {:?}", trace.stages[0], trace.decisions[0]);
    for sensitive in ["private-stage-name", "private-digest", "private-rule"] {
        assert!(!rendered.contains(sensitive));
    }
    assert!(matches!(
        capture
            .replay_stages(&engine, CompileOptions::default())
            .await
            .unwrap(),
        StageReplayOutcome::Matched {
            stages_checked: 4,
            decisions_checked: 1
        }
    ));
    let mut changed = capture.clone();
    changed.proposal.requirements[0].source_text = "changed".into();
    assert!(
        matches!(changed.replay_stages(&engine, CompileOptions::default()).await.unwrap(), StageReplayOutcome::Diverged { stage, .. } if stage == "proposal")
    );
    let mut changed = capture;
    changed.stage_trace.as_mut().unwrap().stages[2].after_digest = "changed".into();
    assert!(
        matches!(changed.replay_stages(&engine, CompileOptions::default()).await.unwrap(), StageReplayOutcome::Diverged { stage, .. } if stage == "relational")
    );
}

#[tokio::test]
async fn retained_stage_inputs_and_ir_must_match_the_recompiled_pipeline() {
    let engine = fixture();
    let result = compile_rows(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: artifact } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    let capture = artifact.capture_stage_replay(128 * 1024, true).unwrap();

    let mut changed = capture.clone();
    changed.stage_trace.as_mut().unwrap().stages[2].before_digest = Some("changed".into());
    assert!(matches!(
        changed
            .replay_stages(&engine, CompileOptions::default())
            .await
            .unwrap(),
        StageReplayOutcome::Diverged { stage, .. } if stage == "relational"
    ));

    let mut changed = capture;
    changed.stage_trace.as_mut().unwrap().stages[1].after_ir =
        Some(serde_json::json!({"tampered": true}));
    assert_eq!(
        changed
            .replay_stages(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_capture_invalid"
    );
}
#[tokio::test]
async fn stage_capture_is_bounded_and_plain_replay_has_explicit_limit() {
    let engine = fixture();
    let result = compile_rows(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query: artifact } = result.outcome else {
        panic!("{:?}", result.outcome)
    };
    assert_eq!(
        artifact.capture_stage_replay(1, true).unwrap_err().code,
        "capture_limit"
    );
    let plain = artifact.capture_replay(128 * 1024).unwrap();
    assert_eq!(
        plain
            .replay_stages(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_version"
    );
    let mut capture = artifact.capture_stage_replay(128 * 1024, false).unwrap();
    assert!(
        capture
            .stage_trace
            .as_ref()
            .unwrap()
            .stages
            .iter()
            .all(|stage| stage.after_ir.is_none())
    );
    capture.pipeline_revision = "retired".into();
    assert_eq!(
        capture
            .replay_stages(&engine, CompileOptions::default())
            .await
            .unwrap_err()
            .code,
        "replay_version"
    );
}
