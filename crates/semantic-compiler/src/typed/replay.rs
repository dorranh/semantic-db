//! Explicit bounded structured replay. Captures contain literals and must be
//! retained under the caller's access/retention policy, separately from telemetry.
use super::{CompileDiagnostic, CompileOptions, CompiledQuery, TypedCompilation, diagnostic};
use semantic_engine::Engine;
use semantic_plan::typed::SemanticQuery;
use serde::{Deserialize, Serialize};

pub const PIPELINE_REVISION: &str = "semantic-compiler/typed-v1/pipeline-5/datafusion-55";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayBundle {
    pub version: u32,
    pub pipeline_revision: String,
    pub snapshot_id: String,
    pub artifact_digest: String,
    pub proposal: SemanticQuery,
    pub request_context: Option<super::RequestContext>,
    pub request_evidence: Option<semantic_plan::typed::RequestEvidence>,
}
impl CompiledQuery {
    /// This is deterministic proposal replay, not model or historical-row replay.
    pub fn capture_replay(&self, max_bytes: usize) -> Result<ReplayBundle, CompileDiagnostic> {
        let bundle = ReplayBundle {
            version: 1,
            pipeline_revision: PIPELINE_REVISION.into(),
            snapshot_id: self.bound.snapshot_id().into(),
            artifact_digest: super::artifact_digest(self),
            proposal: self.intent.clone(),
            request_context: self.request_context.clone(),
            request_evidence: self.request_evidence.clone(),
        };
        super::bounded_json(&bundle, max_bytes).map_err(|_| {
            diagnostic(
                "capture_limit",
                "Requested replay capture exceeds its byte budget",
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
        if self.version != 1 || self.pipeline_revision != PIPELINE_REVISION {
            return Err(diagnostic(
                "replay_version",
                "Replay requires the recorded compiler pipeline revision",
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
}
