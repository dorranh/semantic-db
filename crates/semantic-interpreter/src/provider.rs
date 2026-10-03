use std::{future::Future, time::Duration};

use reqwest::{Client, Url, header::HeaderValue};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// Providers return text; the compiler owns decoding and semantic validation.
pub trait ModelProvider: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }
    fn complete(
        &self,
        messages: &[Message],
    ) -> impl Future<Output = Result<String, ProviderError>> + Send;

    /// Legacy providers remain usable. Missing accounting stays explicitly unknown.
    fn complete_envelope(
        &self,
        messages: &[Message],
    ) -> impl Future<Output = Result<ModelCompletion, ProviderError>> + Send {
        async {
            Ok(ModelCompletion {
                text: Some(self.complete(messages).await?),
                status: CompletionStatus::Complete,
                metadata: CompletionMetadata::default(),
            })
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProviderCapabilities {
    pub json_object_output: bool,
    pub schema_constrained_output: bool,
    pub tool_calls: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionStatus {
    Complete,
    Refused,
    Incomplete,
    ToolCalls,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionMetadata {
    pub provider_request_id: Option<String>,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub usage: TokenUsage,
}
/// Text is intentionally absent from Debug; explicit replay capture owns raw payloads.
#[derive(Clone, Serialize, Deserialize)]
pub struct ModelCompletion {
    pub text: Option<String>,
    pub status: CompletionStatus,
    pub metadata: CompletionMetadata,
}
impl std::fmt::Debug for ModelCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelCompletion")
            .field("status", &self.status)
            .field("metadata", &self.metadata)
            .field("text_bytes", &self.text.as_ref().map(String::len))
            .finish()
    }
}
impl ModelCompletion {
    pub fn into_text(self) -> Result<String, ProviderError> {
        match self.status {
            CompletionStatus::Refused => Err(ProviderError::Refused),
            CompletionStatus::Incomplete | CompletionStatus::ToolCalls => {
                Err(ProviderError::Incomplete)
            }
            CompletionStatus::Complete => self
                .text
                .filter(|text| !text.trim().is_empty())
                .ok_or(ProviderError::Response("missing or empty text content")),
        }
    }
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("invalid provider configuration: {0}")]
    Configuration(&'static str),
    #[error("model request timed out")]
    Timeout,
    #[error("model request failed; check provider connectivity and TLS configuration")]
    Transport,
    #[error("model provider returned HTTP {0}; check credentials, model, quota, and base URL")]
    Http(u16),
    #[error("invalid model response: {0}")]
    Response(&'static str),
    #[error("model refused the request")]
    Refused,
    #[error("model response did not finish normally (possibly truncated or filtered)")]
    Incomplete,
}

/// No Debug implementation: configuration contains a credential.
pub struct OpenAiConfig {
    pub api_key: String,
    /// API root, including any version prefix, e.g. https://api.openai.com/v1.
    pub base_url: String,
    pub model: String,
    pub timeout: Duration,
    /// Disable for compatible servers without response_format support.
    pub json_mode: bool,
}

impl OpenAiConfig {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            base_url: "https://api.openai.com/v1".into(),
            timeout: Duration::from_secs(60),
            json_mode: true,
        }
    }
}

pub struct OpenAiProvider {
    client: Client,
    endpoint: Url,
    authorization: HeaderValue,
    model: String,
    json_mode: bool,
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig) -> Result<Self, ProviderError> {
        if config.api_key.trim().is_empty() || config.model.trim().is_empty() {
            return Err(ProviderError::Configuration(
                "API key and model must be nonempty",
            ));
        }
        if config.timeout.is_zero() {
            return Err(ProviderError::Configuration("timeout must be positive"));
        }
        let mut endpoint = Url::parse(&config.base_url).map_err(|_| {
            ProviderError::Configuration("base URL must be an absolute HTTP(S) URL")
        })?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(ProviderError::Configuration(
                "base URL must be HTTP(S), without credentials, query, or fragment",
            ));
        }
        endpoint.set_path(&format!(
            "{}/chat/completions",
            endpoint.path().trim_end_matches('/')
        ));
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", config.api_key))
            .map_err(|_| ProviderError::Configuration("API key is not a valid header value"))?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.timeout.min(Duration::from_secs(10)))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(transport_error)?;
        Ok(Self {
            client,
            endpoint,
            authorization,
            model: config.model,
            json_mode: config.json_mode,
        })
    }
}

