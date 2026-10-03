use super::*;
use serde::Deserialize;

/// Private capture data, never part of normal telemetry. Replay pins definitions
/// and compiler identity, but does not capture or promise historical source rows.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphReplayBundle {
    pub version: u32,
    pub pipeline_revision: String,
    pub execution_profile_revision: String,
    pub snapshot_id: String,
    pub artifact_digest: String,
    pub proposal: GraphQuery,
    pub request_context: Option<RequestContext>,
    #[serde(default)]
    pub request_evidence: Option<GraphRequestEvidence>,
}
impl std::fmt::Debug for GraphReplayBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GraphReplayBundle")
            .field("version", &self.version)
            .field("has_request_evidence", &self.request_evidence.is_some())
            .finish_non_exhaustive()
    }
}
impl CompiledGraph {
    pub fn capture_replay(&self, max_bytes: usize) -> Result<GraphReplayBundle, CompileDiagnostic> {
        let bundle = GraphReplayBundle {
            version: 1,
            pipeline_revision: PIPELINE_REVISION.into(),
            execution_profile_revision: self.execution_profile_revision.into(),
            snapshot_id: self.snapshot_id.clone(),
            artifact_digest: semantic_catalog::canonical_digest(
                &serde_json::json!({"pipeline":PIPELINE_REVISION,"artifact":self}),
            ),
            proposal: self.proposal.clone(),
            request_context: self.request_context.clone(),
            request_evidence: self.request_evidence.clone(),
        };
        bounded_json(&bundle, max_bytes).map_err(|_| {
            diagnostic(
                "capture_limit",
                "Graph replay capture exceeds its byte budget",
            )
        })?;
        Ok(bundle)
    }
}
impl GraphReplayBundle {
    pub async fn replay(
        &self,
        engine: &Engine,
        mut options: CompileOptions,
    ) -> Result<TypedCompilation, CompileDiagnostic> {
        if self.version != 1 || self.pipeline_revision != PIPELINE_REVISION {
            return Err(diagnostic(
                "replay_version",
                "Graph replay requires the recorded pipeline revision",
            ));
        }
        if self.execution_profile_revision != MVP_EXECUTION_PROFILE_REVISION {
            return Err(diagnostic(
                "replay_profile",
                "Graph replay requires the recorded execution profile revision",
            ));
        }
        if self.snapshot_id != engine.catalog().snapshot().id() {
            return Err(diagnostic(
                "snapshot_mismatch",
                "Graph replay requires its recorded catalog snapshot",
            ));
        }
        if options
            .request_context
            .as_ref()
            .is_some_and(|context| Some(context) != self.request_context.as_ref())
        {
            return Err(diagnostic(
                "replay_context",
                "Graph replay cannot change the recorded request context",
            ));
        }
        options.request_context = self.request_context.clone();
        if options
            .graph_request_evidence
            .as_ref()
            .is_some_and(|evidence| Some(evidence) != self.request_evidence.as_ref())
        {
            return Err(diagnostic(
                "replay_evidence",
                "Graph replay cannot change the recorded request evidence",
            ));
        }
        options.graph_request_evidence = self.request_evidence.clone();
        preflight_graph(&self.proposal, &options.clone().start())?;
        let result = compile_graph(engine, self.proposal.clone(), options).await;
        if result
            .record
            .artifact_digest
            .as_ref()
            .is_some_and(|digest| digest != &self.artifact_digest)
        {
            return Err(diagnostic(
                "replay_mismatch",
                "Graph replay differs from the captured artifact",
            ));
        }
        Ok(result)
    }
}
