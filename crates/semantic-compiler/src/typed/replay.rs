//! Explicit bounded structured replay. Captures contain literals and must be
//! retained under the caller's access/retention policy, separately from telemetry.
use super::{CompileDiagnostic, CompileOptions, CompiledQuery, TypedCompilation, diagnostic};
use semantic_engine::{Engine, MVP_EXECUTION_PROFILE_REVISION};
use semantic_plan::typed::SemanticQuery;
use serde::{Deserialize, Serialize};
mod decision;
pub use decision::{DecisionTrace, RuleDecision, StageArtifact, StageReplayOutcome};

pub const PIPELINE_REVISION: &str = "semantic-compiler/typed-v1/pipeline-36/datafusion-55";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayBundle {
    pub version: u32,
    pub pipeline_revision: String,
    pub execution_profile_revision: String,
    pub snapshot_id: String,
    pub artifact_digest: String,
    pub proposal: SemanticQuery,
    pub request_context: Option<super::RequestContext>,
    pub request_evidence: Option<semantic_plan::typed::RequestEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_trace: Option<DecisionTrace>,
}
impl std::fmt::Debug for ReplayBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayBundle")
            .field("version", &self.version)
            .field("pipeline_revision", &self.pipeline_revision)
            .field("has_stage_trace", &self.stage_trace.is_some())
            .finish_non_exhaustive()
    }
}
impl CompiledQuery {
    /// This is deterministic proposal replay, not model or historical-row replay.
    pub fn capture_replay(&self, max_bytes: usize) -> Result<ReplayBundle, CompileDiagnostic> {
        let bundle = ReplayBundle {
            version: 1,
            pipeline_revision: PIPELINE_REVISION.into(),
            execution_profile_revision: self.execution_profile_revision.into(),
            snapshot_id: self.bound.snapshot_id().into(),
            artifact_digest: super::artifact_digest(self),
            proposal: self.intent.clone(),
            request_context: self.request_context.clone(),
            request_evidence: self.request_evidence.clone(),
            stage_trace: None,
        };
        super::bounded_json(&bundle, max_bytes).map_err(|_| {
            diagnostic(
                "capture_limit",
                "Requested replay capture exceeds its byte budget",
            )
        })?;
        Ok(bundle)
    }
    /// Opt-in sensitive stage capture. The caller chooses whether before/after
    /// IR is retained; stage and decision digests are always retained.
    pub fn capture_stage_replay(
        &self,
        max_bytes: usize,
        include_ir: bool,
    ) -> Result<ReplayBundle, CompileDiagnostic> {
        let mut bundle = self.capture_replay(max_bytes)?;
        bundle.version = 2;
        bundle.stage_trace = Some(DecisionTrace::from_query(self, include_ir));
        super::bounded_json(&bundle, max_bytes).map_err(|_| {
            diagnostic(
                "capture_limit",
                "Stage replay capture exceeds its byte budget",
            )
        })?;
        Ok(bundle)
    }
}
impl ReplayBundle {
    /// Deserialized proposals always re-enter binding. Digests and versions are
    /// assertions to verify, never authority to skip validation.
    pub async fn replay(
        &self,
        engine: &Engine,
        mut options: CompileOptions,
    ) -> Result<TypedCompilation, CompileDiagnostic> {
        if !(1..=2).contains(&self.version) || self.pipeline_revision != PIPELINE_REVISION {
            return Err(diagnostic(
                "replay_version",
                "Replay requires the recorded compiler pipeline revision",
            ));
        }
        if self.execution_profile_revision != MVP_EXECUTION_PROFILE_REVISION {
            return Err(diagnostic(
                "replay_profile",
                "Replay requires the recorded execution profile revision",
            ));
        }
        if self.snapshot_id != engine.catalog().snapshot().id() {
            return Err(diagnostic(
                "snapshot_mismatch",
                "Replay requires the recorded catalog snapshot",
            ));
        }
        if options
            .request_context
            .as_ref()
            .is_some_and(|context| Some(context) != self.request_context.as_ref())
        {
            return Err(diagnostic(
                "replay_context",
                "Replay cannot change the recorded request context",
            ));
        }
        if options
            .request_evidence
            .as_ref()
            .is_some_and(|evidence| Some(evidence) != self.request_evidence.as_ref())
        {
            return Err(diagnostic(
                "replay_context",
                "Replay cannot change the recorded request evidence",
            ));
        }
        options.request_evidence = self.request_evidence.clone();
        options.request_context = self.request_context.clone();
        super::preflight(&self.proposal, &options.clone().start())?;
        let compilation = super::compile_semantic(engine, self.proposal.clone(), options).await;
        if let Some(digest) = &compilation.record.artifact_digest
            && digest != &self.artifact_digest
        {
            return Err(diagnostic(
                "replay_mismatch",
                "Revalidated artifact differs from the recorded artifact",
            ));
        }
        Ok(compilation)
    }

    /// Recompile under the current engine and scope, reporting the first changed
    /// stage or decision set. A capture without stage data cannot claim parity.
    pub async fn replay_stages(
        &self,
        engine: &Engine,
        mut options: CompileOptions,
    ) -> Result<StageReplayOutcome, CompileDiagnostic> {
        if self.version != 2 || self.pipeline_revision != PIPELINE_REVISION {
            return Err(diagnostic(
                "replay_version",
                "Stage replay requires its recorded pipeline revision",
            ));
        }
        let expected = self.stage_trace.as_ref().ok_or_else(|| {
            diagnostic(
                "replay_stage_missing",
                "The retained capture has no stage trace",
            )
        })?;
        if expected.version != 1 || expected.stages.is_empty() {
            return Err(diagnostic(
                "replay_version",
                "Stage trace version is unsupported",
            ));
        }
        if self.execution_profile_revision != MVP_EXECUTION_PROFILE_REVISION {
            return Err(diagnostic(
                "replay_profile",
                "Stage replay requires its recorded execution profile",
            ));
        }
        if self.snapshot_id != engine.catalog().snapshot().id() {
            return Err(diagnostic(
                "snapshot_mismatch",
                "Stage replay requires its recorded catalog snapshot",
            ));
        }
        if options
            .request_context
            .as_ref()
            .is_some_and(|value| Some(value) != self.request_context.as_ref())
            || options
                .request_evidence
                .as_ref()
                .is_some_and(|value| Some(value) != self.request_evidence.as_ref())
        {
            return Err(diagnostic(
                "replay_context",
                "Stage replay cannot change recorded request context or evidence",
            ));
        }
        options.request_context = self.request_context.clone();
        options.request_evidence = self.request_evidence.clone();
        super::preflight(&self.proposal, &options.clone().start())?;
        let compilation = super::compile_semantic(engine, self.proposal.clone(), options).await;
        let super::TypedOutcome::Compiled { query } = compilation.outcome else {
            return Err(diagnostic(
                "replay_unavailable",
                "Stage replay recompilation did not produce an accepted artifact",
            ));
        };
        let actual = DecisionTrace::from_query(&query, false);
        let comparison = expected.compare(&actual);
        if matches!(comparison, StageReplayOutcome::Matched { .. })
            && !expected.retained_ir_is_consistent()
        {
            return Err(diagnostic(
                "replay_capture_invalid",
                "Retained stage IR differs from its recorded digest",
            ));
        }
        Ok(comparison)
    }
}
