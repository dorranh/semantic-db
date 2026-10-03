//! Explicit sensitive model transcripts. These are never included in normal
//! compilation records. Keep a recorder per compilation; callers own retention.
use super::{CompileDiagnostic, PIPELINE_REVISION, bounded_json, diagnostic};
use crate::provider::{
    Message, ModelCompletion, ModelProvider, ProviderCapabilities, ProviderError,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct CaptureLimits {
    pub max_bytes: usize,
    pub max_calls: usize,
}
impl Default for CaptureLimits {
    fn default() -> Self {
        Self {
            max_bytes: 1024 * 1024,
            max_calls: 8,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturedCall {
    messages: Vec<Message>,
    result: Option<CapturedResult>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CapturedResult {
    Completion { response: ModelCompletion },
    Failure { code: FailureCode },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FailureCode {
    Configuration,
    Timeout,
    Transport,
    Http,
    Response,
    Refused,
    Incomplete,
}
impl FailureCode {
    fn from_error(error: &ProviderError) -> Self {
        match error {
            ProviderError::Configuration(_) => Self::Configuration,
            ProviderError::Timeout => Self::Timeout,
            ProviderError::Transport => Self::Transport,
            ProviderError::Http(_) => Self::Http,
            ProviderError::Response(_) => Self::Response,
            ProviderError::Refused => Self::Refused,
            ProviderError::Incomplete => Self::Incomplete,
        }
    }
    fn error(self) -> ProviderError {
        match self {
            Self::Configuration => ProviderError::Configuration("recorded configuration failure"),
            Self::Timeout => ProviderError::Timeout,
            Self::Transport => ProviderError::Transport,
            Self::Http => ProviderError::Http(502),
            Self::Response => ProviderError::Response("recorded response failure"),
            Self::Refused => ProviderError::Refused,
            Self::Incomplete => ProviderError::Incomplete,
        }
    }
}
/// Serialize only under an explicit access/retention policy: this contains raw
/// requests, catalog context, model responses and literal values. Debug redacts it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelTranscript {
    pub version: u32,
    pub pipeline_revision: String,
    pub calls_started: usize,
    pub omitted_calls: usize,
    pub truncated: bool,
    calls: Vec<CapturedCall>,
}
impl std::fmt::Debug for ModelTranscript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelTranscript")
            .field("version", &self.version)
            .field("calls_started", &self.calls_started)
            .field("omitted_calls", &self.omitted_calls)
            .field("complete", &self.is_complete())
            .finish()
    }
}
impl ModelTranscript {
    pub fn is_complete(&self) -> bool {
        !self.truncated
            && self.omitted_calls == 0
            && self.calls.len() == self.calls_started
            && self.calls.iter().all(|call| call.result.is_some())
    }
    pub fn retained_calls(&self) -> usize {
        self.calls.len()
    }
}
struct CaptureState {
    transcript: ModelTranscript,
    bytes: usize,
}
pub struct CaptureRecorder {
    limits: CaptureLimits,
    state: Mutex<CaptureState>,
}
impl std::fmt::Debug for CaptureRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureRecorder")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
impl CaptureRecorder {
    fn new(limits: CaptureLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(CaptureState {
                bytes: 512,
                transcript: ModelTranscript {
                    version: 1,
                    pipeline_revision: PIPELINE_REVISION.into(),
                    calls_started: 0,
                    omitted_calls: 0,
                    truncated: false,
                    calls: vec![],
                },
            }),
        }
    }
    /// A cancellation leaves an incomplete call. Budget omissions are explicit;
    /// neither condition changes the provider response or compilation outcome.
    pub fn snapshot(&self) -> ModelTranscript {
        let mut transcript = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .transcript
            .clone();
        if bounded_json(&transcript, self.limits.max_bytes).is_err() {
            transcript.truncated = true;
            transcript.omitted_calls = transcript.calls_started;
            transcript.calls.clear();
        }
        transcript
    }
    fn start(&self, messages: &[Message]) -> Option<usize> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.transcript.calls_started += 1;
        let bytes = bounded_json(&messages, self.limits.max_bytes.saturating_sub(state.bytes))
            .ok()
            .map(|s| s.len());
        if state.transcript.calls.len() >= self.limits.max_calls || bytes.is_none() {
            state.transcript.truncated = true;
            state.transcript.omitted_calls += 1;
            return None;
        }
        state.bytes = state
            .bytes
            .saturating_add(bytes.expect("bounded messages"))
            .saturating_add(128);
        let slot = state.transcript.calls.len();
        state.transcript.calls.push(CapturedCall {
            messages: messages.to_vec(),
            result: None,
        });
        Some(slot)
    }
    fn complete(&self, slot: Option<usize>, response: &Result<ModelCompletion, ProviderError>) {
        let Some(slot) = slot else {
            return;
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Check a borrowed envelope before cloning potentially large output.
        #[derive(Serialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum BorrowedResult<'a> {
            Completion { response: &'a ModelCompletion },
            Failure { code: FailureCode },
        }
        let borrowed = match response {
            Ok(response) => BorrowedResult::Completion { response },
            Err(error) => BorrowedResult::Failure {
                code: FailureCode::from_error(error),
            },
        };
        if let Ok(serialized) =
            bounded_json(&borrowed, self.limits.max_bytes.saturating_sub(state.bytes))
        {
            state.bytes += serialized.len();
            state.transcript.calls[slot].result = Some(match response {
                Ok(response) => CapturedResult::Completion {
                    response: response.clone(),
                },
                Err(error) => CapturedResult::Failure {
                    code: FailureCode::from_error(error),
                },
            });
        } else {
            state.transcript.truncated = true;
            state.transcript.omitted_calls += 1;
        }
    }
}
/// Wrap a provider explicitly to capture one compilation's calls. The normal
/// provider path and its actual usage/status envelopes remain unchanged.
pub struct RecordingProvider<P> {
    inner: P,
    recorder: Arc<CaptureRecorder>,
}
impl<P> RecordingProvider<P> {
    pub fn new(inner: P, limits: CaptureLimits) -> Self {
        Self {
            inner,
            recorder: Arc::new(CaptureRecorder::new(limits)),
        }
    }
    pub fn recorder(&self) -> Arc<CaptureRecorder> {
        self.recorder.clone()
    }
}
impl<P: ModelProvider> ModelProvider for RecordingProvider<P> {
    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.complete_envelope(messages).await?.into_text()
    }
    async fn complete_envelope(
        &self,
        messages: &[Message],
    ) -> Result<ModelCompletion, ProviderError> {
        let slot = self.recorder.start(messages);
        let response = self.inner.complete_envelope(messages).await;
        self.recorder.complete(slot, &response);
        response
    }
}
struct ReplayState {
    cursor: usize,
    mismatched: bool,
}
/// Offline replay only: compares every exact protocol/context/repair message
/// before returning the recorded completion. It never contacts a provider.
pub struct TranscriptProvider {
    transcript: ModelTranscript,
    state: Mutex<ReplayState>,
}
impl TranscriptProvider {
    pub fn new(transcript: ModelTranscript, max_bytes: usize) -> Result<Self, CompileDiagnostic> {
        bounded_json(&transcript, max_bytes).map_err(|_| {
            diagnostic("capture_limit", "Transcript exceeds the replay byte budget")
        })?;
        if transcript.version != 1 || transcript.pipeline_revision != PIPELINE_REVISION {
            return Err(diagnostic(
                "replay_version",
                "Transcript requires the recorded pipeline revision",
            ));
        }
        if !transcript.is_complete() {
            return Err(diagnostic(
                "replay_incomplete",
                "Transcript contains omitted or interrupted calls",
            ));
        }
        Ok(Self {
            transcript,
            state: Mutex::new(ReplayState {
                cursor: 0,
                mismatched: false,
            }),
        })
    }
    pub fn verify_consumed(&self) -> Result<(), CompileDiagnostic> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.mismatched || state.cursor != self.transcript.calls.len() {
            Err(diagnostic(
                "replay_mismatch",
                "Compiler messages or call count differed from the transcript",
            ))
        } else {
            Ok(())
        }
    }
}
impl ModelProvider for TranscriptProvider {
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.complete_envelope(messages).await?.into_text()
    }
    async fn complete_envelope(
        &self,
        messages: &[Message],
    ) -> Result<ModelCompletion, ProviderError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(call) = self.transcript.calls.get(state.cursor) else {
            state.mismatched = true;
            return Err(ProviderError::Response("transcript exhausted"));
        };
        if call.messages != messages {
            state.mismatched = true;
            return Err(ProviderError::Response("transcript input mismatch"));
        }
        state.cursor += 1;
        match call.result.as_ref().expect("complete transcript") {
            CapturedResult::Completion { response } => Ok(response.clone()),
            CapturedResult::Failure { code } => Err(code.error()),
        }
    }
}
impl ModelProvider for Arc<TranscriptProvider> {
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.as_ref().complete(messages).await
    }
    async fn complete_envelope(
        &self,
        messages: &[Message],
    ) -> Result<ModelCompletion, ProviderError> {
        self.as_ref().complete_envelope(messages).await
    }
}