#[derive(Serialize)]
struct CompletionRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct CompletionResponse {
    choices: Vec<Choice>,
    id: Option<String>,
    model: Option<String>,
    system_fingerprint: Option<String>,
    usage: Option<UsageResponse>,
}
#[derive(Deserialize, Default)]
struct UsageResponse {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    prompt_tokens_details: Option<PromptUsage>,
    completion_tokens_details: Option<OutputUsage>,
}
#[derive(Deserialize)]
struct PromptUsage {
    cached_tokens: Option<u64>,
}
#[derive(Deserialize)]
struct OutputUsage {
    reasoning_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
    refusal: Option<String>,
}

impl ModelProvider for OpenAiProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            json_object_output: self.json_mode,
            schema_constrained_output: false,
            tool_calls: false,
        }
    }

    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.complete_envelope(messages).await?.into_text()
    }
    async fn complete_envelope(
        &self,
        messages: &[Message],
    ) -> Result<ModelCompletion, ProviderError> {
        let body = CompletionRequest {
            model: &self.model,
            messages,
            response_format: self
                .json_mode
                .then(|| serde_json::json!({"type": "json_object"})),
        };
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .header(reqwest::header::AUTHORIZATION, self.authorization.clone())
            .json(&body)
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            // Provider error bodies can echo credentials or user content.
            return Err(ProviderError::Http(response.status().as_u16()));
        }
        const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(ProviderError::Response("body exceeds 1 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        decode_completion(&bytes)
    }
}

fn transport_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Transport
    }
}

fn decode_completion(bytes: &[u8]) -> Result<ModelCompletion, ProviderError> {
    let response: CompletionResponse = serde_json::from_slice(bytes)
        .map_err(|_| ProviderError::Response("expected a Chat Completions JSON envelope"))?;
    if [&response.id, &response.model, &response.system_fingerprint]
        .into_iter()
        .flatten()
        .any(|value| value.len() > 512)
    {
        return Err(ProviderError::Response("response metadata exceeds budget"));
    }
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or(ProviderError::Response("no choices returned"))?;
    let status = if choice
        .message
        .refusal
        .is_some_and(|value| !value.is_empty())
    {
        CompletionStatus::Refused
    } else {
        match choice.finish_reason.as_deref() {
            Some("stop") => CompletionStatus::Complete,
            Some("tool_calls" | "function_call") => CompletionStatus::ToolCalls,
            _ => CompletionStatus::Incomplete,
        }
    };
    let usage = response.usage.unwrap_or_default();
    Ok(ModelCompletion {
        text: choice.message.content,
        status,
        metadata: CompletionMetadata {
            provider_request_id: response.id,
            model: response.model,
            system_fingerprint: response.system_fingerprint,
            usage: TokenUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
                cached_input_tokens: usage.prompt_tokens_details.and_then(|u| u.cached_tokens),
                reasoning_tokens: usage
                    .completion_tokens_details
                    .and_then(|u| u.reasoning_tokens),
            },
        },
    })
}
#[cfg(test)]
mod accounting_tests {
    use super::*;
    #[test]
    fn usage_is_preserved_for_refusal_and_incomplete_attempts() {
        for (finish, refusal, expected) in [
            ("stop", None, CompletionStatus::Complete),
            ("length", None, CompletionStatus::Incomplete),
            ("stop", Some("sensitive refusal"), CompletionStatus::Refused),
            ("tool_calls", None, CompletionStatus::ToolCalls),
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({"id":"req-1","model":"pinned-model","system_fingerprint":"rev-1","usage":{"prompt_tokens":100,"completion_tokens":10,"total_tokens":110,"prompt_tokens_details":{"cached_tokens":40},"completion_tokens_details":{"reasoning_tokens":3}},"choices":[{"message":{"content":"private query", "refusal":refusal},"finish_reason":finish}]})).unwrap();
            let completion = decode_completion(&bytes).unwrap();
            assert_eq!(completion.status, expected);
            assert_eq!(completion.metadata.usage.input_tokens, Some(100));
            assert_eq!(completion.metadata.usage.cached_input_tokens, Some(40));
            assert_eq!(completion.metadata.usage.reasoning_tokens, Some(3));
            assert!(!format!("{completion:?}").contains("private query"));
        }
        let missing = decode_completion(
            br#"{"choices":[{"message":{"content":"{}"},"finish_reason":"stop"}]}"#,
        )
        .unwrap();
        assert!(missing.metadata.usage.input_tokens.is_none());
    }
}
