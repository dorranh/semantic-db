use std::{future::Future, time::Duration};

use reqwest::{Client, Url, header::HeaderValue};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// Providers return text; the compiler owns decoding and semantic validation.
pub trait ModelProvider: Send + Sync {
    fn complete(
        &self,
        messages: &[Message],
    ) -> impl Future<Output = Result<String, ProviderError>> + Send;
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
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
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
        let response: CompletionResponse = serde_json::from_slice(&bytes)
            .map_err(|_| ProviderError::Response("expected a Chat Completions JSON envelope"))?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or(ProviderError::Response("no choices returned"))?;
        if choice
            .message
            .refusal
            .is_some_and(|value| !value.is_empty())
        {
            return Err(ProviderError::Refused);
        }
        if choice.finish_reason.as_deref() != Some("stop") {
            return Err(ProviderError::Incomplete);
        }
        choice
            .message
            .content
            .filter(|text| !text.trim().is_empty())
            .ok_or(ProviderError::Response("missing or empty text content"))
    }
}

fn transport_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Transport
    }
}
